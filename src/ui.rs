//! Menu bar shell: the tao event loop (main thread only), tray-icon and the settings window.
//!
//! The menu holds as little as possible: status, disconnect or connect the keyboard, settings, quit. Everything else is in the settings window.
//! Rules:
//! - Nothing normal is shown: no permission row while the permission is fine.
//! - No parentheses or colons to explain a text in the interface.
//! - Text and icons of a menu item change only when the content really changes, because macOS closes an open menu that is modified.

use std::io::Cursor;

use serde_json::{json, Value};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS, EventLoopWindowTargetExtMacOS};
use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIconBuilder};

use crate::daemon::Status;
use crate::i18n::Msg;
use crate::settings::{self, SettingsWindow, Snapshot};

pub const APP_NAME: &str = "Vibar";

/// After this long without finding the keyboard, say what else could be wrong.
const SEARCH_HINT_AFTER: std::time::Duration = std::time::Duration::from_secs(15);

/// Whether a search that started `elapsed` ago deserves the longer hint.
fn search_hint_due(elapsed: std::time::Duration) -> bool {
    elapsed >= SEARCH_HINT_AFTER
}

/// Messages from the background threads to the UI.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Status(Status),
    Trusted(bool),
    /// The config file has a problem the user must fix (`Some`), or is fine again (`None`).
    /// A problem is shown in the detail line until a good reload clears it.
    ConfigProblem(Option<String>),
    Menu(MenuId),
    /// A message from the web page in the settings window.
    Settings(String),
    /// The interface language changed (settings window or config file).
    LanguageChanged,
}

pub type Proxy = EventLoopProxy<UiEvent>;

pub struct Handlers {
    /// Change the keyboard sleep timeouts in seconds, while connected and after disconnecting. `None` leaves one unchanged.
    pub set_sleep: Box<dyn Fn(Option<u16>, Option<u16>)>,
    /// Switch the microphone on or off.
    pub set_mic: Box<dyn Fn(bool)>,
    /// Remove the microphone driver.
    pub remove_mic: Box<dyn Fn()>,
    /// Disconnect the keyboard (`true`) or connect it again (`false`), to lend it to another computer.
    pub set_released: Box<dyn Fn(bool)>,
}

/// Which icon to show. The voice icon with the yellow dot has exactly the size of the normal one, so switching never moves anything.
#[derive(PartialEq, Clone, Copy)]
enum IconKind {
    Off,
    On,
    Voice,
}

struct Icons {
    on: Icon,
    off: Icon,
    voice_dark: Icon,
    voice_light: Icon,
    shown: Option<IconKind>,
    base_on: bool,
    voice: bool,
}

impl Icons {
    /// Swap the icon only when the one to show has really changed.
    fn refresh(&mut self, tray: &tray_icon::TrayIcon) {
        let kind = if self.voice {
            IconKind::Voice
        } else if self.base_on {
            IconKind::On
        } else {
            IconKind::Off
        };
        if self.shown == Some(kind) {
            return;
        }
        self.shown = Some(kind);
        let _ = match kind {
            IconKind::On => tray.set_icon_templated(Some(self.on.clone())),
            IconKind::Off => tray.set_icon_templated(Some(self.off.clone())),
            // The yellow dot is colored, so a template icon cannot be used. Pick a white or black body to match the menu bar right now.
            IconKind::Voice => tray.set_icon(Some(if menubar_is_dark(tray) {
                self.voice_dark.clone()
            } else {
                self.voice_light.clone()
            })),
        };
    }

    fn set_base(&mut self, on: bool, tray: &tray_icon::TrayIcon) {
        self.base_on = on;
        self.refresh(tray);
    }

    fn set_voice(&mut self, down: bool, tray: &tray_icon::TrayIcon) {
        self.voice = down;
        self.refresh(tray);
    }
}

/// Whether the menu bar button is dark right now, in which case the icon must be white.
/// Ask the button itself: the menu bar color follows the wallpaper and need not match the system light or dark setting.
fn menubar_is_dark(tray: &tray_icon::TrayIcon) -> bool {
    use objc2_app_kit::{
        NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    };
    use objc2_foundation::NSArray;
    let (Some(item), Some(mtm)) = (tray.ns_status_item(), objc2::MainThreadMarker::new()) else {
        return true;
    };
    let Some(button) = item.button(mtm) else {
        return true;
    };
    // SAFETY: reads two appearance name constants provided by AppKit.
    unsafe {
        let appearance = button.effectiveAppearance();
        let names = NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]);
        appearance
            .bestMatchFromAppearancesWithNames(&names)
            .is_some_and(|n| &*n == NSAppearanceNameDarkAqua)
    }
}

/// Bring this app to the front. A menu bar app is not the frontmost app by default.
#[allow(deprecated)]
fn activate_app() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// The standard macOS About panel. Icon, name and version come from Info.plist.
#[allow(deprecated)]
fn show_about() {
    activate_app();
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
    }
}

/// Play a system sound without waiting for it to finish.
fn play_sound(name: &str) {
    let path = format!("/System/Library/Sounds/{name}.aiff");
    if let Ok(mut child) = std::process::Command::new("afplay").arg(path).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Leave the item alone when the text did not change.
fn put(item: &MenuItem, text: impl AsRef<str>) {
    if item.text() != text.as_ref() {
        item.set_text(text.as_ref());
    }
}

/// What a menu line shows. Kept as a message, not as text, so that a language change can redraw it.
#[derive(Clone)]
enum Line {
    Msg(Msg),
    Raw(String),
}

impl Line {
    fn text(&self) -> String {
        match self {
            Line::Msg(m) => m.text(),
            Line::Raw(s) => s.clone(),
        }
    }
}

/// Only the user can switch Accessibility on, so all we do is open the right pane.
fn open_accessibility_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

/// Mark first-run setup finished the moment both steps are seen done.
fn finish_setup(snap: &mut Snapshot, state: &mut crate::setup::Setup) {
    if !state.completed && crate::setup::is_complete(snap.connected, snap.trusted) {
        state.completed = true;
        snap.setup_done = true;
        crate::setup::save(state);
    }
}

/// What the detail line says. An error from the keyboard session wins, because it is the newest
/// and needs a look. Then a config problem, which stays until the config is fixed. Otherwise the
/// normal line, such as the battery.
fn detail_text(normal: &Line, config_problem: &Option<String>, error: &Option<String>) -> String {
    error
        .clone()
        .or_else(|| config_problem.clone())
        .unwrap_or_else(|| normal.text())
}

/// The tray menu items and what they currently show.
struct MenuUi {
    status_item: MenuItem,
    detail_item: MenuItem,
    trust_item: MenuItem,
    release_item: MenuItem,
    settings_item: MenuItem,
    about_item: MenuItem,
    quit_item: MenuItem,
    status_line: Line,
    /// What the detail line normally says: battery, search hint.
    detail_line: Line,
    /// A problem in the config file, shown instead of `detail_line` until a good reload clears it.
    config_problem: Option<String>,
    /// The latest error, shown first. The next status update replaces it.
    error: Option<String>,
}

impl MenuUi {
    fn new() -> Self {
        let q = Some(Accelerator::new(Modifiers::META, Code::KeyQ));
        Self {
            status_item: MenuItem::new(Msg::Starting.text(), false, None),
            detail_item: MenuItem::new("", false, None),
            trust_item: MenuItem::new(Msg::AccessibilityNeeded.text(), true, None),
            release_item: MenuItem::new(Msg::DisconnectKeyboard.text(), false, None),
            settings_item: MenuItem::new(
                Msg::SettingsMenu.text(),
                true,
                Some(Accelerator::new(Modifiers::META, Code::Comma)),
            ),
            about_item: MenuItem::new(Msg::About.text(), true, None),
            quit_item: MenuItem::new(Msg::Quit.text(), true, q),
            status_line: Line::Msg(Msg::Starting),
            detail_line: Line::Raw(String::new()),
            config_problem: None,
            error: None,
        }
    }

    fn set_status(&mut self, m: Msg) {
        self.status_line = Line::Msg(m);
        put(&self.status_item, self.status_line.text());
    }

    /// A new normal line. Whatever error was showing belongs to the past.
    fn set_detail(&mut self, l: Line) {
        self.detail_line = l;
        self.error = None;
        self.show_detail();
    }

    fn set_config_problem(&mut self, problem: Option<String>) {
        self.config_problem = problem;
        self.show_detail();
    }

    fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.show_detail();
    }

    fn show_detail(&self) {
        put(
            &self.detail_item,
            detail_text(&self.detail_line, &self.config_problem, &self.error),
        );
    }

    /// Redraw every line in the current language.
    fn relocalize(&self, released: bool) {
        put(&self.status_item, self.status_line.text());
        self.show_detail();
        put(&self.trust_item, Msg::AccessibilityNeeded.text());
        put(
            &self.release_item,
            if released {
                Msg::ConnectKeyboard
            } else {
                Msg::DisconnectKeyboard
            }
            .text(),
        );
        put(&self.settings_item, Msg::SettingsMenu.text());
        put(&self.about_item, Msg::About.text());
        put(&self.quit_item, Msg::Quit.text());
    }
}

/// While the settings window is open the app becomes a regular app, and it needs a standard main menu for copy, paste, close window and the like to work.
/// Handles to the main menu items whose titles we own, so they can follow the language.
struct MainMenu {
    app: Submenu,
    about: PredefinedMenuItem,
    quit: PredefinedMenuItem,
    edit: Submenu,
    window: Submenu,
}

impl MainMenu {
    fn relocalize(&self) {
        self.about.set_text(Msg::About.text());
        self.quit.set_text(Msg::Quit.text());
        self.edit.set_text(Msg::EditMenu.text());
        self.window.set_text(Msg::WindowMenu.text());
        let _ = &self.app; // the app menu title is the app name and never changes
    }
}

fn install_main_menu() -> Option<MainMenu> {
    let about = PredefinedMenuItem::about(Some(&Msg::About.text()), None);
    let quit = PredefinedMenuItem::quit(Some(&Msg::Quit.text()));
    let build = || -> Result<(Menu, MainMenu), tray_icon::menu::Error> {
        let app = Submenu::with_items(
            APP_NAME,
            true,
            &[
                &about,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::separator(),
                &quit,
            ],
        )?;
        let edit = Submenu::with_items(
            Msg::EditMenu.text(),
            true,
            &[
                &PredefinedMenuItem::undo(None),
                &PredefinedMenuItem::redo(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::cut(None),
                &PredefinedMenuItem::copy(None),
                &PredefinedMenuItem::paste(None),
                &PredefinedMenuItem::select_all(None),
            ],
        )?;
        let window = Submenu::with_items(
            Msg::WindowMenu.text(),
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::close_window(None),
            ],
        )?;
        let menu = Menu::with_items(&[&app, &edit, &window])?;
        Ok((
            menu,
            MainMenu {
                app,
                about: about.clone(),
                quit: quit.clone(),
                edit,
                window,
            },
        ))
    };
    let (menu, handles) = build().ok()?;
    menu.init_for_nsapp();
    std::mem::forget(menu); // the main menu must live as long as the app
    Some(handles)
}

/// Decode a built-in PNG icon, generated by scripts/make-icons.py.
fn load_icon(png_bytes: &[u8]) -> Icon {
    let mut reader = png::Decoder::new(Cursor::new(png_bytes))
        .read_info()
        .expect("invalid icon PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("icon size")];
    let info = reader.next_frame(&mut buf).expect("icon decode failed");
    buf.truncate(info.buffer_size());
    Icon::from_rgba(buf, info.width, info.height).expect("invalid icon data")
}

pub fn run(start: impl FnOnce(Proxy), handlers: Handlers, first_run: crate::setup::Setup) -> ! {
    let mut event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();
    let window_proxy = proxy.clone();

    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = menu_proxy.send_event(UiEvent::Menu(e.id));
    }));

    let icon_on = load_icon(include_bytes!("../assets/menubar-connected.png"));
    let icon_off = load_icon(include_bytes!("../assets/menubar-disconnected.png"));
    let voice_dark = load_icon(include_bytes!("../assets/menubar-voice-dark.png"));
    let voice_light = load_icon(include_bytes!("../assets/menubar-voice-light.png"));

    // ---- Tray menu ----
    let mut mu = MenuUi::new();
    let menu = Menu::new();
    menu.append_items(&[
        &mu.status_item,
        &mu.detail_item,
        &PredefinedMenuItem::separator(),
        &mu.release_item,
        &mu.settings_item,
        &mu.about_item,
        &PredefinedMenuItem::separator(),
        &mu.quit_item,
    ])
    .expect("failed to build the menu");
    // The permission row is inserted after the separator only while a permission is missing.
    const TRUST_POS: usize = 3;
    let mut trust_shown = false;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_tooltip(format!("{APP_NAME} · ReAI Vibe Board"))
        .with_icon_templated(icon_off.clone())
        .build()
        .expect("failed to create the menu bar item");

    // On a click on the status icon, activate this app before the menu opens.
    // The menu is shown in a nested event loop, and when the app is not frontmost the menu opened by the first click is closed again at once.
    tray_icon::TrayIconEvent::set_event_handler(Some(|e: tray_icon::TrayIconEvent| {
        use tray_icon::{MouseButtonState, TrayIconEvent};
        if let TrayIconEvent::Click {
            button_state: MouseButtonState::Down,
            ..
        } = e
        {
            activate_app();
        }
    }));

    let main_menu = install_main_menu();
    start(proxy);
    // Debugging aid: open the settings window right after start.
    if std::env::args().any(|a| a == "--open-settings") {
        let _ = window_proxy.send_event(UiEvent::Menu(mu.settings_item.id().clone()));
    }
    // The very first launch opens the settings window once, where the checklist is.
    let mut setup_state = first_run;
    if !setup_state.welcomed {
        setup_state.welcomed = true;
        crate::setup::save(&setup_state);
        let _ = window_proxy.send_event(UiEvent::Menu(mu.settings_item.id().clone()));
    }

    let mut icons = Icons {
        on: icon_on,
        off: icon_off,
        voice_dark,
        voice_light,
        shown: None,
        base_on: false,
        voice: false,
    };
    let (mic_state, mic_installed) = crate::mic::initial_state();
    let mut snap = Snapshot {
        autostart: crate::autostart::enabled(),
        setup_done: setup_state.completed,
        mic: Some(mic_state),
        mic_installed,
        ..Default::default()
    };
    let mut settings: Option<SettingsWindow> = None;
    let mut search_since: Option<std::time::Instant> = None;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        let ev = match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                if settings.as_ref().is_some_and(|s| s.id() == window_id) {
                    settings = None;
                    target.set_activation_policy_at_runtime(ActivationPolicy::Accessory);
                }
                return;
            }
            Event::UserEvent(ev) => ev,
            _ => return,
        };

        match ev {
            UiEvent::Status(s) => {
                match s {
                    Status::Searching => {
                        let since = *search_since.get_or_insert_with(std::time::Instant::now);
                        snap.search_hint = search_hint_due(since.elapsed());
                        mu.set_status(Msg::Searching);
                        mu.set_detail(Line::Msg(if snap.search_hint {
                            Msg::SearchHint
                        } else {
                            Msg::WakeHint
                        }));
                        snap.connected = false;
                        icons.set_base(false, &tray);
                    }
                    Status::Connecting => {
                        search_since = None;
                        snap.search_hint = false;
                        mu.release_item.set_enabled(true);
                        snap.known = true;
                        mu.set_status(Msg::Connecting);
                        mu.set_detail(Line::Raw(String::new()));
                    }
                    Status::Connected => {
                        search_since = None;
                        snap.search_hint = false;
                        mu.set_status(Msg::Connected);
                        mu.set_detail(Line::Msg(Msg::ReadingInfo));
                        snap.connected = true;
                        icons.set_base(true, &tray);
                    }
                    Status::Info { firmware, battery } => {
                        mu.set_detail(Line::Msg(Msg::Battery(battery)));
                        snap.battery = Some(battery);
                        snap.firmware = Some(firmware);
                    }
                    Status::Sleep { connected, .. } => snap.sleep = Some(connected),
                    Status::Mic { state, installed } => {
                        snap.mic = Some(state);
                        snap.mic_installed = installed;
                    }
                    Status::Disconnected => {
                        search_since = None;
                        snap.search_hint = false;
                        mu.set_status(Msg::Disconnected);
                        mu.set_detail(Line::Msg(Msg::WaitingReconnect));
                        snap.connected = false;
                        icons.set_base(false, &tray);
                    }
                    Status::Released(on) => {
                        search_since = None;
                        snap.search_hint = false;
                        snap.released = on;
                        put(
                            &mu.release_item,
                            if on {
                                Msg::ConnectKeyboard
                            } else {
                                Msg::DisconnectKeyboard
                            }
                            .text(),
                        );
                        if on {
                            mu.set_status(Msg::Disconnected);
                            mu.set_detail(Line::Msg(Msg::OthersCanConnect));
                            snap.connected = false;
                            icons.set_base(false, &tray);
                        }
                    }
                    Status::Mode(m) => {
                        snap.mode = Some(m.clone());
                        if let Some(w) = &settings {
                            w.post(&json!({ "t": "mode", "mode": m }));
                        }
                    }
                    Status::Input { name, pressed } => {
                        if let Some(w) = &settings {
                            w.post(&json!({ "t": "input", "name": name, "pressed": pressed }));
                        }
                    }
                    Status::Voice(down) => {
                        // While held: a yellow dot at the top right of the icon and a soft sound. The icon keeps its size, so nothing jumps.
                        icons.set_voice(down, &tray);
                        play_sound(if down { "Pop" } else { "Tink" });
                    }
                    Status::Error(m) => {
                        if let Some(w) = &settings {
                            w.post(&json!({ "t": "notice", "text": m }));
                        }
                        mu.set_error(m);
                    }
                }
                finish_setup(&mut snap, &mut setup_state);
                if let Some(w) = settings.as_mut() {
                    w.push_state(&snap);
                }
            }
            UiEvent::ConfigProblem(problem) => {
                if let (Some(w), Some(text)) = (&settings, &problem) {
                    w.post(&json!({ "t": "notice", "text": text }));
                }
                mu.set_config_problem(problem);
            }
            UiEvent::Trusted(ok) => {
                snap.trusted = Some(ok);
                finish_setup(&mut snap, &mut setup_state);
                if let Some(w) = settings.as_mut() {
                    w.push_state(&snap);
                }
                if !ok && !trust_shown {
                    let _ = menu.insert(&mu.trust_item, TRUST_POS);
                    trust_shown = true;
                } else if ok && trust_shown {
                    let _ = menu.remove(&mu.trust_item);
                    trust_shown = false;
                }
            }
            UiEvent::Menu(id) => {
                if id == *mu.quit_item.id() {
                    crate::logfile::drain();
                    std::process::exit(0);
                } else if id == *mu.release_item.id() {
                    (handlers.set_released)(!snap.released);
                } else if id == *mu.about_item.id() {
                    show_about();
                } else if id == *mu.settings_item.id() {
                    if let Some(w) = &settings {
                        w.focus();
                    } else {
                        // While the window is open the app becomes a regular app, which gives it a main menu and shortcuts like ⌘C and ⌘V.
                        target.set_activation_policy_at_runtime(ActivationPolicy::Regular);
                        match SettingsWindow::open(target, window_proxy.clone()) {
                            Ok(w) => settings = Some(w),
                            Err(e) => {
                                target
                                    .set_activation_policy_at_runtime(ActivationPolicy::Accessory);
                                mu.set_error(Msg::CannotOpenSettings(e.to_string()).text());
                            }
                        }
                    }
                    activate_app();
                } else if id == *mu.trust_item.id() {
                    open_accessibility_settings();
                }
            }
            UiEvent::Settings(text) => {
                handle_settings_message(&text, &mut settings, &mut snap, &handlers, &window_proxy);
            }
            UiEvent::LanguageChanged => {
                mu.relocalize(snap.released);
                if let Some(m) = &main_menu {
                    m.relocalize();
                }
                if let Some(w) = settings.as_mut() {
                    w.relocalize();
                }
            }
        }
    })
}

/// Handle a message from the web page in the settings window.
fn handle_settings_message(
    text: &str,
    settings: &mut Option<SettingsWindow>,
    snap: &mut Snapshot,
    handlers: &Handlers,
    proxy: &Proxy,
) {
    let Ok(msg) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(w) = settings.as_mut() else { return };
    match msg["t"].as_str().unwrap_or_default() {
        "load" => {
            w.push_config();
            w.push_state(snap);
        }
        "save" => match settings::save(msg["config"].clone()) {
            Ok(saved) => {
                w.post(&json!({ "t": "saved", "warnings": saved.warnings }));
                if saved.language_changed {
                    let _ = proxy.send_event(UiEvent::LanguageChanged);
                }
            }
            Err(e) => eprintln!("[settings] save failed: {e}"),
        },
        "sleep" => {
            if let Some(secs) = msg["secs"].as_u64().and_then(|s| u16::try_from(s).ok()) {
                (handlers.set_sleep)(Some(secs), None);
            }
        }
        "release" => (handlers.set_released)(msg["on"].as_bool().unwrap_or(false)),
        "mic" => (handlers.set_mic)(msg["on"].as_bool().unwrap_or(false)),
        "mic_remove" => (handlers.remove_mic)(),
        "autostart" => {
            let want = msg["on"].as_bool().unwrap_or(false);
            if want && !crate::autostart::can_enable() {
                w.post(&json!({ "t": "notice", "text": Msg::AutostartNeedsApplications.text() }));
            } else if let Err(e) = crate::autostart::set_enabled(want) {
                eprintln!("[settings] launch at login: {e}");
            }
            snap.autostart = crate::autostart::enabled();
            w.push_state(snap);
        }
        "about" => show_about(),
        // Only the repository URL declared in Cargo.toml is opened, never an arbitrary URL from the page.
        "open_repo" => {
            let _ = std::process::Command::new("open")
                .arg(env!("CARGO_PKG_REPOSITORY"))
                .spawn();
        }
        // Show the file in Finder so it can be dragged into a message. Nothing to show before the first write.
        "open_log" => {
            let path = crate::logfile::path();
            let mut open = std::process::Command::new("open");
            if path.exists() {
                open.arg("-R").arg(path);
            } else if let Some(dir) = path.parent() {
                open.arg(dir);
            }
            let _ = open.spawn();
        }
        "open_accessibility" => open_accessibility_settings(),
        "open_config" => {
            let _ = std::process::Command::new("open")
                .arg("-t")
                .arg(crate::config::config_path())
                .spawn();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_detail_line_shows_an_error_first_then_a_config_problem_then_the_normal_line() {
        let battery = Line::Raw("Battery 82%".into());
        let problem = Some("2 invalid entries".to_string());
        let error = Some("Could not open settings".to_string());
        assert_eq!(detail_text(&battery, &None, &None), "Battery 82%");
        assert_eq!(detail_text(&battery, &problem, &None), "2 invalid entries");
        // The error must stay visible while a config problem is pending.
        assert_eq!(
            detail_text(&battery, &problem, &error),
            "Could not open settings"
        );
        // A good reload clears the problem and the battery comes back.
        assert_eq!(detail_text(&battery, &None, &None), "Battery 82%");
    }

    #[test]
    fn the_search_hint_appears_only_after_fifteen_seconds() {
        assert!(!search_hint_due(Duration::from_secs(0)));
        assert!(!search_hint_due(Duration::from_secs(14)));
        assert!(search_hint_due(Duration::from_secs(15)));
        assert!(search_hint_due(Duration::from_secs(300)));
    }

    #[test]
    fn builtin_icons_decode_to_46x36_rgba() {
        for bytes in [
            include_bytes!("../assets/menubar-connected.png").as_slice(),
            include_bytes!("../assets/menubar-disconnected.png").as_slice(),
            include_bytes!("../assets/menubar-voice-dark.png").as_slice(),
            include_bytes!("../assets/menubar-voice-light.png").as_slice(),
        ] {
            let reader = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .unwrap();
            let info = reader.info();
            assert_eq!((info.width, info.height), (46, 36));
        }
    }
}
