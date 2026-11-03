# Third-party notices

Vibar is MIT licensed. It links the open source packages below. Almost all of the roughly 170 crates in the dependency tree on macOS are under permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0). One is not, and it is described first.

## msbc-decoder: LGPL-2.1-or-later

- Source: <https://github.com/ReAI-com/reai-board-sdk>, directory `msbc-decoder/`. Pulled in by the SDK's `ble` and `usb` features.
- Purpose: decodes the keyboard's mSBC audio. It is linked statically into the Vibar binary even when audio is unused.
- How the LGPL is met: Vibar's complete source is public, so anyone can replace the library and rebuild. For example, point the `reai-board-sdk` dependency in `Cargo.toml` at a checkout that contains your modified `msbc-decoder`.
- If you redistribute a Vibar binary, include this file and tell recipients where to get the corresponding source (this repository, at the commit you built).

## reai-board-sdk: MIT

- Source: <https://github.com/ReAI-com/reai-board-sdk>, by ReAI. Provides Bluetooth and USB connection, the keyboard protocol, reconnection and the virtual microphone driver sources.
- Pinned to a tested commit in `Cargo.toml`. Its protocol constants are specific to the ReAI-Vibe-Board.

## Octicons mark: MIT

The GitHub mark on the settings window's General page is from [Octicons](https://github.com/primer/octicons) by GitHub, MIT licensed. GitHub and the GitHub logo are trademarks of GitHub, Inc.

## Other direct dependencies

`tokio`, `serde`, `serde_json`, `toml`, `anyhow`, `env_logger`, `libc`, `png`, `tao`, `wry`, `tray-icon`, `enigo`, `core-foundation`, `coreaudio-sys`, `objc2`, `objc2-app-kit`, `objc2-foundation`. See each crate for its license.

To list every license in the tree:

```bash
cargo metadata --format-version 1 --filter-platform aarch64-apple-darwin \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(sorted({p['license'] or 'UNKNOWN' for p in d['packages']}))"
```

## Trademarks and affiliation

Vibar is an independent community project. It is **not** made, endorsed or supported by ReAI. "ReAI", "ReAI-Vibe-Board" and related names and the product photo linked from the README belong to their owners. They are used only to identify the hardware Vibar works with. If an owner asks for a change, we will make it.
