//! Background logic for the Bluetooth connection, event dispatch and device settings.
//! Independent of the UI, it reports status through a callback.

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

/// The resolved bindings for one lever position
#[derive(Default, Clone)]
pub struct ModeMap {
    pub bindings: HashMap<String, Resolved>,
    pub on_enter: Option<Resolved>,
    pub on_leave: Option<Resolved>,
}

/// All resolved bindings. The default layer plus an override layer for each lever position.
#[derive(Default, Clone)]
pub struct Keymap {
    pub default: HashMap<String, Resolved>,
    pub modes: HashMap<String, ModeMap>,
    pub knob_swap: bool,
}

impl Keymap {
    /// Looks in the current position's overrides first, then falls back to the default layer
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
    /// Scanning, the keyboard may be asleep
    Searching,
    Connecting,
    Connected,
    Info {
        firmware: String,
        battery: u8,
    },
    /// The sleep setting currently in effect on the keyboard, in seconds
    Sleep {
        connected: u16,
        disconnected: u16,
    },
    Disconnected,
    /// Microphone feature state. `installed` tells whether the driver is on the system
    Mic {
        state: MicState,
        installed: bool,
    },
    /// AI voice key pressed or released, for UI feedback
    Voice(bool),
    /// An input was pressed or released, used by the settings window for live highlighting
    Input {
        name: &'static str,
        pressed: bool,
    },
    /// Current lever position, one of chat, yolo or plan
    Mode(String),
    /// Whether the keyboard was released on purpose. True means disconnected with no automatic
    /// reconnect, so another computer can connect.
    Released(bool),
    Error(String),
}

/// Commands the UI sends to the background task
#[derive(Debug, Clone)]
pub enum Command {
    /// Changes the sleep timeout in seconds, None keeps a value unchanged. The change is written to
    /// the keyboard and persists there.
    SetSleep {
        connected: Option<u16>,
        disconnected: Option<u16>,
    },
    /// Turns the microphone on or off. First enable asks for consent, then installs the driver.
    SetMic(bool),
    /// Removes the microphone driver, needs the administrator password
    RemoveMicDriver,
    /// Releases the keyboard or reconnects to it. A released keyboard is not reconnected
    /// automatically, so that another computer can connect.
    SetReleased(bool),
}

/// Reads the config and resolves it into the binding table. Returns the config, the effective
/// bindings and a message for each invalid entry.
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
    // The binding chosen at press time is remembered and the same one is used on release, so a
    // lever move in between cannot leave a key stuck.
    let mut held: HashMap<String, Resolved> = HashMap::new();

    let device = Arc::new(BoardDevice::open(BoardConfig::default())?);
    // The virtual microphone sink must be attached before start(). If the driver is not installed,
    // sending fails silently and does not affect anything else.
    let meter = Arc::new(mic::Meter::default());
    match VirtualMic::start() {
        Ok(sink) => device.set_pcm_sink(Arc::new(mic::MeteredSink::new(sink, meter.clone()))),
        Err(e) => eprintln!("[mic] could not create the sender: {e}"),
    }
    device.start().await?;

    // Find the board. Connect straight to the one named in the config, otherwise scan until one
    // appears. A sleeping keyboard does not advertise until a key press wakes it.
    let target = match device_name {
        Some(n) => n,
        None => {
            let mut until_next_note = 0u32;
            loop {
                status(Status::Searching);
                // A sleeping keyboard means thousands of scans. Say so at the start and every ten
                // minutes, otherwise the log fills up with this one line.
                if until_next_note == 0 {
                    println!(
                        "Scanning for REAI_VB_ devices... (press a key if the keyboard is asleep)"
                    );
                    until_next_note = 100;
                }
                until_next_note -= 1;
                let found = device.scan_ble_devices(Duration::from_secs(6)).await?;
                if let Some(d) = found.first() {
                    break d.name.clone();
                }
            }
        }
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
                // Microphone commands do not need the keyboard to be online
                match &cmd {
                    Command::SetMic(true) => {
                        println!("[mic] enable requested, driver installed: {}", mic::is_installed());
                        if mic::is_installed() {
                            mic.set_enabled(true);
                        } else if !mic_is_busy(&mic) {
                            mic.set_installing(true);
                            let tx = install_tx.clone();
                            // Installing waits for the administrator password, so
                            // it runs on a blocking thread and key handling is not stalled
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
                            // The disconnect after a deliberate release. Keep showing
                            // "released" instead of overwriting it with "disconnected"
                            status(Status::Released(true));
                        } else if c.connected {
                            status(Status::Connected);
                            // Once connected, read the current sleep setting (read only),
                            // after the SDK has finished its own handshake commands
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
                        // The first event after connecting is the initial state and does not
                        // trigger anything. After that, only a real change triggers.
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

/// Reports only when the state changed, to avoid refreshing the UI every 500 ms
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
            // Read the current values first and change only the requested one.
            // Skip the write if the value is unchanged, since settings persist on the device.
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
        // Microphone commands are already handled in the main loop
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
