//! 后台逻辑:蓝牙连接 + 事件分发 + 设备设置。不依赖 UI,通过回调上报状态。

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reai_board_sdk::kernel::types::SleepTimeout;
use reai_board_sdk::virtual_mic::VirtualMic;
use reai_board_sdk::{BoardConfig, BoardDevice, BoardEvent};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::actions::{Action, Resolved};
use crate::config;
use crate::i18n::Msg;
use crate::mic::{self, InstallOutcome, MicCtl, MicState};

/// 拨杆某一档解析好的绑定
#[derive(Default, Clone)]
pub struct ModeMap {
    pub bindings: HashMap<String, Resolved>,
    pub on_enter: Option<Resolved>,
    pub on_leave: Option<Resolved>,
}

/// 解析好的全部绑定:默认层加上拨杆三档各自的覆盖层
#[derive(Default, Clone)]
pub struct Keymap {
    pub default: HashMap<String, Resolved>,
    pub modes: HashMap<String, ModeMap>,
    pub knob_swap: bool,
}

impl Keymap {
    /// 先看当前档位有没有覆盖,没有就用默认层
    pub fn lookup(&self, mode: Option<&str>, name: &str) -> Option<&Resolved> {
        mode.and_then(|m| self.modes.get(m))
            .and_then(|m| m.bindings.get(name))
            .or_else(|| self.default.get(name))
    }

    pub fn count(&self) -> usize {
        self.default.len() + self.modes.values().map(|m| m.bindings.len()).sum::<usize>()
    }
}

pub type Bindings = Arc<RwLock<Keymap>>;

#[derive(Debug, Clone)]
pub enum Status {
    /// 在扫描,键盘可能在休眠
    Searching,
    Connecting,
    Connected,
    Info {
        firmware: String,
        battery: u8,
    },
    /// 键盘当前生效的休眠设置(秒)
    Sleep {
        connected: u16,
        disconnected: u16,
    },
    Disconnected,
    /// 麦克风功能的状态;`installed` 表示系统里是否装有驱动
    Mic {
        state: MicState,
        installed: bool,
    },
    /// AI 语音键按下 / 松开(给界面做反馈)
    Voice(bool),
    /// 某个输入被按下或松开,设置窗口用它实时高亮
    Input {
        name: &'static str,
        pressed: bool,
    },
    /// 拨杆当前所在的档位 chat yolo plan
    Mode(String),
    /// 键盘是否已被主动释放(true = 已断开且不再自动重连,别的电脑可以连)
    Released(bool),
    Error(String),
}

/// 界面发给后台的指令
#[derive(Debug, Clone)]
pub enum Command {
    /// 修改休眠超时(秒);None 表示保持不变。会写入键盘并持久化。
    SetSleep {
        connected: Option<u16>,
        disconnected: Option<u16>,
    },
    /// 启用 / 关闭麦克风。首次启用会先征得同意并安装驱动。
    SetMic(bool),
    /// 移除麦克风驱动(需要管理员密码)
    RemoveMicDriver,
    /// 释放键盘 / 重新连接。释放后不再自动重连,让别的电脑能连上
    SetReleased(bool),
}

/// 读配置并解析成绑定表。返回 配置、生效的绑定、无效项说明
pub fn load_bindings() -> anyhow::Result<(config::Config, Keymap, Vec<String>)> {
    let cfg = config::load()?;
    Ok(resolve_all(cfg))
}

fn resolve_one(label: &str, b: &config::Binding, bad: &mut Vec<String>) -> Option<Resolved> {
    match crate::actions::resolve(b) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("[config] binding `{label}` is invalid: {e}");
            bad.push(
                Msg::InvalidEntry {
                    label: label.to_string(),
                    reason: e.to_string(),
                }
                .text(),
            );
            None
        }
    }
}

/// Resolve every config entry into actions. Invalid entries are not dropped silently: each one
/// becomes a message that the UI shows to the user.
pub fn resolve_all(cfg: config::Config) -> (config::Config, Keymap, Vec<String>) {
    let mut bad = Vec::new();
    let mut km = Keymap {
        knob_swap: cfg.knob_swap,
        ..Default::default()
    };
    for (name, b) in &cfg.bindings {
        if !config::INPUTS.contains(&name.as_str()) {
            bad.push(Msg::NotAnInput(name.clone()).text());
            continue;
        }
        if let Some(r) = resolve_one(name, b, &mut bad) {
            km.default.insert(name.clone(), r);
        }
    }
    for (mode, mc) in &cfg.modes {
        if !config::MODES.contains(&mode.as_str()) {
            bad.push(Msg::NotAMode(mode.clone()).text());
            continue;
        }
        let mut mm = ModeMap::default();
        for (name, b) in &mc.bindings {
            if !config::INPUTS.contains(&name.as_str()) {
                let label = Msg::LabelInMode {
                    mode: mode.clone(),
                    name: name.clone(),
                }
                .text();
                bad.push(Msg::NotAnInput(label).text());
                continue;
            }
            let label = Msg::LabelInMode {
                mode: mode.clone(),
                name: name.clone(),
            }
            .text();
            if let Some(r) = resolve_one(&label, b, &mut bad) {
                mm.bindings.insert(name.clone(), r);
            }
        }
        mm.on_enter = mc
            .on_enter
            .as_ref()
            .and_then(|b| resolve_one(&Msg::LabelOnEnter(mode.clone()).text(), b, &mut bad));
        mm.on_leave = mc
            .on_leave
            .as_ref()
            .and_then(|b| resolve_one(&Msg::LabelOnLeave(mode.clone()).text(), b, &mut bad));
        km.modes.insert(mode.clone(), mm);
    }
    (cfg, km, bad)
}

pub async fn run(
    device_name: Option<String>,
    bindings: Bindings,
    exec: Sender<Action>,
    status: Arc<dyn Fn(Status) + Send + Sync>,
    mut commands: UnboundedReceiver<Command>,
) -> anyhow::Result<()> {
    // 按下时选中的绑定要记住,松开时用同一个,即使中途拨杆换了档也不会卡住按键
    let mut held: HashMap<String, Resolved> = HashMap::new();

    let device = Arc::new(BoardDevice::open(BoardConfig::default())?);
    // 虚拟麦克风的发送端必须在 start() 前挂上;驱动没装时发送会静默失败,不影响其他功能
    let meter = Arc::new(mic::Meter::default());
    match VirtualMic::start() {
        Ok(sink) => device.set_pcm_sink(Arc::new(mic::MeteredSink::new(sink, meter.clone()))),
        Err(e) => eprintln!("[mic] could not create the sender: {e}"),
    }
    device.start().await?;

    // 找板子:配置里指定了就直接连;否则一直扫到出现为止(键盘休眠时不广播,按键唤醒后才出现)
    let target = match device_name {
        Some(n) => n,
        None => loop {
            status(Status::Searching);
            println!("Scanning for REAI_VB_ devices... (press a key if the keyboard is asleep)");
            let found = device.scan_ble_devices(Duration::from_secs(6)).await?;
            if let Some(d) = found.first() {
                break d.name.clone();
            }
        },
    };
    println!("Connecting to {target} ...");
    status(Status::Connecting);
    device.connect_ble(&target);

    let mut events = device.events();
    let mut last_mode: Option<String> = None;
    let mut connected = false;
    let mut released = false;

    let mut mic = MicCtl::new(meter);
    let (install_tx, mut install_rx) = unbounded_channel::<InstallOutcome>();
    let mut mic_tick = tokio::time::interval(Duration::from_millis(500));
    mic_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    emit_mic(&mut mic, &*status);

    loop {
        tokio::select! {
            _ = mic_tick.tick() => {
                mic.tick(&device).await;
                emit_mic(&mut mic, &*status);
            }
            outcome = install_rx.recv() => {
                let Some(outcome) = outcome else { continue };
                mic.set_installing(false);
                println!("[mic] install result: {outcome:?}");
                match outcome {
                    InstallOutcome::Installed => mic.set_enabled(true),
                    InstallOutcome::Cancelled => {}
                    InstallOutcome::Failed(e) => status(Status::Error(Msg::MicInstallFailed(e).text())),
                }
                emit_mic(&mut mic, &*status);
            }
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { continue };
                // 麦克风相关指令不需要键盘在线
                match &cmd {
                    Command::SetMic(true) => {
                        println!("[mic] enable requested, driver installed: {}", mic::is_installed());
                        if mic::is_installed() {
                            mic.set_enabled(true);
                        } else if !mic_is_busy(&mic) {
                            mic.set_installing(true);
                            let tx = install_tx.clone();
                            // 安装要等用户输入管理员密码,放到阻塞线程里,不卡住按键处理
                            tokio::task::spawn_blocking(move || {
                                let _ = tx.send(mic::install_with_consent());
                            });
                        }
                        emit_mic(&mut mic, &*status);
                        continue;
                    }
                    Command::SetMic(false) => {
                        println!("[mic] disable requested");
                        mic.disable(&device).await;
                        emit_mic(&mut mic, &*status);
                        continue;
                    }
                    Command::RemoveMicDriver => {
                        mic.disable(&device).await;
                        let status = status.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = mic::remove_driver() {
                                status(Status::Error(Msg::MicRemoveFailed(e).text()));
                            }
                        });
                        continue;
                    }
                    Command::SetReleased(true) => {
                        println!("[share] releasing the keyboard");
                        released = true;
                        device.set_auto_reconnect(false);
                        let _ = device.disconnect_ble().await;
                        connected = false;
                        mic.set_connected(false);
                        status(Status::Released(true));
                        emit_mic(&mut mic, &*status);
                        continue;
                    }
                    Command::SetReleased(false) => {
                        println!("[share] reconnecting to the keyboard");
                        released = false;
                        status(Status::Released(false));
                        status(Status::Connecting);
                        device.connect_ble(&target);
                        continue;
                    }
                    Command::SetSleep { .. } => {}
                }
                if !connected {
                    status(Status::Error(Msg::KeyboardNotConnected.text()));
                    continue;
                }
                let (device, status) = (device.clone(), status.clone());
                tokio::spawn(async move { handle_command(&device, cmd, &*status).await });
            }
            evt = events.recv() => match evt {
                Ok(Some(evt)) => match &evt {
                    BoardEvent::Connection(c) => {
                        println!("[connection] connected={} {:?}", c.connected, c.connection_type);
                        last_mode = None;
                        connected = c.connected;
                        mic.set_connected(c.connected);
                        emit_mic(&mut mic, &*status);
                        if released {
                            // 主动释放后的断开事件:保持"已释放"显示,不要被覆盖成"已断开"
                            status(Status::Released(true));
                        } else if c.connected {
                            status(Status::Connected);
                            // 连接建立后读一次当前休眠设置(只读),等 SDK 自己的握手命令先跑完
                            let (device, status) = (device.clone(), status.clone());
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(1500)).await;
                                match device.get_sleep_timeout().await {
                                    Ok(t) => {
                                        println!("[sleep] connected {}s, disconnected {}s", t.connected, t.disconnected);
                                        status(Status::Sleep { connected: t.connected, disconnected: t.disconnected });
                                    }
                                    Err(e) => eprintln!("[sleep] read failed: {e}"),
                                }
                            });
                        } else {
                            status(Status::Disconnected);
                        }
                    }
                    BoardEvent::Reconnect(r) => {
                        use reai_board_sdk::kernel::event::ReconnectState as S;
                        if !released && matches!(r.state, S::Scanning | S::WaitingForDevice) {
                            status(Status::Searching);
                        }
                    }
                    BoardEvent::DeviceInfo(d) => {
                        println!("[device] fw={} battery={}%", d.firmware_version, d.battery_level);
                        status(Status::Info {
                            firmware: d.firmware_version.to_string(),
                            battery: d.battery_level,
                        });
                    }
                    BoardEvent::KeyPress(k) => {
                        let swap = bindings.read().unwrap().knob_swap;
                        if let Some(name) = config::key_name(k.key_index, swap) {
                            println!("[key] {name} pressed={}", k.pressed);
                            if name == "voice" {
                                status(Status::Voice(k.pressed));
                            }
                            status(Status::Input { name, pressed: k.pressed });
                            let action = if k.pressed {
                                let km = bindings.read().unwrap();
                                let found = km.lookup(last_mode.as_deref(), name).cloned();
                                if let Some(r) = &found {
                                    held.insert(name.to_string(), r.clone());
                                }
                                found.and_then(|r| r.on_press)
                            } else {
                                held.remove(name).and_then(|r| r.on_release)
                            };
                            if let Some(a) = action {
                                let _ = exec.send(a);
                            }
                        }
                    }
                    BoardEvent::ModeChange(m) => {
                        let mode = m.mode.to_string().to_lowercase();
                        // 连接后的第一条是初始状态,不触发;之后只在真正变化时触发
                        if last_mode.as_deref().is_some_and(|prev| prev != mode) {
                            println!("[lever] {mode}");
                            let km = bindings.read().unwrap();
                            let leave = last_mode.as_deref().and_then(|p| km.modes.get(p)).and_then(|m| m.on_leave.as_ref());
                            let enter = km.modes.get(&mode).and_then(|m| m.on_enter.as_ref());
                            for r in leave.into_iter().chain(enter) {
                                if let Some(a) = r.on_press.clone() {
                                    let _ = exec.send(a);
                                }
                            }
                        }
                        status(Status::Mode(mode.clone()));
                        last_mode = Some(mode);
                    }
                    BoardEvent::Error(e) => eprintln!("[error] {}", e.message),
                    _ => {}
                },
                Ok(None) => break,
                Err(e) => eprintln!("[event error] {e:?}"),
            }
        }
    }
    device.shutdown();
    status(Status::Disconnected);
    Ok(())
}

/// 状态有变化才上报,避免每 500ms 刷一次界面
fn emit_mic(mic: &mut MicCtl, status: &(dyn Fn(Status) + Send + Sync)) {
    if let Some((state, installed)) = mic.take_change() {
        status(Status::Mic { state, installed });
    }
}

fn mic_is_busy(mic: &MicCtl) -> bool {
    mic.state() == MicState::Installing
}

async fn handle_command(
    device: &BoardDevice,
    cmd: Command,
    status: &(dyn Fn(Status) + Send + Sync),
) {
    match cmd {
        Command::SetSleep {
            connected,
            disconnected,
        } => {
            // 先读当前值,只改被指定的那一项;值没变就不写(设置会持久化到设备)
            let cur = match device.get_sleep_timeout().await {
                Ok(t) => t,
                Err(e) => return status(Status::Error(Msg::SleepReadFailed(e.to_string()).text())),
            };
            let want = SleepTimeout::new(
                disconnected.unwrap_or(cur.disconnected),
                connected.unwrap_or(cur.connected),
            );
            if want == cur {
                status(Status::Sleep {
                    connected: cur.connected,
                    disconnected: cur.disconnected,
                });
                return;
            }
            match device.set_sleep_timeout(want).await {
                Ok(eff) => {
                    println!(
                        "[sleep] set: connected {}s, disconnected {}s",
                        eff.connected, eff.disconnected
                    );
                    status(Status::Sleep {
                        connected: eff.connected,
                        disconnected: eff.disconnected,
                    });
                }
                Err(e) => status(Status::Error(Msg::SleepSetFailed(e.to_string()).text())),
            }
        }
        // 麦克风指令已在主循环里处理
        Command::SetMic(_) | Command::RemoveMicDriver | Command::SetReleased(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn km(text: &str) -> (Keymap, Vec<String>) {
        let (_, km, bad) = resolve_all(config::parse(text).unwrap());
        (km, bad)
    }

    #[test]
    fn invalid_bindings_are_reported_not_silently_dropped() {
        let (km, bad) =
            km("[bindings]\ntab = \"tab\"\nesc = \"nonsense\"\nnew = { media = \"rewind\" }\n");
        assert_eq!(km.default.len(), 1);
        assert!(km.default.contains_key("tab"));
        assert_eq!(bad.len(), 2);
        assert!(bad.iter().any(|b| b.starts_with("esc ")));
        assert!(bad.iter().any(|b| b.starts_with("new ")));
    }

    #[test]
    fn builtin_default_config_has_no_invalid_bindings() {
        let (km, bad) = km(config::DEFAULT_CONFIG);
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(km.default.len(), 9);
    }

    const LAYERED: &str = "[bindings]\nenter = \"return\"\ntab = \"tab\"\n\
        [modes.plan.bindings]\nenter = \"cmd+return\"\n\
        [modes.plan]\non_enter = \"shift+tab\"\n";

    #[test]
    fn mode_layer_overrides_only_the_keys_it_defines() {
        let (km, bad) = km(LAYERED);
        assert!(bad.is_empty(), "{bad:?}");
        let dbg = |m: Option<&str>, n: &str| {
            format!("{:?}", km.lookup(m, n).and_then(|r| r.on_press.clone()))
        };
        assert!(dbg(Some("plan"), "enter").contains("Return"));
        assert!(
            dbg(Some("plan"), "enter").contains("Meta"),
            "enter in PLAN should carry cmd"
        );
        assert!(
            !dbg(Some("chat"), "enter").contains("Meta"),
            "CHAT keeps the default"
        );
        assert!(
            !dbg(None, "enter").contains("Meta"),
            "without lever info the default applies"
        );
        assert!(
            dbg(Some("plan"), "tab").contains("Tab"),
            "PLAN does not override tab, the default applies"
        );
    }

    #[test]
    fn unknown_input_and_mode_names_are_reported() {
        let (km, bad) = km(
            "[bindings]\ntab = \"tab\"\ntabb = \"tab\"\n[modes.turbo.bindings]\nesc = \"esc\"\n[modes.plan.bindings]\nescc = \"esc\"\n",
        );
        assert_eq!(km.default.len(), 1);
        assert!(bad.iter().any(|b| b.starts_with("tabb ")), "{bad:?}");
        assert!(bad.iter().any(|b| b.starts_with("turbo ")), "{bad:?}");
        assert!(
            bad.iter().any(|b| b.contains("escc") && b.contains("PLAN")),
            "{bad:?}"
        );
    }

    #[test]
    fn mode_enter_action_is_resolved() {
        let (km, _) = km(LAYERED);
        assert!(km.modes["plan"].on_enter.is_some());
        assert!(km.modes["plan"].on_leave.is_none());
    }

    #[test]
    fn invalid_layer_entries_name_the_mode() {
        let (_, bad) = km("[bindings]\ntab = \"tab\"\n[modes.yolo.bindings]\nesc = \"nonsense\"\n");
        assert!(bad[0].contains("esc") && bad[0].contains("YOLO"), "{bad:?}");
    }

    #[test]
    fn missing_key_in_every_layer_yields_none() {
        let (km, _) = km(LAYERED);
        assert!(km.lookup(Some("plan"), "voice").is_none());
    }
}
