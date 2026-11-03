//! 菜单栏外壳:tao 事件循环(必须在主线程) + tray-icon + 设置窗口。
//!
//! 菜单只放最少的东西:状态、断开 / 连接键盘、设置、退出。其余设置都在设置窗口里。
//! 约定:
//! - 正常的东西不出现:权限没问题时不显示权限行。
//! - 界面文字不用括号和冒号补充说明。
//! - 菜单项的文字和图标只在内容真的变化时才更新,展开着的菜单被改动会被 macOS 收起。

use std::io::Cursor;

use serde_json::{json, Value};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS, EventLoopWindowTargetExtMacOS};
use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIconBuilder};

use crate::daemon::Status;
use crate::settings::{self, SettingsWindow, Snapshot};

pub const APP_NAME: &str = "Vibar";

/// 后台线程发给界面的消息
#[derive(Debug, Clone)]
pub enum UiEvent {
    Status(Status),
    Trusted(bool),
    /// 一条临时提示,显示在次要信息行,下一次状态更新时被覆盖
    Notice(String),
    Menu(MenuId),
    /// 设置窗口里的网页发来的消息
    Settings(String),
}

pub type Proxy = EventLoopProxy<UiEvent>;

pub struct Handlers {
    /// 修改键盘休眠超时秒数,依次是已连接、断开后,None 表示不改
    pub set_sleep: Box<dyn Fn(Option<u16>, Option<u16>)>,
    /// 启用或关闭麦克风
    pub set_mic: Box<dyn Fn(bool)>,
    /// 移除麦克风驱动
    pub remove_mic: Box<dyn Fn()>,
    /// 断开键盘 true,或重新连接 false,用于把键盘借给别的电脑
    pub set_released: Box<dyn Fn(bool)>,
}

/// 当前应显示哪张图标。语音键按住时的黄点图标和普通图标尺寸完全相同,切换不会让位置跳动。
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
    /// 只在要显示的图标真的变了才换
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
            // 黄点是彩色的,不能用模板图标。按菜单栏此刻的深浅选白色或黑色机身
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

/// 菜单栏按钮此刻是不是深色外观,深色时图标应为白色。
/// 必须问按钮本身,macOS 的菜单栏颜色会跟着壁纸变,和系统的深浅色设置不一定一致。
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
    // SAFETY: 读取两个 AppKit 提供的外观名常量。
    unsafe {
        let appearance = button.effectiveAppearance();
        let names = NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]);
        appearance
            .bestMatchFromAppearancesWithNames(&names)
            .is_some_and(|n| &*n == NSAppearanceNameDarkAqua)
    }
}

/// 激活本应用,菜单栏应用默认不是前台应用
#[allow(deprecated)]
fn activate_app() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// macOS 标准的关于面板,图标、名称、版本号都取自 Info.plist
#[allow(deprecated)]
fn show_about() {
    activate_app();
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
    }
}

/// 播放一个系统提示音,不等待播放结束
fn play_sound(name: &str) {
    let path = format!("/System/Library/Sounds/{name}.aiff");
    if let Ok(mut child) = std::process::Command::new("afplay").arg(path).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// 文字没变就不动它
fn put(item: &MenuItem, text: impl AsRef<str>) {
    if item.text() != text.as_ref() {
        item.set_text(text.as_ref());
    }
}

/// 设置窗口打开时应用会切成普通应用,这时需要一个标准的主菜单,复制粘贴、关闭窗口等快捷键才有效
fn install_main_menu() {
    let build = || -> Result<Menu, tray_icon::menu::Error> {
        let app = Submenu::with_items(
            APP_NAME,
            true,
            &[
                &PredefinedMenuItem::about(Some(&format!("关于 {APP_NAME}")), None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(Some(&format!("退出 {APP_NAME}"))),
            ],
        )?;
        let edit = Submenu::with_items(
            "编辑",
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
            "窗口",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::close_window(None),
            ],
        )?;
        Menu::with_items(&[&app, &edit, &window])
    };
    if let Ok(menu) = build() {
        menu.init_for_nsapp();
        std::mem::forget(menu); // 主菜单要一直存在
    }
}

/// 解码内置的 PNG 图标,由 scripts/make-icons.py 生成
fn load_icon(png_bytes: &[u8]) -> Icon {
    let mut reader = png::Decoder::new(Cursor::new(png_bytes))
        .read_info()
        .expect("图标 PNG 无效");
    let mut buf = vec![0; reader.output_buffer_size().expect("图标尺寸")];
    let info = reader.next_frame(&mut buf).expect("图标解码失败");
    buf.truncate(info.buffer_size());
    Icon::from_rgba(buf, info.width, info.height).expect("图标数据无效")
}

pub fn run(start: impl FnOnce(Proxy), handlers: Handlers) -> ! {
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

    // ---- 菜单 ----
    let status_item = MenuItem::new("正在启动…", false, None);
    let detail_item = MenuItem::new("", false, None);
    let trust_item = MenuItem::new("需要辅助功能权限…", true, None);
    let release_item = MenuItem::new("断开键盘", false, None);
    let settings_item = MenuItem::new(
        "设置…",
        true,
        Some(Accelerator::new(Modifiers::META, Code::Comma)),
    );
    let about_item = MenuItem::new(format!("关于 {APP_NAME}"), true, None);
    let quit_item = MenuItem::new(
        format!("退出 {APP_NAME}"),
        true,
        Some(Accelerator::new(Modifiers::META, Code::KeyQ)),
    );

    let menu = Menu::new();
    menu.append_items(&[
        &status_item,
        &detail_item,
        &PredefinedMenuItem::separator(),
        &release_item,
        &settings_item,
        &about_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])
    .expect("构建菜单失败");
    // 权限行只在缺权限时插到分隔线之后,授权后移除
    const TRUST_POS: usize = 3;
    let mut trust_shown = false;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_tooltip(format!("{APP_NAME} · ReAI Vibe Board"))
        .with_icon_templated(icon_off.clone())
        .build()
        .expect("创建菜单栏图标失败");

    // 点击状态栏图标时先把本应用激活再弹菜单。
    // 菜单是在嵌套事件循环里显示的,应用不是前台时第一次点击弹出的菜单会被立刻收起。
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

    install_main_menu();
    start(proxy);
    // 调试用:启动后直接打开设置窗口
    if std::env::args().any(|a| a == "--open-settings") {
        let _ = window_proxy.send_event(UiEvent::Menu(settings_item.id().clone()));
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
        mic: Some(mic_state),
        mic_installed,
        ..Default::default()
    };
    let mut settings: Option<SettingsWindow> = None;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        let ev = match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, window_id, .. } => {
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
                        put(&status_item, "正在搜索键盘…");
                        put(&detail_item, "按任意键唤醒键盘");
                        snap.connected = false;
                        icons.set_base(false, &tray);
                    }
                    Status::Connecting => {
                        release_item.set_enabled(true);
                        snap.known = true;
                        put(&status_item, "正在连接…");
                        put(&detail_item, "");
                    }
                    Status::Connected => {
                        put(&status_item, "已连接");
                        put(&detail_item, "正在读取设备信息…");
                        snap.connected = true;
                        icons.set_base(true, &tray);
                    }
                    Status::Info { firmware, battery } => {
                        put(&detail_item, format!("电量 {battery}%"));
                        snap.battery = Some(battery);
                        snap.firmware = Some(firmware);
                    }
                    Status::Sleep { connected, .. } => snap.sleep = Some(connected),
                    Status::Mic { state, installed } => {
                        snap.mic = Some(state);
                        snap.mic_installed = installed;
                    }
                    Status::Disconnected => {
                        put(&status_item, "已断开");
                        put(&detail_item, "等待重新连接…");
                        snap.connected = false;
                        icons.set_base(false, &tray);
                    }
                    Status::Released(on) => {
                        snap.released = on;
                        put(&release_item, if on { "连接键盘" } else { "断开键盘" });
                        if on {
                            put(&status_item, "已断开");
                            put(&detail_item, "别的电脑现在可以连接键盘");
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
                        // 按住时图标右上角亮黄点并有一声轻提示。图标尺寸不变,位置不会跳
                        icons.set_voice(down, &tray);
                        play_sound(if down { "Pop" } else { "Tink" });
                    }
                    Status::Error(m) => {
                        if let Some(w) = &settings {
                            w.post(&json!({ "t": "notice", "text": m }));
                        }
                        put(&detail_item, m);
                    }
                }
                if let Some(w) = settings.as_mut() {
                    w.push_state(&snap);
                }
            }
            UiEvent::Notice(m) => {
                if let Some(w) = &settings {
                    w.post(&json!({ "t": "notice", "text": m }));
                }
                put(&detail_item, m);
            }
            UiEvent::Trusted(ok) => {
                if !ok && !trust_shown {
                    let _ = menu.insert(&trust_item, TRUST_POS);
                    trust_shown = true;
                } else if ok && trust_shown {
                    let _ = menu.remove(&trust_item);
                    trust_shown = false;
                }
            }
            UiEvent::Menu(id) => {
                if id == *quit_item.id() {
                    std::process::exit(0);
                } else if id == *release_item.id() {
                    (handlers.set_released)(!snap.released);
                } else if id == *about_item.id() {
                    show_about();
                } else if id == *settings_item.id() {
                    if let Some(w) = &settings {
                        w.focus();
                    } else {
                        // 窗口打开期间变成普通应用,才有主菜单和 ⌘C ⌘V 这些快捷键
                        target.set_activation_policy_at_runtime(ActivationPolicy::Regular);
                        match SettingsWindow::open(target, window_proxy.clone()) {
                            Ok(w) => settings = Some(w),
                            Err(e) => {
                                target.set_activation_policy_at_runtime(ActivationPolicy::Accessory);
                                put(&detail_item, format!("打不开设置窗口,{e}"));
                            }
                        }
                    }
                    activate_app();
                } else if id == *trust_item.id() {
                    let _ = std::process::Command::new("open")
                        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                        .spawn();
                }
            }
            UiEvent::Settings(text) => {
                handle_settings_message(&text, &mut settings, &mut snap, &handlers);
            }
        }
    })
}

/// 处理设置窗口里网页发来的消息
fn handle_settings_message(
    text: &str,
    settings: &mut Option<SettingsWindow>,
    snap: &mut Snapshot,
    handlers: &Handlers,
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
            Ok(bad) => w.post(&json!({ "t": "saved", "warnings": bad })),
            Err(e) => eprintln!("[设置] 保存失败 {e}"),
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
            if let Err(e) = crate::autostart::set_enabled(want) {
                eprintln!("[设置] 开机启动 {e}");
            }
            snap.autostart = crate::autostart::enabled();
            w.push_state(snap);
        }
        "about" => show_about(),
        // 只打开 Cargo.toml 里声明的仓库地址,不接受页面传来的任意网址
        "open_repo" => {
            let _ = std::process::Command::new("open")
                .arg(env!("CARGO_PKG_REPOSITORY"))
                .spawn();
        }
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
