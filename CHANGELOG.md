# Changelog

All notable changes to Vibar. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [semantic versioning](https://semver.org/).

## [Unreleased]

### Added
- English and Chinese interface that follows the system language, switchable in Settings → General.
- A hint in the menu and the settings window when the keyboard has not been found for 15 seconds, pointing at Bluetooth permission and at another computer holding the keyboard.
- `scripts/dmg.sh` to package the app into a disk image, and `scripts/dmg.sh --adhoc` for internal builds that need no certificate, with a bilingual install note inside.
- A first-run checklist at the top of the Keys page, with Accessibility and the keyboard ticked off live, and the settings window opening once on the very first launch. It never shows for someone who already has a config file. State is in `setup.toml` next to the config.
- A log file at `~/Library/Logs/Vibar/vibar.log` when the app is started from Finder or at login, with a time stamp on every line and rotation at 1 MB. Settings → General shows its path and opens it in Finder. `VIBAR_LOG` writes it elsewhere.
- `VIBAR_CONFIG` environment variable to use another config file.
- `docs/releasing.md`.

### Fixed
- Shell command bindings did nothing while the Accessibility permission was missing, although they press no keys.
- Saving settings replaced the battery line in the menu with "Config updated, 9 entries", and the battery came back only after a reconnect. A successful reload now says nothing. A config problem shows in that line until it is fixed, then the battery line returns.

### Changed
- The keyboard drawn in the settings window now matches the real keyboard: capital keycap legends, a microphone symbol on the voice key, a tall ENTER and a wide ACTION, lever positions in the order of the lights.
- Launch at login can only be switched on when the app is in an Applications folder.
- Logs and panic messages are English.

## [0.1.0] - not released yet

First public version of the source.

### Added
- Bluetooth connection to the ReAI-Vibe-Board keyboard through the vendor SDK, with reconnection after sleep.
- Remapping of the knob (three inputs), six keys and the three-position lever, with a layer per lever position and enter and leave actions.
- Settings window with a live view of pressed inputs, keyboard sleep time, hand-over to another computer, launch at login.
- Keyboard as a system microphone through a virtual audio device that opens only while an app records.
- Menu bar item with a voice-key indicator, light and dark mode.
