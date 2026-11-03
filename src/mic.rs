//! Microphone. Turns the keyboard's onboard microphone into a system microphone.
//!
//! Flow: install the driver (once, needs the administrator password), the user enables it, the
//! keyboard is connected and an app is recording, ask the keyboard for an audio stream, the SDK
//! decodes mSBC, the PCM goes into the driver over local UDP, and the system's "ReAI-Vibe-Board"
//! device then carries the sound. The stream stops at once when nobody is recording, which saves
//! power and keeps the keyboard microphone off while nobody uses it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use reai_board_sdk::sink::PcmSink;
use reai_board_sdk::virtual_mic::{VirtualMic, DEVICE_NAME};
use reai_board_sdk::{AudioStreamAction, AudioStreamScope, AudioTransport, BoardDevice};
use serde::{Deserialize, Serialize};

use crate::coreaudio;
use crate::i18n::Msg;

const LEASE_ID: u32 = 0x5642_4152; // "VBAR"
const LEASE_TTL_MS: u16 = 5_000;
/// Ticks (500 ms each) to wait after the last recorder is gone before stopping the stream. Apps
/// that probe the microphone repeatedly would otherwise make the stream flap.
const STOP_GRACE_TICKS: u8 = 3;
/// Ticks between audio lease renewals (500 ms × 4 = 2 s)
const HEARTBEAT_TICKS: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicState {
    /// Not enabled by the user
    Off,
    /// Installing the driver, waiting for the administrator password
    Installing,
    /// Enabled, the keyboard is not connected
    WaitingKeyboard,
    /// Enabled and the keyboard is online, no app is recording
    Idle,
    /// An app is recording and audio is flowing
    InUse,
}

/// Result of installing the driver
#[derive(Debug)]
pub enum InstallOutcome {
    Installed,
    Cancelled,
    Failed(String),
}

// ---------- Audio level, for logs and troubleshooting, proves that sound really arrives ----------

#[derive(Default)]
struct Stats {
    samples: u64,
    peak: f32,
    sum_sq: f64,
}

/// Running statistics of the PCM received. Taking them also resets them.
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

    /// Returns (sample count, RMS, peak) and resets the counters
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

/// Records the level while passing PCM on to the virtual microphone
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

// ---------- Persistence of whether the user has enabled the microphone ----------

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

// ---------- Driver install and removal ----------

pub fn is_installed() -> bool {
    VirtualMic::is_installed()
}

/// Microphone state at startup before the keyboard connects, so the UI shows the real situation
pub fn initial_state() -> (MicState, bool) {
    let installed = is_installed();
    let state = if load_enabled() && installed {
        MicState::WaitingKeyboard
    } else {
        MicState::Off
    };
    (state, installed)
}

/// Driver shipped inside the app bundle, at Vibar.app/Contents/Resources/ReAIVibeBoard.driver
fn bundled_driver() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe
        .parent()?
        .parent()?
        .join("Resources/ReAIVibeBoard.driver");
    p.exists().then_some(p)
}

/// Escape text for an AppleScript string literal.
fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// The consent dialog shown before the driver is installed.
fn consent_dialog_script() -> String {
    format!(
        "display dialog \"{text}\" with title \"{title}\" buttons {{\"{cancel}\", \"{install}\"}} default button \"{install}\" with icon note",
        text = applescript_escape(&Msg::MicDialogText.text()),
        title = applescript_escape(&Msg::MicDialogTitle.text()),
        cancel = applescript_escape(&Msg::DialogCancel.text()),
        install = applescript_escape(&Msg::DialogInstall.text()),
    )
}

/// Ask for consent first, then install the driver (the system then asks for the administrator
/// password). Blocks.
pub fn install_with_consent() -> InstallOutcome {
    let dialog = consent_dialog_script();
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
        // The SDK looks for the driver at an absolute path baked in at build time. Point it at the
        // copy inside the app bundle so the app can be moved.
        std::env::set_var("REAI_VIRTUAL_MIC_DRIVER_OVERRIDE", path);
    }
    match VirtualMic::ensure_installed() {
        Ok(()) => InstallOutcome::Installed,
        Err(e) => {
            let msg = e.to_string();
            // The user cancelled the password prompt. This matches the SDK's own message text, which
            // is Chinese; the SDK does not expose a typed error for it.
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

// ---------- State machine ----------

#[derive(PartialEq, Eq, Clone, Copy)]
enum Lease {
    None,
    /// Leased audio stream, firmware v1.59 and later
    Managed,
    /// Plain session stream of older firmware
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
            // If the driver was deleted by hand, do not pretend it is still enabled
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

    /// New state if it changed since the last report, otherwise None. This deduplicates reports.
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
            // The connection is gone, so the lease lapses on its own and no stop command is needed
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

    /// User turned the microphone off. Stop the stream first, then mark it disabled.
    pub async fn disable(&mut self, device: &BoardDevice) {
        self.stop(device).await;
        self.set_enabled(false);
    }

    /// Called every 500 ms. See if anyone is recording, start or stop the stream, renew the lease.
    pub async fn tick(&mut self, device: &BoardDevice) {
        // The device may appear after us (driver just installed) or vanish midway (driver removed),
        // so the lookup is repeated whenever the id is unknown.
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
                println!("[mic] last 2 s: {n} samples, rms={rms:.4}, peak={peak:.3}");
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
                    eprintln!("[mic] lease renewal failed, stopping audio: {e}");
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
                println!("[mic] an app is recording, audio stream started");
                self.lease = Lease::Managed;
            }
            Err(e) => {
                eprintln!(
                    "[mic] leased audio stream unavailable ({e}), trying the old firmware path"
                );
                match device.start_legacy_ble_session_reader() {
                    Ok(()) => self.lease = Lease::Legacy,
                    Err(e) => eprintln!("[mic] could not start audio: {e}"),
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
                println!("[mic] no app is recording any more, audio stream stopped");
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
    fn applescript_text_is_escaped() {
        assert_eq!(applescript_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn consent_dialog_has_both_buttons_in_the_current_language() {
        let script = consent_dialog_script();
        assert!(script.starts_with("display dialog \""));
        assert!(script.contains(&format!("\"{}\"", Msg::DialogInstall.text())));
        assert!(script.contains(&format!("\"{}\"", Msg::DialogCancel.text())));
        assert!(
            !script.contains('\n'),
            "newlines must be escaped for AppleScript"
        );
    }

    #[test]
    fn initial_state_is_off_without_a_driver() {
        // Without the driver, never pretend to be enabled, whatever the saved settings say
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
        assert_eq!(c.state(), MicState::Idle); // the stream has not started yet
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
