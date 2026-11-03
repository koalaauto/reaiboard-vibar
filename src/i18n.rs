//! Localization for everything the user reads from the Rust side: menu, status lines, notices,
//! dialogs. The settings page has its own strings in `ui/strings.json` (see `settings.rs`).
//!
//! Every message is a variant of [`Msg`]. `en` and `zh` are exhaustive matches, so a missing
//! translation is a compile error, not a runtime surprise. Developer-facing text (logs, panics)
//! is plain English and does not go through here.
//!
//! Style rules for all languages are in `docs/design-principles.md`: no parentheses and no
//! colons. `tests::no_message_uses_parentheses_or_colons` enforces it.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// Code stored in the config file and shown to the settings page.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh-CN",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Lang::Zh
    } else {
        Lang::En
    }
}

/// Switch the language. Returns true if it actually changed.
pub fn set(lang: Lang) -> bool {
    let new = u8::from(lang == Lang::Zh);
    CURRENT.swap(new, Ordering::Relaxed) != new
}

/// Turn the config value into a language. `None`, `auto` and unknown values follow the system.
pub fn resolve(setting: Option<&str>) -> Lang {
    match setting.map(str::trim) {
        Some("en") => Lang::En,
        Some(s) if s.starts_with("zh") => Lang::Zh,
        _ => system_lang(),
    }
}

/// The first language in the user's preference list that we support, else English.
pub fn system_lang() -> Lang {
    use objc2_foundation::NSLocale;
    for lang in NSLocale::preferredLanguages().iter() {
        let lang = lang.to_string();
        if lang.starts_with("zh") {
            return Lang::Zh;
        }
        if lang.starts_with("en") {
            return Lang::En;
        }
    }
    Lang::En
}

/// A user-facing message from the Rust side.
#[derive(Clone, Debug)]
pub enum Msg {
    // Menu and main menu
    Starting,
    AccessibilityNeeded,
    DisconnectKeyboard,
    ConnectKeyboard,
    SettingsMenu,
    About,
    Quit,
    /// Launch at login was switched on while the app is not in an Applications folder.
    AutostartNeedsApplications,
    EditMenu,
    WindowMenu,
    SettingsWindowTitle,
    // Status lines
    Searching,
    WakeHint,
    /// Shown after searching for a while: the keyboard may be held by another computer.
    SearchHint,
    Connecting,
    Connected,
    ReadingInfo,
    Battery(u8),
    Disconnected,
    WaitingReconnect,
    OthersCanConnect,
    CannotOpenSettings(String),
    // Errors from the background session
    KeyboardNotConnected,
    MicInstallFailed(String),
    MicRemoveFailed(String),
    SleepReadFailed(String),
    SleepSetFailed(String),
    // Config notices
    ConfigUpdatedWithInvalid {
        count: usize,
        first: String,
    },
    ConfigInvalid {
        count: usize,
        first: String,
    },
    KeepingPrevious(String),
    UsingDefaults(String),
    ConfigLineError(usize),
    ConfigFormatError,
    // Config entries that cannot be used
    LabelInMode {
        mode: String,
        name: String,
    },
    LabelOnEnter(String),
    LabelOnLeave(String),
    NotAnInput(String),
    NotAMode(String),
    InvalidEntry {
        label: String,
        reason: String,
    },
    UnknownKey(String),
    UnknownModifier(String),
    UnknownMedia(String),
    EmptyShortcut,
    EmptyBinding,
    // Microphone driver dialog
    MicDialogTitle,
    MicDialogText,
    DialogCancel,
    DialogInstall,
}

impl Msg {
    pub fn text(&self) -> String {
        match current() {
            Lang::En => self.en(),
            Lang::Zh => self.zh(),
        }
    }

    fn en(&self) -> String {
        use Msg::*;
        match self {
            Starting => "Starting…".into(),
            AccessibilityNeeded => "Accessibility permission needed…".into(),
            DisconnectKeyboard => "Disconnect Keyboard".into(),
            ConnectKeyboard => "Connect Keyboard".into(),
            SettingsMenu => "Settings…".into(),
            About => "About Vibar".into(),
            Quit => "Quit Vibar".into(),
            AutostartNeedsApplications => "Move Vibar to the Applications folder first, then turn this on".into(),
            EditMenu => "Edit".into(),
            WindowMenu => "Window".into(),
            SettingsWindowTitle => "Vibar Settings".into(),
            Searching => "Searching for the keyboard…".into(),
            WakeHint => "Press any key to wake the keyboard".into(),
            SearchHint => "Still not found, is another computer connected to it?".into(),
            Connecting => "Connecting…".into(),
            Connected => "Connected".into(),
            ReadingInfo => "Reading device info…".into(),
            Battery(n) => format!("Battery {n}%"),
            Disconnected => "Disconnected".into(),
            WaitingReconnect => "Waiting to reconnect…".into(),
            OthersCanConnect => "Other computers can connect to the keyboard now".into(),
            CannotOpenSettings(e) => format!("Could not open the settings window, {e}"),
            KeyboardNotConnected => "The keyboard is not connected, nothing was written".into(),
            MicInstallFailed(e) => format!("Could not install the microphone driver, {e}"),
            MicRemoveFailed(e) => format!("Could not remove the microphone driver, {e}"),
            SleepReadFailed(e) => format!("Could not read the sleep setting, {e}"),
            SleepSetFailed(e) => format!("Could not change the sleep setting, {e}"),
            ConfigUpdatedWithInvalid { count, first } => {
                format!("Config updated, but {count} invalid, {first}")
            }
            ConfigInvalid { count, first } => format!("{count} invalid config entries, {first}"),
            KeepingPrevious(e) => format!("{e}, keeping the previous settings"),
            UsingDefaults(e) => format!("{e}, using the defaults"),
            ConfigLineError(l) => format!("Config format error on line {l}"),
            ConfigFormatError => "Config format error".into(),
            LabelInMode { mode, name } => format!("{name} in {}", mode.to_uppercase()),
            LabelOnEnter(m) => format!("entering {}", m.to_uppercase()),
            LabelOnLeave(m) => format!("leaving {}", m.to_uppercase()),
            NotAnInput(n) => format!("{n} is not a key on the keyboard"),
            NotAMode(m) => format!("{m} is not a lever position"),
            InvalidEntry { label, reason } => format!("{label} is invalid, {reason}"),
            UnknownKey(k) => format!("unknown key `{k}`"),
            UnknownModifier(m) => format!("unknown modifier `{m}`"),
            UnknownMedia(m) => format!("unknown media key `{m}`"),
            EmptyShortcut => "the shortcut is empty".into(),
            EmptyBinding => "no action is set".into(),
            MicDialogTitle => "Install the microphone driver".into(),
            MicDialogText => "Vibar needs to install an audio driver so the keyboard's microphone shows up in the system sound input list.\n\n· An administrator password is required\n· System audio drops for a second or two during installation\n· You can uninstall it any time in Settings".into(),
            DialogCancel => "Cancel".into(),
            DialogInstall => "Install".into(),
        }
    }

    fn zh(&self) -> String {
        use Msg::*;
        match self {
            Starting => "正在启动…".into(),
            AccessibilityNeeded => "需要辅助功能权限…".into(),
            DisconnectKeyboard => "断开键盘".into(),
            ConnectKeyboard => "连接键盘".into(),
            SettingsMenu => "设置…".into(),
            About => "关于 Vibar".into(),
            Quit => "退出 Vibar".into(),
            AutostartNeedsApplications => "请先把 Vibar 拖进应用程序文件夹,再打开这个开关".into(),
            EditMenu => "编辑".into(),
            WindowMenu => "窗口".into(),
            SettingsWindowTitle => "Vibar 设置".into(),
            Searching => "正在搜索键盘…".into(),
            WakeHint => "按任意键唤醒键盘".into(),
            SearchHint => "还没找到,是不是被别的电脑连着了?".into(),
            Connecting => "正在连接…".into(),
            Connected => "已连接".into(),
            ReadingInfo => "正在读取设备信息…".into(),
            Battery(n) => format!("电量 {n}%"),
            Disconnected => "已断开".into(),
            WaitingReconnect => "等待重新连接…".into(),
            OthersCanConnect => "别的电脑现在可以连接键盘".into(),
            CannotOpenSettings(e) => format!("打不开设置窗口,{e}"),
            KeyboardNotConnected => "键盘未连接,设置没有写入".into(),
            MicInstallFailed(e) => format!("麦克风驱动安装失败,{e}"),
            MicRemoveFailed(e) => format!("卸载麦克风驱动失败,{e}"),
            SleepReadFailed(e) => format!("读取休眠设置失败,{e}"),
            SleepSetFailed(e) => format!("修改休眠设置失败,{e}"),
            ConfigUpdatedWithInvalid { count, first } => {
                format!("已更新,但 {count} 项无效,{first}")
            }
            ConfigInvalid { count, first } => format!("{count} 项配置无效,{first}"),
            KeepingPrevious(e) => format!("{e},仍用旧设置"),
            UsingDefaults(e) => format!("{e},已使用默认设置"),
            ConfigLineError(l) => format!("配置第 {l} 行格式有误"),
            ConfigFormatError => "配置格式有误".into(),
            LabelInMode { mode, name } => format!("{} 档的 {name}", mode.to_uppercase()),
            LabelOnEnter(m) => format!("{} 档进入时", m.to_uppercase()),
            LabelOnLeave(m) => format!("{} 档离开时", m.to_uppercase()),
            NotAnInput(n) => format!("{n} 不是键盘上的按键"),
            NotAMode(m) => format!("{m} 不是拨杆的档位"),
            InvalidEntry { label, reason } => format!("{label} 无效,{reason}"),
            UnknownKey(k) => format!("未知按键 `{k}`"),
            UnknownModifier(m) => format!("未知修饰键 `{m}`"),
            UnknownMedia(m) => format!("未知媒体键 `{m}`"),
            EmptyShortcut => "快捷键是空的".into(),
            EmptyBinding => "没有设置任何动作".into(),
            MicDialogTitle => "安装麦克风驱动".into(),
            MicDialogText => "Vibar 需要安装一个音频驱动,键盘的麦克风才会出现在系统的声音输入列表里。\n\n· 需要输入管理员密码\n· 安装时系统声音会短暂中断一两秒\n· 随时可以在设置里卸载".into(),
            DialogCancel => "取消".into(),
            DialogInstall => "安装".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One value of every variant. The compiler cannot enforce this list, but
    /// `every_variant_has_a_sample` below fails if a variant is added without updating it.
    fn samples() -> Vec<Msg> {
        use Msg::*;
        let s = || "x".to_string();
        vec![
            Starting,
            AccessibilityNeeded,
            DisconnectKeyboard,
            ConnectKeyboard,
            SettingsMenu,
            About,
            Quit,
            AutostartNeedsApplications,
            EditMenu,
            WindowMenu,
            SettingsWindowTitle,
            Searching,
            WakeHint,
            SearchHint,
            Connecting,
            Connected,
            ReadingInfo,
            Battery(7),
            Disconnected,
            WaitingReconnect,
            OthersCanConnect,
            CannotOpenSettings(s()),
            KeyboardNotConnected,
            MicInstallFailed(s()),
            MicRemoveFailed(s()),
            SleepReadFailed(s()),
            SleepSetFailed(s()),
            ConfigUpdatedWithInvalid {
                count: 2,
                first: s(),
            },
            ConfigInvalid {
                count: 2,
                first: s(),
            },
            KeepingPrevious(s()),
            UsingDefaults(s()),
            ConfigLineError(4),
            ConfigFormatError,
            LabelInMode {
                mode: "plan".into(),
                name: s(),
            },
            LabelOnEnter("plan".into()),
            LabelOnLeave("yolo".into()),
            NotAnInput(s()),
            NotAMode(s()),
            InvalidEntry {
                label: s(),
                reason: s(),
            },
            UnknownKey(s()),
            UnknownModifier(s()),
            UnknownMedia(s()),
            EmptyShortcut,
            EmptyBinding,
            MicDialogTitle,
            MicDialogText,
            DialogCancel,
            DialogInstall,
        ]
    }

    #[test]
    fn every_variant_has_a_sample() {
        // Count the variants declared in this very file, so the sample list cannot silently fall behind.
        let src = include_str!("i18n.rs");
        let enum_body = src
            .split("pub enum Msg {")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        // A variant declaration is a line indented by exactly four spaces that starts with a
        // capital letter. Fields (eight spaces), closing braces and comments do not count, so
        // rustfmt reflowing a struct variant over several lines does not change the number.
        let declared = enum_body
            .lines()
            .filter(|l| {
                l.starts_with("    ")
                    && !l.starts_with("     ")
                    && l.trim_start()
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase())
            })
            .count();
        assert_eq!(
            declared,
            samples().len(),
            "add the new variant to samples()"
        );
    }

    #[test]
    fn both_languages_produce_text_for_every_message() {
        for m in samples() {
            let (en, zh) = (m.en(), m.zh());
            assert!(!en.trim().is_empty() && !zh.trim().is_empty(), "{m:?}");
            assert!(
                !en.contains("{") && !zh.contains("{"),
                "unfilled placeholder in {m:?}"
            );
        }
    }

    #[test]
    fn languages_differ_for_user_facing_words() {
        assert_ne!(Msg::Connected.en(), Msg::Connected.zh());
        assert_eq!(Msg::Battery(9).en(), "Battery 9%");
        assert_eq!(Msg::Battery(9).zh(), "电量 9%");
    }

    #[test]
    fn no_message_uses_parentheses_or_colons() {
        for m in samples() {
            for text in [m.en(), m.zh()] {
                for bad in ['(', ')', '(', ')', ':', ':'] {
                    assert!(!text.contains(bad), "{bad} in {text:?}");
                }
            }
        }
    }

    #[test]
    fn config_value_resolves_to_a_language() {
        assert_eq!(resolve(Some("en")), Lang::En);
        assert_eq!(resolve(Some("zh-CN")), Lang::Zh);
        assert_eq!(resolve(Some("zh")), Lang::Zh);
        // auto and garbage follow the system, which is one of the two
        let auto = resolve(Some("auto"));
        assert!(auto == Lang::En || auto == Lang::Zh);
        assert_eq!(resolve(None), auto);
        assert_eq!(resolve(Some("klingon")), auto);
    }

    #[test]
    fn language_codes_round_trip_through_resolve() {
        for l in [Lang::En, Lang::Zh] {
            assert_eq!(resolve(Some(l.code())), l);
        }
    }

    #[test]
    fn mode_names_are_shown_in_capitals() {
        let m = Msg::LabelInMode {
            mode: "plan".into(),
            name: "esc".into(),
        };
        assert!(m.en().contains("PLAN") && m.zh().contains("PLAN"));
    }
}
