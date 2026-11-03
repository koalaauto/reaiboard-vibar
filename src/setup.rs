//! First-run state, so a new user is told what to do and an old user is left alone.
//!
//! Setup is complete once the keyboard is connected while Accessibility is granted. Until
//! then the settings window shows a short checklist. The window also opens by itself on the very
//! first launch, once. The state lives in `setup.toml` next to the config file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Setup {
    /// The settings window has been opened for the user at least once.
    #[serde(default)]
    pub welcomed: bool,
    /// The keyboard was connected while Accessibility was granted.
    #[serde(default)]
    pub completed: bool,
}

fn path() -> PathBuf {
    crate::config::config_path().with_file_name("setup.toml")
}

/// The state to start with. `config_existed` says whether a config file was there before this
/// launch. If it was, an earlier version has been used, and welcoming that person again would
/// be noise.
pub fn at_startup(config_existed: bool) -> Setup {
    resolve(config_existed, read(&path()))
}

fn resolve(config_existed: bool, stored: Option<Setup>) -> Setup {
    stored.unwrap_or(if config_existed {
        Setup {
            welcomed: true,
            completed: true,
        }
    } else {
        Setup::default()
    })
}

/// Setup is complete when the keyboard is connected and macOS lets us press keys.
/// `trusted` is `None` until the first report, which does not count.
pub fn is_complete(connected: bool, trusted: Option<bool>) -> bool {
    connected && trusted == Some(true)
}

/// Remember the state. Failing to write only means the checklist shows once more.
pub fn save(state: &Setup) {
    if let Err(e) = write(&path(), state) {
        eprintln!("[setup] could not save the first-run state: {e}");
    }
}

fn read(path: &Path) -> Option<Setup> {
    toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write(path: &Path, state: &Setup) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string(state).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_install_is_welcomed_and_an_existing_one_is_not() {
        assert_eq!(resolve(false, None), Setup::default());
        assert_eq!(
            resolve(true, None),
            Setup {
                welcomed: true,
                completed: true
            }
        );
    }

    #[test]
    fn stored_state_wins_over_the_config_file() {
        let half_way = Setup {
            welcomed: true,
            completed: false,
        };
        assert_eq!(resolve(true, Some(half_way)), half_way);
        assert_eq!(resolve(false, Some(half_way)), half_way);
    }

    #[test]
    fn setup_needs_the_keyboard_and_the_permission_together() {
        assert!(is_complete(true, Some(true)));
        assert!(!is_complete(true, Some(false)));
        assert!(!is_complete(true, None));
        assert!(!is_complete(false, Some(true)));
    }

    #[test]
    fn state_round_trips_and_a_broken_file_counts_as_missing() {
        let dir = std::env::temp_dir().join(format!("vibar-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("nested/setup.toml");
        assert_eq!(read(&file), None);
        let state = Setup {
            welcomed: true,
            completed: false,
        };
        write(&file, &state).unwrap();
        assert_eq!(read(&file), Some(state));
        std::fs::write(&file, "welcomed = [").unwrap();
        assert_eq!(read(&file), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
