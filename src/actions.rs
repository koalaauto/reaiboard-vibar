//! Action engine. Turns bindings into real system key presses.
//! Enigo is not necessarily Send, so it lives on a dedicated thread and callers hand it actions
//! through a channel.

use std::process::Stdio;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::config::{Binding, BindingDetail};
use crate::i18n::Msg;

/// One resolved system action
#[derive(Debug, Clone)]
pub enum Action {
    /// Modifiers down, click the main key, modifiers up
    Click(Vec<Key>, Key),
    /// Only press, or only release, one combination (push-to-talk)
    Hold(Vec<Key>, Key, Direction),
    Shell(String),
}

/// The actions a binding performs at press time and at release time
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

/// How often a missing Accessibility permission is checked again while no key is pressed.
const TRUST_POLL: Duration = Duration::from_secs(2);

/// Starts the executor thread. With `dry_run` it only prints and presses nothing.
/// Does not exit when the Accessibility permission is missing. It checks again every couple of
/// seconds and before every action, and reports changes through `on_trust`, so the UI notices the
/// permission being granted without a key press.
pub fn spawn_executor(
    dry_run: bool,
    on_trust: impl Fn(bool) + Send + 'static,
) -> Result<Sender<Action>> {
    let (tx, rx) = mpsc::channel::<Action>();
    thread::Builder::new()
        .name("actions".into())
        .spawn(move || {
            let mut enigo: Option<Enigo> = None;
            let mut trusted = None::<bool>;
            if !dry_run {
                // The first probe shows the system permission prompt.
                probe_trust(&mut enigo, true, &mut trusted, &on_trust);
            }
            loop {
                let action = match rx.recv_timeout(TRUST_POLL) {
                    Ok(action) => action,
                    Err(RecvTimeoutError::Timeout) => {
                        if !dry_run {
                            probe_trust(&mut enigo, false, &mut trusted, &on_trust);
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if dry_run {
                    println!("[dry-run] {action:?}");
                    continue;
                }
                probe_trust(&mut enigo, false, &mut trusted, &on_trust);
                if let Err(e) = run(&mut enigo, &action) {
                    eprintln!("[action failed] {action:?}: {e}");
                }
            }
        })?;
    Ok(tx)
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

/// Whether macOS lets this app press keys. Unlike creating an Enigo, asking does not log an error
/// and does not show a dialog, so it is safe to ask every couple of seconds.
fn accessibility_granted() -> bool {
    // SAFETY: a plain query with no arguments and no side effects.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Try to get a working Enigo if there is none yet, and report when the permission state changed.
/// `prompt` makes macOS show its permission dialog, which is only wanted once, at startup.
fn probe_trust(
    enigo: &mut Option<Enigo>,
    prompt: bool,
    trusted: &mut Option<bool>,
    on_trust: &impl Fn(bool),
) {
    // Without `prompt`, do not even try while the permission is missing: every failed attempt
    // makes the enigo library log an error, and that would fill the log file.
    if enigo.is_none() && (prompt || accessibility_granted()) {
        let settings = Settings {
            open_prompt_to_get_permissions: prompt,
            ..Default::default()
        };
        *enigo = Enigo::new(&settings).ok();
    }
    let ok = enigo.is_some();
    if *trusted != Some(ok) {
        *trusted = Some(ok);
        on_trust(ok);
    }
}

fn run(enigo: &mut Option<Enigo>, action: &Action) -> Result<()> {
    // A shell command presses no keys, so it must not depend on the Accessibility permission.
    if let Action::Shell(cmd) = action {
        return spawn_shell(cmd);
    }
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
        Action::Shell(_) => unreachable!("handled above, it needs no Accessibility"),
    }
    Ok(())
}

/// Fire and forget. The command's own output stays out of the log file, which people attach to
/// bug reports, and a thread waits for it so no zombie process is left.
fn spawn_shell(cmd: &str) -> Result<()> {
    let mut child = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_action_still_runs_with_its_output_detached() {
        let marker = std::env::temp_dir().join(format!("vibar-shell-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("echo to-the-void; echo done > '{}'", marker.display());
        run(&mut None, &Action::Shell(cmd)).unwrap();
        for _ in 0..100 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "done");
        let _ = std::fs::remove_file(&marker);
    }

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
