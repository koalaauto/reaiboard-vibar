//! 设置窗口:一个内嵌网页的原生窗口(wry),通过消息和程序主体通信。
//! 页面在 ui/settings.html。配置的读写、状态快照都在这里,界面逻辑在页面里。

use serde_json::{json, Value};
use tao::dpi::LogicalSize;
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::window::{Window, WindowBuilder, WindowId};
use wry::{WebView, WebViewBuilder};

use crate::config::{self, Config};
use crate::daemon::resolve_all;
use crate::mic::MicState;
use crate::ui::{Proxy, UiEvent};

const PAGE_TEMPLATE: &str = include_str!("../ui/settings.html");

/// 页面里的 {{VERSION}} 和 {{REPO_SHORT}} 换成 Cargo.toml 里的版本号和仓库名,不在多处硬编码
fn page() -> String {
    let repo = env!("CARGO_PKG_REPOSITORY");
    PAGE_TEMPLATE
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
        .replace(
            "{{REPO_SHORT}}",
            repo.trim_start_matches("https://github.com/"),
        )
}

/// 设置窗口需要展示的全部运行状态。界面逻辑每处理一次事件就更新它。
#[derive(Default, Clone, PartialEq)]
pub struct Snapshot {
    pub connected: bool,
    pub released: bool,
    /// 已经知道要连哪块键盘,断开按钮才可用
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
            .with_title("Vibar 设置")
            .with_inner_size(LogicalSize::new(840.0, 660.0))
            .with_min_inner_size(LogicalSize::new(780.0, 580.0))
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
            .build(target)?;
        let webview = WebViewBuilder::new()
            .with_html(page())
            // 调试用:环境变量 VIBAR_PAGE 指定打开哪一页,VIBAR_THEME 强制深色或浅色
            .with_initialization_script(format!(
                "window.__page = {}; window.__layer = {}; window.__sel = {}; {}",
                serde_json::to_string(&std::env::var("VIBAR_PAGE").unwrap_or_default())
                    .unwrap_or_default(),
                serde_json::to_string(&std::env::var("VIBAR_LAYER").unwrap_or_default())
                    .unwrap_or_default(),
                serde_json::to_string(&std::env::var("VIBAR_SEL").unwrap_or_default())
                    .unwrap_or_default(),
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

    /// 状态没变就不推送
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
}

/// 当前配置文件的内容,给页面渲染
pub fn config_json() -> Value {
    config::load()
        .ok()
        .and_then(|c| serde_json::to_value(c).ok())
        .unwrap_or_else(|| json!({ "bindings": {} }))
}

/// 页面传来的配置:检查、写入文件,返回无效项说明。写入后文件监视线程会自动重新载入。
pub fn save(value: Value) -> anyhow::Result<Vec<String>> {
    let mut cfg: Config = serde_json::from_value(value)?;
    cfg.normalize();
    let (cfg, _, bad) = resolve_all(cfg);
    let text = config::to_toml(&cfg)?;
    let path = config::config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)?;
    Ok(bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_posts_and_receives_the_same_message_types_as_the_host() {
        // 页面里发出的消息类型,必须是程序主体会处理的
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
            assert!(page().contains(&format!("t: \"{t}\"")), "页面没有发送 {t}");
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

    /// 页面里读者能看到的文字:去掉样式、脚本和标签
    fn visible_text() -> String {
        let full = page();
        let html = full.split("<script>").next().unwrap();
        let (head, rest) = html.split_once("</style>").unwrap();
        let _ = head;
        let mut out = String::new();
        let mut in_tag = false;
        for c in rest.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => out.push(c),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn page_text_avoids_parentheses_and_colons_in_visible_copy() {
        // 界面文字约定:不用括号和冒号补充说明
        let visible = visible_text();
        for bad in ['(', ')', '(', ')', ':', ':'] {
            assert!(!visible.contains(bad), "界面文字里有 {bad} {visible}");
        }
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
        assert!(none.on_press.is_none(), "无操作的绑定不应有动作");
    }
}
