//! 动作引擎:把绑定变成真实的系统按键。
//! Enigo 不一定 Send,所以放在专用线程里,外部通过 channel 投递。

use std::sync::mpsc::{self, Sender};
use std::thread;

use anyhow::{anyhow, bail, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::config::{Binding, BindingDetail};
use crate::i18n::Msg;

/// 解析后的一次系统动作
#[derive(Debug, Clone)]
pub enum Action {
    /// 修饰键按下 → 主键点击 → 修饰键释放
    Click(Vec<Key>, Key),
    /// 仅按下 / 仅释放一个组合(push-to-talk)
    Hold(Vec<Key>, Key, Direction),
    Shell(String),
}

/// 一个绑定在「按下」「松开」两个时刻各自要做的动作
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub on_press: Option<Action>,
    pub on_release: Option<Action>,
}

pub fn resolve(binding: &Binding) -> Result<Resolved> {
    let d = match binding {
        Binding::Shortcut(s) => BindingDetail {
            key: Some(s.clone()),
            ..Default::default()
        },
        Binding::Detail(d) => d.clone(),
    };
    let mut r = Resolved::default();
    if d.none == Some(true) {
        return Ok(r);
    }
    if let Some(s) = &d.key {
        let (mods, key) = parse_shortcut(s)?;
        r.on_press = Some(Action::Click(mods, key));
    } else if let Some(s) = &d.hold {
        let (mods, key) = parse_shortcut(s)?;
        r.on_press = Some(Action::Hold(mods.clone(), key, Direction::Press));
        r.on_release = Some(Action::Hold(mods, key, Direction::Release));
    } else if let Some(m) = &d.media {
        let key = match m.as_str() {
            "volume_up" => Key::VolumeUp,
            "volume_down" => Key::VolumeDown,
            "mute" => Key::VolumeMute,
            "play_pause" => Key::MediaPlayPause,
            other => bail!("{}", Msg::UnknownMedia(other.to_string()).text()),
        };
        r.on_press = Some(Action::Click(vec![], key));
    } else if let Some(cmd) = &d.shell {
        r.on_press = Some(Action::Shell(cmd.clone()));
    } else {
        bail!("{}", Msg::EmptyBinding.text());
    }
    Ok(r)
}

/// "cmd+shift+4" → ([Meta, Shift], Unicode('4'))
pub fn parse_shortcut(s: &str) -> Result<(Vec<Key>, Key)> {
    let parts: Vec<String> = s.split('+').map(|p| p.trim().to_lowercase()).collect();
    let (last, mods) = parts
        .split_last()
        .ok_or_else(|| anyhow!(Msg::EmptyShortcut.text()))?;
    let mut mod_keys = Vec::new();
    for m in mods {
        mod_keys.push(match m.as_str() {
            "cmd" | "command" | "meta" => Key::Meta,
            "ctrl" | "control" => Key::Control,
            "alt" | "option" | "opt" => Key::Alt,
            "shift" => Key::Shift,
            other => bail!("{}", Msg::UnknownModifier(other.to_string()).text()),
        });
    }
    Ok((
        mod_keys,
        parse_key(last).ok_or_else(|| anyhow!(Msg::UnknownKey(last.to_string()).text()))?,
    ))
}

fn parse_key(name: &str) -> Option<Key> {
    Some(match name {
        "tab" => Key::Tab,
        "return" | "enter" => Key::Return,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "f13" => Key::F13,
        "f14" => Key::F14,
        "f15" => Key::F15,
        "f16" => Key::F16,
        "f17" => Key::F17,
        "f18" => Key::F18,
        "f19" => Key::F19,
        "f20" => Key::F20,
        s if s.chars().count() == 1 => Key::Unicode(s.chars().next()?),
        _ => return None,
    })
}

/// 启动执行线程;`dry_run` 时只打印不真按。
/// 缺少「辅助功能」权限时不退出:每次动作前重试,权限状态变化通过 `on_trust` 上报。
pub fn spawn_executor(
    dry_run: bool,
    on_trust: impl Fn(bool) + Send + 'static,
) -> Result<Sender<Action>> {
    let (tx, rx) = mpsc::channel::<Action>();
    thread::Builder::new()
        .name("actions".into())
        .spawn(move || {
            let mut enigo: Option<Enigo> = None;
            let mut first = true;
            let mut trusted = None::<bool>;
            if !dry_run {
                // 启动时先探测一次(这次会弹系统授权提示),让界面一开始就知道权限状态
                enigo = Enigo::new(&Settings::default()).ok();
                first = false;
                trusted = Some(enigo.is_some());
                on_trust(enigo.is_some());
            }
            for action in rx {
                if dry_run {
                    println!("[dry-run] {action:?}");
                    continue;
                }
                if enigo.is_none() {
                    // 只在第一次尝试时弹系统授权提示,之后静默重试
                    let settings = Settings {
                        open_prompt_to_get_permissions: first,
                        ..Default::default()
                    };
                    first = false;
                    enigo = Enigo::new(&settings).ok();
                }
                let ok = enigo.is_some();
                if trusted != Some(ok) {
                    trusted = Some(ok);
                    on_trust(ok);
                }
                if let Err(e) = run(&mut enigo, &action) {
                    eprintln!("[action failed] {action:?}: {e}");
                }
            }
        })?;
    Ok(tx)
}

fn run(enigo: &mut Option<Enigo>, action: &Action) -> Result<()> {
    let Some(enigo) = enigo else {
        bail!("Accessibility permission is missing, action dropped");
    };
    match action {
        Action::Click(mods, key) => {
            for m in mods {
                enigo
                    .key(*m, Direction::Press)
                    .map_err(|e| anyhow!("{e}"))?;
            }
            let res = enigo.key(*key, Direction::Click);
            for m in mods.iter().rev() {
                let _ = enigo.key(*m, Direction::Release);
            }
            res.map_err(|e| anyhow!("{e}"))?;
        }
        Action::Hold(mods, key, dir) => match dir {
            Direction::Release => {
                enigo
                    .key(*key, Direction::Release)
                    .map_err(|e| anyhow!("{e}"))?;
                for m in mods.iter().rev() {
                    let _ = enigo.key(*m, Direction::Release);
                }
            }
            _ => {
                for m in mods {
                    enigo
                        .key(*m, Direction::Press)
                        .map_err(|e| anyhow!("{e}"))?;
                }
                enigo.key(*key, *dir).map_err(|e| anyhow!("{e}"))?;
            }
        },
        Action::Shell(cmd) => {
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(cmd)
                .spawn()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbg<T: std::fmt::Debug>(v: &T) -> String {
        format!("{v:?}")
    }

    #[test]
    fn parses_modifiers_and_key() {
        let (mods, key) = parse_shortcut("cmd+shift+4").unwrap();
        assert_eq!(dbg(&mods), "[Meta, Shift]");
        assert_eq!(dbg(&key), "Unicode('4')");
    }

    #[test]
    fn modifier_aliases_and_case_are_accepted() {
        let (mods, _) = parse_shortcut("Command + OPTION + Ctrl + c").unwrap();
        assert_eq!(dbg(&mods), "[Meta, Alt, Control]");
    }

    #[test]
    fn parses_named_and_function_keys() {
        for (name, want) in [
            ("tab", "Tab"),
            ("enter", "Return"),
            ("esc", "Escape"),
            ("f18", "F18"),
            ("pagedown", "PageDown"),
        ] {
            let (mods, key) = parse_shortcut(name).unwrap();
            assert!(mods.is_empty());
            assert_eq!(dbg(&key), want, "{name}");
        }
    }

    #[test]
    fn rejects_unknown_modifier_and_key() {
        assert!(parse_shortcut("hyper+a")
            .unwrap_err()
            .to_string()
            .contains("hyper"));
        assert!(parse_shortcut("cmd+nonsense")
            .unwrap_err()
            .to_string()
            .contains("nonsense"));
        assert!(parse_shortcut("").is_err());
    }

    #[test]
    fn shortcut_binding_clicks_on_press_only() {
        let r = resolve(&Binding::Shortcut("cmd+n".into())).unwrap();
        assert!(matches!(r.on_press, Some(Action::Click(..))));
        assert!(r.on_release.is_none());
    }

    #[test]
    fn hold_binding_presses_then_releases() {
        let b = Binding::Detail(BindingDetail {
            hold: Some("f18".into()),
            ..Default::default()
        });
        let r = resolve(&b).unwrap();
        assert!(matches!(
            r.on_press,
            Some(Action::Hold(_, _, Direction::Press))
        ));
        assert!(matches!(
            r.on_release,
            Some(Action::Hold(_, _, Direction::Release))
        ));
    }

    #[test]
    fn media_and_shell_bindings() {
        let media = Binding::Detail(BindingDetail {
            media: Some("volume_up".into()),
            ..Default::default()
        });
        assert!(
            matches!(resolve(&media).unwrap().on_press, Some(Action::Click(m, _)) if m.is_empty())
        );
        let shell = Binding::Detail(BindingDetail {
            shell: Some("true".into()),
            ..Default::default()
        });
        assert!(matches!(
            resolve(&shell).unwrap().on_press,
            Some(Action::Shell(_))
        ));
        let bad = Binding::Detail(BindingDetail {
            media: Some("rewind".into()),
            ..Default::default()
        });
        assert!(resolve(&bad).is_err());
    }

    #[test]
    fn empty_binding_is_an_error() {
        assert!(resolve(&Binding::Detail(BindingDetail::default())).is_err());
    }
}
