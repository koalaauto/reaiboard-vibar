//! The settings window: a native window around a web page (wry) that talks to the app by messages.
//! The page is `ui/settings.html`, its texts are `ui/strings.json`. This file owns reading and
//! writing the config and the state snapshot; the interface logic lives in the page.

use serde_json::{json, Value};
use tao::dpi::LogicalSize;
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::window::{Window, WindowBuilder, WindowId};
use wry::{WebView, WebViewBuilder};

use crate::config::{self, Config};
use crate::daemon::resolve_all;
use crate::i18n::{self, Lang, Msg};
use crate::mic::MicState;
use crate::ui::{Proxy, UiEvent};

const PAGE_TEMPLATE: &str = include_str!("../ui/settings.html");
const STRINGS: &str = include_str!("../ui/strings.json");

/// The page with `{{VERSION}}` and `{{REPO_SHORT}}` replaced by the values from `Cargo.toml`,
/// so neither is hard-coded in several places.
fn page() -> String {
    let repo = env!("CARGO_PKG_REPOSITORY");
    PAGE_TEMPLATE
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
        .replace(
            "{{REPO_SHORT}}",
            repo.trim_start_matches("https://github.com/"),
        )
}

/// All page texts for one language, as a JSON object.
pub fn strings_for(lang: Lang) -> Value {
    let all: Value = serde_json::from_str(STRINGS).expect("ui/strings.json is valid JSON");
    let key = match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
    };
    all[key].clone()
}

/// Everything the settings window shows about the running app. The UI updates it on every event.
#[derive(Default, Clone, PartialEq)]
pub struct Snapshot {
    pub connected: bool,
    pub released: bool,
    /// A keyboard to connect to is known, so the disconnect button can be enabled.
    pub known: bool,
    pub battery: Option<u8>,
    pub firmware: Option<String>,
    pub mode: Option<String>,
    pub sleep: Option<u16>,
    pub mic: Option<MicState>,
    pub mic_installed: bool,
    pub autostart: bool,
}

impl Snapshot {
    pub fn to_json(&self) -> Value {
        let mic = match self.mic.unwrap_or(MicState::Off) {
            MicState::Off => "off",
            MicState::Installing => "installing",
            MicState::WaitingKeyboard => "waiting",
            MicState::Idle => "idle",
            MicState::InUse => "inuse",
        };
        json!({
            "connected": self.connected,
            "released": self.released,
            "known": self.known,
            "battery": self.battery,
            "firmware": self.firmware,
            "mode": self.mode,
            "sleep": self.sleep,
            "mic": { "state": mic, "installed": self.mic_installed },
            "autostart": self.autostart,
            "configPath": config::config_path().display().to_string(),
        })
    }
}

pub struct SettingsWindow {
    window: Window,
    webview: WebView,
    last_state: String,
}

impl SettingsWindow {
    pub fn open(target: &EventLoopWindowTarget<UiEvent>, proxy: Proxy) -> anyhow::Result<Self> {
        let window = WindowBuilder::new()
            .with_title(Msg::SettingsWindowTitle.text())
            .with_inner_size(LogicalSize::new(840.0, 660.0))
            .with_min_inner_size(LogicalSize::new(780.0, 580.0))
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
            .build(target)?;
        let lang = i18n::current();
        let webview = WebViewBuilder::new()
            .with_html(page())
            // The page needs its texts before its own script runs. VIBAR_PAGE, VIBAR_LAYER,
            // VIBAR_SEL and VIBAR_THEME are debugging aids, see docs/architecture.md.
            .with_initialization_script(format!(
                "window.__lang = {}; window.__strings = {}; window.__v = {}; \
                 window.__page = {}; window.__layer = {}; window.__sel = {}; {}",
                json!(lang.code()),
                strings_for(lang),
                json!(env!("CARGO_PKG_VERSION")),
                json!(std::env::var("VIBAR_PAGE").unwrap_or_default()),
                json!(std::env::var("VIBAR_LAYER").unwrap_or_default()),
                json!(std::env::var("VIBAR_SEL").unwrap_or_default()),
                match std::env::var("VIBAR_THEME").as_deref() {
                    Ok(t @ ("dark" | "light")) =>
                        format!("document.documentElement.dataset.theme = '{t}';"),
                    _ => String::new(),
                }
            ))
            .with_ipc_handler(move |req| {
                let _ = proxy.send_event(UiEvent::Settings(req.body().clone()));
            })
            .build(&window)?;
        Ok(Self {
            window,
            webview,
            last_state: String::new(),
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn focus(&self) {
        self.window.set_minimized(false);
        self.window.set_focus();
    }

    pub fn post(&self, msg: &Value) {
        let _ = self
            .webview
            .evaluate_script(&format!("window.__host({msg})"));
    }

    /// Do not push when nothing changed.
    pub fn push_state(&mut self, snap: &Snapshot) {
        let msg = json!({ "t": "state", "state": snap.to_json() });
        let text = msg.to_string();
        if text != self.last_state {
            self.last_state = text;
            self.post(&msg);
        }
    }

    pub fn push_config(&self) {
        self.post(&json!({ "t": "config", "config": config_json() }));
    }

    /// The language changed: new window title and new page texts.
    pub fn relocalize(&mut self) {
        self.window.set_title(&Msg::SettingsWindowTitle.text());
        let lang = i18n::current();
        self.post(&json!({ "t": "lang", "lang": lang.code(), "strings": strings_for(lang) }));
    }
}

/// The current config file, for the page to render.
pub fn config_json() -> Value {
    config::load()
        .ok()
        .and_then(|c| serde_json::to_value(c).ok())
        .unwrap_or_else(|| json!({ "bindings": {} }))
}

/// The result of saving the config from the page.
pub struct Saved {
    /// Entries that cannot be used, already in the user's language.
    pub warnings: Vec<String>,
    /// The saved config switched the interface language.
    pub language_changed: bool,
}

/// Check the config that came from the page and write it to the file. The config watcher then
/// reloads the key bindings by itself.
pub fn save(value: Value) -> anyhow::Result<Saved> {
    let mut cfg: Config = serde_json::from_value(value)?;
    cfg.normalize();
    // Switch the language before resolving, so the warnings come out in the new language.
    let language_changed = i18n::set(i18n::resolve(cfg.language.as_deref()));
    let (cfg, _, warnings) = resolve_all(cfg);
    let text = config::to_toml(&cfg)?;
    let path = config::config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)?;
    Ok(Saved {
        warnings,
        language_changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn strings(lang: &str) -> serde_json::Map<String, Value> {
        let all: Value = serde_json::from_str(STRINGS).unwrap();
        all[lang].as_object().unwrap().clone()
    }

    /// Text that follows each occurrence of `needle` up to the next double quote.
    fn after_all<'a>(hay: &'a str, needle: &str) -> Vec<&'a str> {
        hay.split(needle)
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect()
    }

    /// The keys of standalone `t("...")` calls. A call like `createElement("div")` also ends in
    /// `t(` and must not count, so the character before has to be neither a word character nor
    /// `.` or `$`.
    fn t_call_keys(hay: &str) -> Vec<&str> {
        hay.match_indices("t(\"")
            .filter(|(i, _)| {
                hay[..*i]
                    .chars()
                    .last()
                    .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '$')))
            })
            .filter_map(|(i, m)| hay[i + m.len()..].split('"').next())
            .collect()
    }

    /// Placeholders such as `{n}` in a string, sorted.
    fn placeholders(s: &str) -> BTreeSet<String> {
        s.split('{')
            .skip(1)
            .filter_map(|rest| rest.split('}').next())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn page_posts_and_receives_the_same_message_types_as_the_host() {
        // Every message type the page sends must be one the app handles.
        for t in [
            "load",
            "save",
            "sleep",
            "release",
            "mic",
            "mic_remove",
            "autostart",
            "open_config",
            "about",
            "open_repo",
        ] {
            assert!(
                page().contains(&format!("t: \"{t}\"")),
                "the page never sends {t}"
            );
        }
    }

    #[test]
    fn version_placeholder_is_replaced_everywhere() {
        let p = page();
        assert!(!p.contains("{{VERSION}}"));
        assert!(p.contains(&format!("Vibar {}", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn repository_url_is_declared_once_in_cargo_toml() {
        let repo = env!("CARGO_PKG_REPOSITORY");
        assert!(repo.starts_with("https://github.com/"), "{repo}");
        assert!(page().contains(repo.trim_start_matches("https://github.com/")));
        assert!(!page().contains("{{REPO_SHORT}}"));
    }

    #[test]
    fn snapshot_json_has_every_field_the_page_reads() {
        let v = Snapshot::default().to_json();
        for key in [
            "connected",
            "released",
            "known",
            "battery",
            "firmware",
            "sleep",
            "mic",
            "autostart",
            "configPath",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn both_languages_define_exactly_the_same_keys() {
        let en: BTreeSet<_> = strings("en").keys().cloned().collect();
        let zh: BTreeSet<_> = strings("zh").keys().cloned().collect();
        let missing_zh: Vec<_> = en.difference(&zh).collect();
        let missing_en: Vec<_> = zh.difference(&en).collect();
        assert!(missing_zh.is_empty(), "missing in zh: {missing_zh:?}");
        assert!(missing_en.is_empty(), "missing in en: {missing_en:?}");
    }

    #[test]
    fn both_languages_use_the_same_placeholders() {
        let (en, zh) = (strings("en"), strings("zh"));
        for (key, value) in &en {
            let (a, b) = (
                placeholders(value.as_str().unwrap()),
                placeholders(zh[key].as_str().unwrap()),
            );
            assert_eq!(a, b, "placeholders differ for {key}");
        }
    }

    #[test]
    fn no_string_uses_parentheses_or_colons() {
        // Style rule for every language, see docs/design-principles.md.
        for lang in ["en", "zh"] {
            for (key, value) in strings(lang) {
                let text = value.as_str().unwrap();
                for bad in ['(', ')', '(', ')', ':', ':'] {
                    assert!(!text.contains(bad), "{lang}.{key} contains {bad}: {text}");
                }
            }
        }
    }

    #[test]
    fn no_string_is_empty() {
        for lang in ["en", "zh"] {
            for (key, value) in strings(lang) {
                assert!(
                    !value.as_str().unwrap().trim().is_empty(),
                    "{lang}.{key} is empty"
                );
            }
        }
    }

    #[test]
    fn the_page_contains_no_hard_coded_chinese() {
        // Every visible text must come from strings.json.
        let cjk = |c: char| ('\u{4e00}'..='\u{9fff}').contains(&c);
        let found: String = page().chars().filter(|c| cjk(*c)).collect();
        assert!(
            found.is_empty(),
            "hard-coded Chinese in settings.html: {found}"
        );
    }

    /// Key families that the page builds at run time, e.g. `t("media_" + k)`.
    const FAMILIES: &[(&str, &[&str])] = &[
        (
            "media_",
            &["volume_up", "volume_down", "mute", "play_pause"],
        ),
        (
            "sleep_",
            &["600", "1800", "3600", "14400", "43200", "custom"],
        ),
        (
            "mic_state_",
            &["off", "installing", "waiting", "idle", "inuse"],
        ),
        ("type_", &["key", "hold", "media", "shell", "none"]),
    ];

    #[test]
    fn every_key_the_page_asks_for_exists() {
        let html = page();
        let en = strings("en");
        let mut asked: Vec<&str> = after_all(&html, "data-i18n=\"");
        asked.extend(t_call_keys(&html));
        for key in asked {
            // `t("media_" + k)` is a family prefix, checked below
            if FAMILIES.iter().any(|(prefix, _)| *prefix == key) {
                continue;
            }
            assert!(en.contains_key(key), "the page asks for unknown key {key}");
        }
        for (prefix, suffixes) in FAMILIES {
            assert!(
                html.contains(&format!("t(\"{prefix}\" +")),
                "the page no longer builds {prefix} keys"
            );
            for s in *suffixes {
                assert!(
                    en.contains_key(&format!("{prefix}{s}")),
                    "missing {prefix}{s}"
                );
            }
        }
    }

    #[test]
    fn every_key_in_strings_json_is_used() {
        let html = page();
        for key in strings("en").keys() {
            let in_family = FAMILIES.iter().any(|(p, _)| key.starts_with(p));
            let used = in_family
                || html.contains(&format!("\"{key}\""))
                || html.contains(&format!("t(\"{key}\""));
            assert!(used, "strings.json has an unused key: {key}");
        }
    }

    #[test]
    fn strings_for_returns_the_right_language() {
        assert_eq!(strings_for(Lang::En)["nav_keys"], "Keys");
        assert_eq!(strings_for(Lang::Zh)["nav_keys"], "键位");
    }

    #[test]
    fn saved_layers_round_trip_through_the_page_format() {
        let json = json!({
            "bindings": { "tab": "tab", "voice": { "hold": "f18" } },
            "modes": { "plan": { "bindings": { "esc": { "none": true } }, "on_enter": "shift+tab" } }
        });
        let cfg: Config = serde_json::from_value(json).unwrap();
        let (_, km, bad) = resolve_all(cfg);
        assert!(bad.is_empty(), "{bad:?}");
        let none = km.lookup(Some("plan"), "esc").unwrap();
        assert!(
            none.on_press.is_none(),
            "a no-op binding must have no action"
        );
    }
}
