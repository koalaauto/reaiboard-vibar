//! 麦克风:把键盘的板载麦克风变成系统麦克风。
//!
//! 流程:装驱动(一次性,需管理员密码) → 用户启用 → 键盘已连接且有 App 在录 →
//! 向键盘申请音频流 → SDK 解码 mSBC → 经本机 UDP 送进驱动 → 系统里的 "ReAI-Vibe-Board" 设备出声。
//! 没人录音时立刻停流:省电,也保证没人用时键盘麦克风是关的。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use reai_board_sdk::sink::PcmSink;
use reai_board_sdk::virtual_mic::{VirtualMic, DEVICE_NAME};
use reai_board_sdk::{AudioStreamAction, AudioStreamScope, AudioTransport, BoardDevice};
use serde::{Deserialize, Serialize};

use crate::coreaudio;

const LEASE_ID: u32 = 0x5642_4152; // "VBAR"
const LEASE_TTL_MS: u16 = 5_000;
/// 没人录之后再等几个 tick(500ms 一次)才停流,避免 App 反复探测麦克风造成抖动
const STOP_GRACE_TICKS: u8 = 3;
/// 每几个 tick 给音频租约续一次期(500ms × 4 = 2s)
const HEARTBEAT_TICKS: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicState {
    /// 用户没有启用
    Off,
    /// 正在安装驱动(等管理员密码)
    Installing,
    /// 已启用,键盘没连上
    WaitingKeyboard,
    /// 已启用且键盘在线,没有 App 在录
    Idle,
    /// 有 App 在录,音频流动中
    InUse,
}

/// 安装驱动的结果
#[derive(Debug)]
pub enum InstallOutcome {
    Installed,
    Cancelled,
    Failed(String),
}

// ---------- 音频电平:给日志和排障用,证明声音真的流进来了 ----------

#[derive(Default)]
struct Stats {
    samples: u64,
    peak: f32,
    sum_sq: f64,
}

/// 累计收到的 PCM 统计,按需取走并清零
#[derive(Default)]
pub struct Meter(Mutex<Stats>);

impl Meter {
    fn add(&self, samples: &[f32]) {
        let mut st = self.0.lock().unwrap();
        for &x in samples {
            st.samples += 1;
            st.peak = st.peak.max(x.abs());
            st.sum_sq += (x as f64) * (x as f64);
        }
    }

    /// 返回 (采样数, 均方根, 峰值) 并清零
    pub fn take(&self) -> (u64, f32, f32) {
        let mut st = self.0.lock().unwrap();
        let rms = if st.samples == 0 {
            0.0
        } else {
            (st.sum_sq / st.samples as f64).sqrt() as f32
        };
        let out = (st.samples, rms, st.peak);
        *st = Stats::default();
        out
    }
}

/// 在把 PCM 送进虚拟麦克风的同时记录电平
pub struct MeteredSink {
    inner: VirtualMic,
    meter: Arc<Meter>,
}

impl MeteredSink {
    pub fn new(inner: VirtualMic, meter: Arc<Meter>) -> Self {
        Self { inner, meter }
    }
}

impl PcmSink for MeteredSink {
    fn on_pcm(&self, samples: &[f32]) {
        self.inner.on_pcm(samples);
        self.meter.add(samples);
    }
}

// ---------- 持久化:用户是否启用了麦克风 ----------

#[derive(Serialize, Deserialize, Default)]
struct Persisted {
    #[serde(default)]
    mic_enabled: bool,
}

fn state_path() -> PathBuf {
    crate::config::config_path().with_file_name("state.toml")
}

pub fn load_enabled() -> bool {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|t| toml::from_str::<Persisted>(&t).ok())
        .map(|p| p.mic_enabled)
        .unwrap_or(false)
}

fn save_enabled(on: bool) {
    let path = state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string(&Persisted { mic_enabled: on }) {
        let _ = std::fs::write(path, text);
    }
}

// ---------- 驱动的安装 / 移除 ----------

pub fn is_installed() -> bool {
    VirtualMic::is_installed()
}

/// 启动时、还没连上键盘之前的麦克风状态,界面用它显示真实情况
pub fn initial_state() -> (MicState, bool) {
    let installed = is_installed();
    let state = if load_enabled() && installed {
        MicState::WaitingKeyboard
    } else {
        MicState::Off
    };
    (state, installed)
}

/// App 包里自带的驱动位置:Vibar.app/Contents/Resources/ReAIVibeBoard.driver
fn bundled_driver() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe
        .parent()?
        .parent()?
        .join("Resources/ReAIVibeBoard.driver");
    p.exists().then_some(p)
}

/// 先弹说明并征得同意,再安装驱动(系统会再弹管理员密码框)。阻塞调用。
pub fn install_with_consent() -> InstallOutcome {
    let dialog = "display dialog \"Vibar 需要安装一个音频驱动,键盘的麦克风才会出现在系统的声音输入列表里。\\n\\n\
· 需要输入管理员密码\\n\
· 安装时系统声音会短暂中断一两秒\\n\
· 随时可以在菜单里移除\" with title \"安装麦克风驱动\" buttons {\"取消\", \"安装\"} default button \"安装\" with icon note";
    match std::process::Command::new("osascript")
        .arg("-e")
        .arg(dialog)
        .output()
    {
        Ok(o) if o.status.success() => {}
        Ok(_) => return InstallOutcome::Cancelled,
        Err(e) => return InstallOutcome::Failed(e.to_string()),
    }
    if let Some(path) = bundled_driver() {
        // SDK 默认用编译期的绝对路径找驱动;指向包内自带的那份才可搬动
        std::env::set_var("REAI_VIRTUAL_MIC_DRIVER_OVERRIDE", path);
    }
    match VirtualMic::ensure_installed() {
        Ok(()) => InstallOutcome::Installed,
        Err(e) => {
            let msg = e.to_string();
            // 用户在密码框里点了取消
            if msg.contains("取消") {
                InstallOutcome::Cancelled
            } else {
                InstallOutcome::Failed(msg)
            }
        }
    }
}

pub fn remove_driver() -> Result<(), String> {
    VirtualMic::uninstall().map_err(|e| e.to_string())
}

// ---------- 状态机 ----------

#[derive(PartialEq, Eq, Clone, Copy)]
enum Lease {
    None,
    /// 固件 v1.59+ 的带租约音频流
    Managed,
    /// 旧固件的纯会话流
    Legacy,
}

pub struct MicCtl {
    enabled: bool,
    installing: bool,
    connected: bool,
    recording: bool,
    lease: Lease,
    device_id: Option<u32>,
    quiet_ticks: u8,
    beat: u8,
    log_tick: u8,
    reported: Option<(MicState, bool)>,
    meter: Arc<Meter>,
}

impl MicCtl {
    pub fn new(meter: Arc<Meter>) -> Self {
        Self {
            meter,
            // 驱动被手动删掉时,不能还假装已启用
            enabled: load_enabled() && is_installed(),
            installing: false,
            connected: false,
            recording: false,
            lease: Lease::None,
            device_id: None,
            quiet_ticks: 0,
            beat: 0,
            log_tick: 0,
            reported: None,
        }
    }

    pub fn state(&self) -> MicState {
        if self.installing {
            MicState::Installing
        } else if !self.enabled {
            MicState::Off
        } else if !self.connected {
            MicState::WaitingKeyboard
        } else if self.lease != Lease::None && self.recording {
            MicState::InUse
        } else {
            MicState::Idle
        }
    }

    /// 状态相对上一次上报有变化时返回新状态,否则 None(用于去重)
    pub fn take_change(&mut self) -> Option<(MicState, bool)> {
        let now = (self.state(), is_installed());
        (self.reported != Some(now)).then(|| {
            self.reported = Some(now);
            now
        })
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        if !connected {
            // 连接已断,租约自然作废,不必再发停止命令
            self.lease = Lease::None;
            self.beat = 0;
        }
    }

    pub fn set_installing(&mut self, on: bool) {
        self.installing = on;
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        save_enabled(on);
    }

    /// 用户关闭:先停流再标记
    pub async fn disable(&mut self, device: &BoardDevice) {
        self.stop(device).await;
        self.set_enabled(false);
    }

    /// 每 500ms 调一次:看有没有人在录 → 开/停流 → 续租
    pub async fn tick(&mut self, device: &BoardDevice) {
        // 设备可能晚于我们出现(刚装完驱动)或中途消失(驱动被移除),每次都重新确认
        if self.device_id.is_none() {
            self.device_id = coreaudio::find_device(DEVICE_NAME);
        }
        self.recording = match self.device_id.map(coreaudio::is_running_somewhere) {
            Some(Some(r)) => r,
            Some(None) => {
                self.device_id = None;
                false
            }
            None => false,
        };

        let want = self.enabled && self.connected && self.recording;
        if want {
            self.quiet_ticks = 0;
            if self.lease == Lease::None {
                self.start(device).await;
            }
        } else if self.lease != Lease::None {
            self.quiet_ticks += 1;
            if !self.enabled || !self.connected || self.quiet_ticks >= STOP_GRACE_TICKS {
                self.stop(device).await;
            }
        }

        if self.lease != Lease::None && self.recording {
            self.log_tick += 1;
            if self.log_tick >= HEARTBEAT_TICKS {
                self.log_tick = 0;
                let (n, rms, peak) = self.meter.take();
                println!("[麦克风] 近 2 秒收到 {n} 个采样,音量 rms={rms:.4} 峰值={peak:.3}");
            }
        } else {
            self.log_tick = 0;
        }

        if self.lease == Lease::Managed {
            self.beat += 1;
            if self.beat >= HEARTBEAT_TICKS {
                self.beat = 0;
                let r = device
                    .control_audio_stream(
                        AudioStreamAction::Heartbeat,
                        AudioTransport::BleGatt,
                        AudioStreamScope::Session,
                        LEASE_ID,
                        LEASE_TTL_MS,
                    )
                    .await;
                if let Err(e) = r {
                    eprintln!("[麦克风] 续租失败,停止音频:{e}");
                    device.stop_local_audio_reader();
                    self.lease = Lease::None;
                }
            }
        }
    }

    async fn start(&mut self, device: &BoardDevice) {
        let managed = device
            .start_board_audio(
                AudioTransport::BleGatt,
                AudioStreamScope::Session,
                LEASE_ID,
                LEASE_TTL_MS,
            )
            .await;
        match managed {
            Ok(_) => {
                println!("[麦克风] 有 App 在录,音频流已开启");
                self.lease = Lease::Managed;
            }
            Err(e) => {
                eprintln!("[麦克风] 带租约的音频流不可用({e}),尝试旧固件方式");
                match device.start_legacy_ble_session_reader() {
                    Ok(()) => self.lease = Lease::Legacy,
                    Err(e) => eprintln!("[麦克风] 开启失败:{e}"),
                }
            }
        }
        self.beat = 0;
    }

    async fn stop(&mut self, device: &BoardDevice) {
        match self.lease {
            Lease::Managed => {
                let _ = device
                    .control_audio_stream(
                        AudioStreamAction::Stop,
                        AudioTransport::BleGatt,
                        AudioStreamScope::Session,
                        LEASE_ID,
                        LEASE_TTL_MS,
                    )
                    .await;
                println!("[麦克风] 没有 App 在录了,音频流已停");
            }
            Lease::Legacy => device.stop_local_audio_reader(),
            Lease::None => {}
        }
        self.lease = Lease::None;
        self.quiet_ticks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(enabled: bool, connected: bool) -> MicCtl {
        MicCtl {
            enabled,
            installing: false,
            connected,
            recording: false,
            lease: Lease::None,
            device_id: None,
            quiet_ticks: 0,
            beat: 0,
            log_tick: 0,
            reported: None,
            meter: Arc::new(Meter::default()),
        }
    }

    #[test]
    fn meter_reports_rms_and_peak_then_resets() {
        let m = Meter::default();
        m.add(&[0.5, -0.5, 0.5, -0.5]);
        let (n, rms, peak) = m.take();
        assert_eq!(n, 4);
        assert!((rms - 0.5).abs() < 1e-6);
        assert!((peak - 0.5).abs() < 1e-6);
        assert_eq!(m.take().0, 0);
    }

    #[test]
    fn initial_state_is_off_without_a_driver() {
        // 没装驱动时,无论设置里记了什么,都不能假装已启用
        let (state, installed) = initial_state();
        if !installed {
            assert_eq!(state, MicState::Off);
        }
    }

    #[test]
    fn state_follows_enabled_and_connection() {
        assert_eq!(ctl(false, true).state(), MicState::Off);
        assert_eq!(ctl(true, false).state(), MicState::WaitingKeyboard);
        assert_eq!(ctl(true, true).state(), MicState::Idle);
    }

    #[test]
    fn installing_takes_precedence() {
        let mut c = ctl(false, true);
        c.set_installing(true);
        assert_eq!(c.state(), MicState::Installing);
    }

    #[test]
    fn in_use_requires_an_active_lease_and_a_recorder() {
        let mut c = ctl(true, true);
        c.recording = true;
        assert_eq!(c.state(), MicState::Idle); // 还没开流
        c.lease = Lease::Managed;
        assert_eq!(c.state(), MicState::InUse);
    }

    #[test]
    fn disconnect_drops_the_lease() {
        let mut c = ctl(true, true);
        c.lease = Lease::Managed;
        c.set_connected(false);
        assert!(c.lease == Lease::None);
        assert_eq!(c.state(), MicState::WaitingKeyboard);
    }

    #[test]
    fn take_change_reports_each_change_once() {
        let mut c = ctl(true, true);
        assert!(c.take_change().is_some());
        assert!(c.take_change().is_none());
        c.set_connected(false);
        assert!(c.take_change().is_some());
    }
}
