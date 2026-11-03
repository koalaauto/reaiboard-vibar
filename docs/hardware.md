# The keyboard: what we know

Notes on the ReAI-Vibe-Board as seen by Vibar. Sources: the maker's [website](https://ai-board.reai.com/en), the vendor SDK ([`ReAI-com/reai-board-sdk`](https://github.com/ReAI-com/reai-board-sdk)), and measurements on one unit (firmware 1.75, macOS 26.6, Apple Silicon). Anything not measured by us is marked.

## Names and identifiers

| What | Value |
|---|---|
| Product names in the wild | ReAI-Vibe-Board, ReAI Vibe Board, REAI Vibe Board, AI-Board |
| USB vendor / product id | `0x363C` / `0xED20` (the Bluetooth side reports the same product id) |
| USB strings | manufacturer `REAI`, product `REAI Vibe Board` |
| Bluetooth name | `REAI_VB_` followed by 8 hex digits, different per unit |
| Bluetooth services | vendor GATT, `FE60` to `FE63`: command `FE61`, event `FE62`, audio `FE63` |
| DFU mode | USB product id `0xFF06`. Vibar never enters it. |

## Inputs

Twelve input slots, reported by the SDK as `key_index` 0 to 11. Over Bluetooth every press arrives as two events: *pressed* (source Consumer, a usage code `0x0F01` to `0x0F0B`) and *released* (source Gatt, value 0). There are no repeat events while held; time a long press yourself.

| key_index | Firmware label | Physical input | Vibar name |
|---|---|---|---|
| 0 | volume phase A | knob, one direction | `knob_ccw` (until calibrated) |
| 1 | volume phase B | knob, other direction | `knob_cw` (until calibrated) |
| 2 | volume press | knob pressed | `knob_press` |
| 3 | Tab | key | `tab` |
| 4 | New | key | `new` |
| 5 | Esc | key | `esc` |
| 6 | AI voice | key. Also reported as a separate `AiVoiceKey` event. | `voice` |
| 7 | Action | key | `action` |
| 8 | Enter | key | `enter` |
| 9 | YOLO lever | lever moved to YOLO | handled through `ModeChange` |
| 10 | PLAN lever | lever moved to PLAN | handled through `ModeChange` |
| 11 | CHAT lever | slot exists. Moving back to CHAT produced **no key event** in our tests, only `ModeChange(CHAT)`. | handled through `ModeChange` |

Notes:

- One knob detent is one pair of events, A or B. **Which of A and B is clockwise is not verified**, which is why the settings window has a calibration step and the config has `knob_swap`.
- Lever positions are best read from `ModeChange` (`CHAT` 0x00, `YOLO` 0x01, `PLAN` 0x02). The first `ModeChange` after connecting reports the current position and is not a move.
- What the maker intends the lever to mean (from the website): CHAT asks before acting, PLAN proposes without running, YOLO runs on its own.

## Bluetooth behavior

- Over Bluetooth the keyboard is **not** a standard HID keyboard. A program must subscribe to its GATT notifications. System pairing is unnecessary and harmful: a system-paired keyboard stays connected to the system and stops advertising.
- It does **not advertise** while a USB cable is plugged in, and not while asleep. A key press wakes it and it starts advertising again.
- One central at a time. Whoever connects first holds it. To hand it over, disconnect on the first computer (Vibar: *断开键盘*).
- The SDK does not connect by name prefix on its own in a Bluetooth-only setup. Scan first, then connect to a named device. Vibar scans until something appears, then connects.
- Reconnect after sleep worked in our tests without intervention.
- Handshake in our tests: mode `Legacy`, no host binding.

### Host binding (firmware 1.75 and newer)

The firmware can bind to one computer, which is set up over USB with a 16-byte token, so that only that computer may use Bluetooth. Vibar does **not** use it. A keyboard that is bound to another computer may refuse Vibar; plugging it into the new computer over USB rebinds it. We have not seen a bound unit.

## Sleep

Two idle timeouts, stored in the keyboard and persistent across power cycles (firmware 1.51 and newer):

| | Default | Range |
|---|---|---|
| connected | 600 s | 30 to 65535 s |
| disconnected | 60 s | 30 to 65535 s |

Vibar reads both after connecting and writes only when you change the value in the settings. We set the connected timeout to 4 hours on our unit and read it back.

## Microphone

- Board audio is mSBC, 16 kHz mono, sent over the vendor channel (USB vendor HID or Bluetooth `FE63`). From firmware 1.59 it needs no system microphone permission because the system never sees a recording device.
- Over Bluetooth the system gets no audio device for the keyboard, so the SDK ships a CoreAudio plug-in that publishes one: `ReAI-Vibe-Board`, 1 channel, 16 kHz. Audio is pushed to it over a loopback UDP port.
- The stream is requested with a lease that must be renewed (Vibar renews every 2 s). Older firmware uses a session-only stream; the SDK falls back to it.
- Installing the plug-in needs an administrator password and restarts `coreaudiod`. The bundle is ad-hoc signed by the SDK build.

## What we have not measured

Battery life under different sleep settings, Intel Macs, firmware other than 1.75, USB-cable operation through Vibar, a host-bound unit, and the audio path into a real recording app.
