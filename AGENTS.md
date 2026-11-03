# AGENTS.md

Guide for AI coding agents working in this repository. Humans should read [CONTRIBUTING.md](CONTRIBUTING.md); everything here is consistent with it.

## What this is

Vibar: a macOS menu bar app, one Rust crate, that connects to the ReAI-Vibe-Board keyboard over Bluetooth and maps its knob, keys and 3-position lever to system actions. Settings window is a `wry` web view (`ui/settings.html`). Vendor SDK is a pinned git dependency. Start with [docs/architecture.md](docs/architecture.md).

## Commands

| Task | Command |
|---|---|
| Everything a commit needs | `scripts/ci.sh` (add `--fix` to run rustfmt first) |
| Tests only | `cargo test --release` |
| Build and launch the app | `scripts/bundle.sh --run` |
| Run without a keyboard or without pressing keys | `target/release/vibar --headless --dry-run` |
| Regenerate icons | `python3 scripts/make-icons.py` (needs Pillow and numpy) |

There is no GitHub Actions workflow on purpose. `scripts/ci.sh` is the CI. Run it before declaring work done.

## Map

`src/daemon.rs` Bluetooth session and key handling. `src/config.rs` config model. `src/actions.rs` key simulation. `src/mic.rs` and `src/coreaudio.rs` microphone. `src/ui.rs` menu bar. `src/settings.rs` and `ui/settings.html` settings window. Details and extension recipes are in `docs/architecture.md`.

## Invariants. Do not break these

1. **Key release uses the binding chosen at press time** (`held` map in `daemon::run`). Never look the binding up again on release.
2. **The only write to the keyboard is the sleep time**, on explicit user action, and skipped when unchanged. Never add key configuration, DFU, factory or shutdown commands.
3. **Menu items and the web view are touched only from the main thread**, through `UiEvent`. Update a menu item only when its content changed (`ui::put`); changing an open menu closes it.
4. **The settings page never saves an empty shortcut, hold key or command.** It keeps a draft.
5. **UI strings have no parentheses and no colons.** A unit test enforces this for the settings page; keep menu strings in `ui.rs` to the same rule. See `docs/design-principles.md`.
6. **Icons come from `scripts/make-icons.py`.** Edit the script, regenerate, commit both.
7. **The repository URL lives only in `Cargo.toml`.** Rust reads `env!("CARGO_PKG_REPOSITORY")`, the page and bundle script read it from there.
8. **The sink for microphone audio is installed before the SDK `start()`.**

## Working safely on this machine

- **Bluetooth, Accessibility and administrator prompts need a human.** Do not try to click them away. Say what is waiting and stop.
- **Test through the `.app`, not the bare binary.** A bare binary started from your shell is killed by macOS on first Bluetooth use. `scripts/devapp.sh <binary> <name> [args]` wraps one into a throwaway app.
- **Screenshots: capture the window by id** (`screencapture -l <id>`), never a screen region. A region captures whatever is in front, including the user's private windows. Find the id with `CGWindowListCopyWindowInfo`; `docs/architecture.md` lists the `VIBAR_*` environment variables and `--open-settings` that put the settings window into any state. Delete screenshots when done.
- **Never run `rm` with an unguarded variable.** Use `"${VAR:?}"/path` or a literal path.
- Launching via `open` loses stdout. Use `open -n --stdout FILE --stderr FILE build/Vibar.app`.
- Do not install or remove the audio driver, and do not change the keyboard's sleep time, as a side effect of testing. Those are user decisions.

## Definition of done

- `scripts/ci.sh` passes.
- New behavior has tests, a bug fix has a regression test.
- Docs changed with the code (`docs/`, README tables, the settings page contract test in `settings.rs`).
- For UI changes: looked at light and dark with window screenshots.
- If something could not be verified on real hardware, say so in the commit message or the pull request. Do not claim it works.

## Style

Rust 2021, `rustfmt` defaults, clippy clean with `-D warnings`. Comments explain why, not what. Code comments are mostly Chinese today; match the surrounding file. Keep functions small, prefer pure functions that can be unit tested (see `config.rs` and `Keymap`) and keep platform calls at the edges.
