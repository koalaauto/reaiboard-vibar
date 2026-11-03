# Vibar

**Unofficial macOS menu bar client for the ReAI-Vibe-Board keyboard.**
Connect it over Bluetooth and remap every knob, key and mode lever, without installing the vendor app.

English | [简体中文](README.zh-CN.md)

<p align="center">
  <img src="https://ai-board.reai.com/img/aiboard/hardware-unibody.webp" alt="ReAI-Vibe-Board keyboard" width="560">
  <br>
  <sub>Product photo © ReAI, linked from the <a href="https://ai-board.reai.com/en">official site</a>.</sub>
</p>

> **Vibar is a community project and is not affiliated with ReAI.**
> "ReAI", "ReAI-Vibe-Board" and related names belong to their owners and are used here only to say which hardware Vibar works with.

## The keyboard

The [ReAI-Vibe-Board](https://ai-board.reai.com/en) (also written *ReAI Vibe Board*, *REAI Vibe Board*, or *AI-Board*) is a voice-first mechanical keyboard built around AI workflows. Hardware, as described by the maker:

- CNC aluminum unibody, metal knob that rotates and clicks, 6 keys
- A physical **3-position lever: CHAT, PLAN, YOLO**, meant as an AI permission switch (ask first, propose only, run on its own)
- Dual microphone array, USB-C and Bluetooth with automatic switching
- Open SDK: [`ReAI-com/reai-board-sdk`](https://github.com/ReAI-com/reai-board-sdk)

| | |
|---|---|
| USB | vendor id `0x363C`, product id `0xED20`, product string `REAI Vibe Board` |
| Bluetooth | advertises as `REAI_VB_xxxxxxxx`, vendor GATT service `FE60`–`FE63` |

## Why Vibar exists

Over Bluetooth the keyboard does **not** behave like a normal Bluetooth keyboard. It talks through a vendor-specific GATT service, so pairing it in *System Settings → Bluetooth* does nothing useful. Something has to subscribe to its notifications and turn knob turns, key presses and lever moves into real actions. The vendor ships a desktop app for that. Vibar is a small, open, hackable alternative built on the vendor's MIT-licensed SDK.

<p align="center">
  <img src="docs/images/settings.png" alt="Vibar settings window, keys page" width="640">
</p>

## Features

- **Bluetooth direct connection**, reconnects automatically after the keyboard wakes from sleep
- **Remap everything**: 3 knob inputs (counter-clockwise, clockwise, press) and 6 keys to a shortcut, hold-to-talk, media key, shell command, or nothing
- **Three layers, one per lever position.** Set defaults once, override only what differs in CHAT, YOLO or PLAN, and run an action when the lever enters or leaves a position
- **Settings window** with a live view: press a real key and it lights up on screen
- **Sleep time** of the keyboard, adjustable from the app (the factory 10 minutes is short)
- **Keyboard as a system microphone** through a virtual audio device, opened only while some app is recording
- **Hand the keyboard to another computer** with one click (disconnect, then connect on the other machine)
- Menu bar only, light and dark mode, optional launch at login

The interface is currently in Chinese. Translations are welcome, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Status

Version **0.1.0**, macOS only (the vendor ships a Windows app, Vibar does not target it yet).

| Verified on real hardware | |
|---|---|
| Environment | macOS 26.6, Apple Silicon, keyboard firmware 1.75, Bluetooth |
| Works | scan and connect, full event stream (knob, keys, lever), reconnect after sleep, knob to system volume, reading and writing the sleep time, virtual microphone driver install |
| **Not yet verified** | per-lever-position layers on hardware, knob direction calibration, microphone audio reaching a recording app, hand-over to a second computer, launch at login |

Intel Macs and other firmware versions are untested. Reports are welcome.

## Install

There is no prebuilt release yet. Build from source:

```bash
git clone https://github.com/koalaauto/reaiboard-vibar.git
cd reaiboard-vibar
scripts/bundle.sh --run      # builds build/Vibar.app and launches it
```

Requirements: macOS 12+, Rust 1.87+, `cmake` (compiles the virtual microphone driver, `brew install cmake`).

### Permissions and signing

Vibar asks for two permissions:

- **Bluetooth**, to talk to the keyboard
- **Accessibility**, to press keys on your behalf. The menu shows *需要辅助功能权限…* (accessibility permission needed) until it is granted.

macOS remembers a permission by **bundle id plus code signature**. `scripts/bundle.sh` signs with the first code-signing identity in your keychain. If it finds none it falls back to ad-hoc signing, and **an ad-hoc signature changes on every build, so you would have to grant the permissions again after each rebuild**. Details and how to avoid it: [docs/signing-and-permissions.md](docs/signing-and-permissions.md).

### First run checklist

1. Unplug USB. The keyboard does not advertise Bluetooth while it is plugged in.
2. Do **not** pair it in System Settings → Bluetooth. If you did, remove it there first, otherwise the system keeps the connection and Vibar cannot find it.
3. Press any key to wake it. The menu bar icon turns solid when connected.
4. Do not run the vendor app at the same time. Only one program can hold the keyboard.

## Use

Menu bar icon → **设置… (Settings)**. The default layout out of the box:

| Input | Default |
|---|---|
| Knob counter-clockwise / clockwise / press | `↑` / `↓` / `Return` |
| Tab, New, Esc, Action, Enter | `Tab`, `⌘N`, `Esc`, `⌘Return`, `Return` |
| Voice key | hold `F18` |

Set your push-to-talk or dictation tool to **hold F18** and the voice key becomes a talk button. Everything is also in a plain TOML file, see [docs/configuration.md](docs/configuration.md).

## Documentation

| | |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Modules, threads, data flow. Start here to change code. |
| [docs/configuration.md](docs/configuration.md) | Config file reference |
| [docs/hardware.md](docs/hardware.md) | What we learned about the keyboard: inputs, Bluetooth behavior, sleep, microphone |
| [docs/design-principles.md](docs/design-principles.md) | UI copy and icon rules |
| [docs/signing-and-permissions.md](docs/signing-and-permissions.md) | Code signing, permissions, releasing |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Setup, local CI, conventions |
| [AGENTS.md](AGENTS.md) | Guide for AI coding agents working in this repo |

## Safety

Apart from the **sleep time** that you change yourself, Vibar only reads from the keyboard. It never touches key configuration, firmware update or factory commands. Installing the virtual microphone needs an administrator password and restarts `coreaudiod`, so system audio drops for a second or two. You are asked first, and you can remove the driver from the settings window.

## Acknowledgements and license

- [`reai-board-sdk`](https://github.com/ReAI-com/reai-board-sdk) by ReAI (MIT) does the Bluetooth and protocol work. Its `msbc-decoder` is **LGPL-2.1-or-later**, see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- Built with [tao](https://github.com/tauri-apps/tao), [wry](https://github.com/tauri-apps/wry), [tray-icon](https://github.com/tauri-apps/tray-icon), [enigo](https://github.com/enigo-rs/enigo).

Vibar itself is released under the [MIT License](LICENSE).
