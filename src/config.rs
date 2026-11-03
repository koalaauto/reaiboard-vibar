//! 配置:~/.config/vibar/config.toml。设置窗口读写它,首次运行自动写入默认配置。
//!
//! 结构:
//!   [bindings]            默认层,所有拨杆档位都生效
//!   [modes.plan]          拨杆在 PLAN 档时,覆盖默认层里的某几个键,没写的沿用默认
//!   on_enter / on_leave   进入、离开某一档时触发的动作

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Msg;

pub const DEFAULT_CONFIG: &str = include_str!("../default-config.toml");

/// 键盘上全部可配置的输入,界面和配置文件都用这些名字
pub const INPUTS: &[&str] = &[
    "knob_ccw",
    "knob_cw",
    "knob_press",
    "tab",
    "new",
    "esc",
    "voice",
    "action",
    "enter",
];

/// 拨杆的三档
pub const MODES: &[&str] = &["chat", "yolo", "plan"];

/// 一个绑定。字符串简写 = 点击快捷键;表写法可选 hold / media / shell。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Binding {
    Shortcut(String),
    Detail(BindingDetail),
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct BindingDetail {
    /// 点击一次的快捷键,如 "cmd+shift+4"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// 按下时按住、松开时释放,适合按住说话
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    /// 媒体键 volume_up volume_down mute play_pause
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// 执行 shell 命令
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// 什么也不做。在拨杆某一档里用来"取消"默认层的绑定
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub none: Option<bool>,
}

/// 拨杆某一档的设置
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ModeConfig {
    /// 在这一档里覆盖默认层的键
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, Binding>,
    /// 拨到这一档时触发
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_enter: Option<Binding>,
    /// 离开这一档时触发
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_leave: Option<Binding>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// 指定要连接的蓝牙名,不填则连扫描到的第一块 REAI_VB_
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// 旋钮方向反了就打开,界面里的「校准旋钮」会帮你设置
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub knob_swap: bool,
    /// Interface language. Absent or "auto" follows the system, otherwise "en" or "zh-CN".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, ModeConfig>,
}

impl Config {
    /// 把旧版配置的写法迁移到当前结构:
    /// knob_a / knob_b 改名为 knob_ccw / knob_cw,mode_xxx 变成该档的 on_enter。
    pub fn normalize(&mut self) {
        for (old, new) in [("knob_a", "knob_ccw"), ("knob_b", "knob_cw")] {
            if let Some(b) = self.bindings.remove(old) {
                self.bindings.entry(new.to_string()).or_insert(b);
            }
        }
        for mode in MODES {
            if let Some(b) = self.bindings.remove(&format!("mode_{mode}")) {
                let m = self.modes.entry(mode.to_string()).or_default();
                m.on_enter.get_or_insert(b);
            }
        }
    }
}

/// 配置文件位置。环境变量 VIBAR_CONFIG 可以改指向别处,方便测试时不碰真实配置。
pub fn config_path() -> PathBuf {
    path_from(
        std::env::var_os("VIBAR_CONFIG"),
        &std::env::var("HOME").unwrap_or_else(|_| ".".into()),
    )
}

fn path_from(override_path: Option<std::ffi::OsString>, home: &str) -> PathBuf {
    match override_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(home).join(".config/vibar/config.toml"),
    }
}

/// 旧版本叫 reai-bar。首次运行时把旧配置目录里的文件搬过来,并清掉旧的开机启动项。
pub fn migrate_legacy() {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    let (old, new) = (home.join(".config/reai-bar"), home.join(".config/vibar"));
    if old.exists() && !new.exists() && std::fs::create_dir_all(&new).is_ok() {
        for f in ["config.toml", "state.toml"] {
            let _ = std::fs::copy(old.join(f), new.join(f));
        }
        println!(
            "Migrated the old config from {} to {}",
            old.display(),
            new.display()
        );
    }
    let agent = home.join("Library/LaunchAgents/dev.koala.reai-bar.plist");
    if agent.exists() {
        let _ = std::fs::remove_file(agent);
    }
}

pub fn load() -> anyhow::Result<Config> {
    let path = config_path();
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, DEFAULT_CONFIG)?;
        println!("Wrote the default config to {}", path.display());
    }
    let text = std::fs::read_to_string(&path)?;
    parse(&text).map_err(|e| {
        println!("[config] {} {e}", path.display()); // full reason goes to the log
        let line = e
            .span()
            .map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1);
        match line {
            Some(l) => anyhow::anyhow!(Msg::ConfigLineError(l).text()),
            None => anyhow::anyhow!(Msg::ConfigFormatError.text()),
        }
    })
}

/// Read only the `language` value, leniently, so the right language is known before the
/// config is fully parsed (a broken config still has to be reported in the user's language).
pub fn peek_language() -> Option<String> {
    let text = std::fs::read_to_string(config_path()).ok()?;
    let table: toml::Table = text.parse().ok()?;
    table.get("language")?.as_str().map(str::to_string)
}

pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
    let mut cfg: Config = toml::from_str(text)?;
    cfg.normalize();
    Ok(cfg)
}

/// 序列化成配置文件内容,供设置窗口保存
pub fn to_toml(cfg: &Config) -> anyhow::Result<String> {
    let body = toml::to_string(cfg)?;
    Ok(format!(
        "# Generated by the Vibar settings window. You can also edit it by hand, changes apply within about a second.\n\n{body}"
    ))
}

/// 按键索引转成输入名。旋钮每格是 A 或 B 一对脉冲,哪个是顺时针由 knob_swap 决定。
pub fn key_name(index: usize, knob_swap: bool) -> Option<&'static str> {
    Some(match index {
        0 => {
            if knob_swap {
                "knob_cw"
            } else {
                "knob_ccw"
            }
        }
        1 => {
            if knob_swap {
                "knob_ccw"
            } else {
                "knob_cw"
            }
        }
        2 => "knob_press",
        3 => "tab",
        4 => "new",
        5 => "esc",
        6 => "voice",
        7 => "action",
        8 => "enter",
        // 9、10 拨杆由 ModeChange 事件统一处理,这里不绑定,避免同一次拨动触发两次
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(s: &str) -> Binding {
        Binding::Shortcut(s.into())
    }

    #[test]
    fn language_is_optional_and_round_trips() {
        assert_eq!(parse("[bindings]\n").unwrap().language, None);
        let cfg = parse("language = \"zh-CN\"\n[bindings]\n").unwrap();
        assert_eq!(cfg.language.as_deref(), Some("zh-CN"));
        assert_eq!(parse(&to_toml(&cfg).unwrap()).unwrap(), cfg);
        assert!(!to_toml(&Config::default()).unwrap().contains("language"));
    }

    #[test]
    fn config_path_defaults_under_home_and_can_be_overridden() {
        assert_eq!(
            path_from(None, "/h"),
            PathBuf::from("/h/.config/vibar/config.toml")
        );
        assert_eq!(
            path_from(Some("".into()), "/h"),
            PathBuf::from("/h/.config/vibar/config.toml")
        );
        assert_eq!(
            path_from(Some("/tmp/x.toml".into()), "/h"),
            PathBuf::from("/tmp/x.toml")
        );
    }

    #[test]
    fn default_config_parses_and_every_binding_resolves() {
        let cfg = parse(DEFAULT_CONFIG).expect("the default config must parse");
        assert!(!cfg.bindings.is_empty());
        for (name, b) in &cfg.bindings {
            crate::actions::resolve(b)
                .unwrap_or_else(|e| panic!("default binding `{name}` is invalid {e}"));
        }
    }

    #[test]
    fn default_config_covers_every_input() {
        let cfg = parse(DEFAULT_CONFIG).unwrap();
        for input in INPUTS {
            assert!(
                cfg.bindings.contains_key(*input),
                "the default config lacks {input}"
            );
        }
    }

    #[test]
    fn default_config_only_uses_known_input_names() {
        let cfg = parse(DEFAULT_CONFIG).unwrap();
        for name in cfg.bindings.keys() {
            assert!(INPUTS.contains(&name.as_str()), "unknown input `{name}`");
        }
    }

    #[test]
    fn every_input_is_reachable_from_some_key_index() {
        for input in INPUTS {
            assert!(
                (0..=8).any(|i| key_name(i, false) == Some(*input)),
                "{input} has no matching key index"
            );
        }
    }

    #[test]
    fn levers_are_not_bound_by_key_index() {
        assert_eq!(key_name(9, false), None);
        assert_eq!(key_name(10, false), None);
    }

    #[test]
    fn knob_swap_exchanges_the_two_directions() {
        assert_eq!(key_name(0, false), Some("knob_ccw"));
        assert_eq!(key_name(1, false), Some("knob_cw"));
        assert_eq!(key_name(0, true), Some("knob_cw"));
        assert_eq!(key_name(1, true), Some("knob_ccw"));
    }

    #[test]
    fn string_and_table_bindings_both_deserialize() {
        let cfg = parse("[bindings]\na = \"cmd+c\"\n[bindings.b]\nhold = \"f18\"\n").unwrap();
        assert!(matches!(cfg.bindings["a"], Binding::Shortcut(_)));
        assert!(matches!(cfg.bindings["b"], Binding::Detail(_)));
    }

    #[test]
    fn broken_toml_is_rejected() {
        assert!(parse("[bindings]\ntab = \n").is_err());
        assert!(parse("[bindings]\ntab = \"tab\"\n").is_ok());
    }

    #[test]
    fn old_knob_names_and_mode_keys_are_migrated() {
        let cfg = parse(
            "[bindings]\nknob_a = { media = \"volume_down\" }\nknob_b = { media = \"volume_up\" }\nmode_plan = \"shift+tab\"\ntab = \"tab\"\n",
        )
        .unwrap();
        assert!(cfg.bindings.contains_key("knob_ccw") && cfg.bindings.contains_key("knob_cw"));
        assert!(!cfg.bindings.contains_key("knob_a") && !cfg.bindings.contains_key("mode_plan"));
        assert_eq!(cfg.modes["plan"].on_enter, Some(sc("shift+tab")));
    }

    #[test]
    fn migration_never_overwrites_a_new_style_entry() {
        let cfg = parse("[bindings]\nknob_a = \"left\"\nknob_ccw = \"up\"\n").unwrap();
        assert_eq!(cfg.bindings["knob_ccw"], sc("up"));
    }

    #[test]
    fn serialize_then_parse_round_trips_including_layers() {
        let mut cfg = parse(DEFAULT_CONFIG).unwrap();
        cfg.knob_swap = true;
        let plan = cfg.modes.entry("plan".into()).or_default();
        plan.bindings.insert("enter".into(), sc("cmd+return"));
        plan.bindings.insert(
            "voice".into(),
            Binding::Detail(BindingDetail {
                hold: Some("f19".into()),
                ..Default::default()
            }),
        );
        plan.on_enter = Some(sc("shift+tab"));
        let text = to_toml(&cfg).unwrap();
        assert_eq!(parse(&text).unwrap(), cfg, "{text}");
    }

    #[test]
    fn empty_layers_are_not_written_out() {
        let text = to_toml(&parse(DEFAULT_CONFIG).unwrap()).unwrap();
        assert!(!text.contains("[modes"));
        assert!(!text.contains("knob_swap"));
    }
}
