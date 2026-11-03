# Contributing

Thanks for helping. Vibar is small on purpose, so a change is easy to review when it is small too.

## Set up

```bash
git clone https://github.com/koalaauto/reaiboard-vibar.git
cd reaiboard-vibar
brew install cmake          # compiles the virtual microphone driver
scripts/bundle.sh --run     # build/Vibar.app, signed, launched
```

Needs macOS 12+, Rust 1.87+. Read [docs/signing-and-permissions.md](docs/signing-and-permissions.md) first, otherwise you will grant Bluetooth and Accessibility again after every build.

You can work on most things without the keyboard: `cargo test`, and `vibar --headless --dry-run`. Hardware changes need a real ReAI-Vibe-Board.

Read [docs/architecture.md](docs/architecture.md) to find where a change belongs.

## Local CI

This project deliberately has **no GitHub Actions**. Checks run on your machine:

```bash
scripts/ci.sh          # rustfmt check, clippy with warnings as errors, tests, release build
scripts/ci.sh --fix    # apply rustfmt first
```

It must pass before you commit. To enforce that automatically, add it as a hook:

```bash
printf '#!/bin/sh\nexec scripts/ci.sh\n' > .git/hooks/pre-push && chmod +x .git/hooks/pre-push
```

## What a good change looks like

- **One purpose per commit.** Message in the imperative, with the why in the body: `Keep key release bound to the layer it was pressed in`.
- **Tests with behavior.** Parsing, config, layering and state machines have unit tests next to the code. New logic gets tests. A bug fix gets a test that fails without the fix.
- **No new warnings.** Clippy runs with `-D warnings`. Fix the cause instead of adding `#[allow]`; if an allow is truly needed, say why in a comment.
- **Docs move with code.** Changing a config key, a flag or a message type means updating `docs/` in the same commit.
- **UI changes follow [docs/design-principles.md](docs/design-principles.md)** and come with screenshots in the pull request, light and dark. Capture the window by its id, not a screen region.

## Safety rules for hardware code

- Do not add anything that writes to the keyboard unless the user asked for that exact thing in the UI. Today the only write is the sleep time.
- Never call key configuration, firmware update (DFU), factory test or shutdown commands. They can leave a unit in a bad state.
- Anything that needs an administrator password (driver install) must ask first, in plain words, and be reversible.
- Do not add network access. Vibar talks to a keyboard and to your system, nothing else.

## Language and translations

Docs are English with a Chinese README. The interface comes in English and Chinese, chosen by `language` in the config (`auto` follows macOS). All user-visible text is in two places, and tests keep the languages in step:

- `src/i18n.rs` for the menu, status lines, notices and dialogs
- `ui/strings.json` for the settings window

Comments, logs, issues and pull requests may be English or Chinese; English is preferred for new code. To add a third language: add a `Lang` variant and a `code`, a third `match` arm set in `Msg`, a third block in `ui/strings.json`, teach `i18n::resolve` and `system_lang` about it, add an `.lproj` folder in `scripts/bundle.sh` and an option in the page's language list. Open an issue first so we agree on the language code and who maintains it.

## Reporting hardware results

Other firmware versions, Intel Macs and a second computer are the biggest unknowns. In an issue include: macOS version and chip, keyboard firmware (shown in Settings → Keyboard), what you did, and the output of:

```bash
open -n --stdout /tmp/vibar.out --stderr /tmp/vibar.err build/Vibar.app && sleep 20 && cat /tmp/vibar.out
```

Remove anything personal from the log before posting. It can contain the keyboard's Bluetooth name.

## Licensing

By contributing you agree that your work is released under the [MIT License](LICENSE). Do not paste code or assets from the vendor's closed-source apps; implement behavior from observation and the public SDK instead.
