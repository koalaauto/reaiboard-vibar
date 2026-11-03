# Releasing

How to turn the repository into something other people can install, and what is still missing.

## What we can build today

```bash
scripts/bundle.sh      # build/Vibar.app
scripts/dmg.sh         # build/Vibar-<version>-<arch>.dmg, with a shortcut to /Applications
```

The disk image holds `Vibar.app` and an `Applications` shortcut to drag it onto. The file name states the architecture that is really inside. Today that is `arm64`, so it runs on Apple Silicon Macs only.

## Who can open it without a warning

This is decided by how the app is signed, not by the disk image.

| Signature | What the person receiving the DMG sees |
|---|---|
| Ad-hoc, or Apple Development (what we have today) | Gatekeeper **rejects** the app on other Macs when the file came from a browser, AirDrop or a chat app (it carries a quarantine flag). `spctl --assess` on our build says *rejected*. |
| Developer ID Application, notarized and stapled | Opens after the usual "downloaded from the Internet" confirmation. This is what a public release needs. |

A person who trusts the source can still open an unnotarized build:

- **macOS 15 and later**: try to open it once, then System Settings → Privacy & Security → *Open Anyway*. Control-click → Open no longer bypasses the check.
- **Earlier macOS**: Control-click the app → Open → Open.
- **Any version**, from a terminal: `xattr -dr com.apple.quarantine /Applications/Vibar.app`.

Files copied by USB stick or `scp` carry no quarantine flag and open normally. That is enough for handing a build to a colleague, but not for a public download page.

A Developer ID certificate needs a paid Apple Developer Program membership. The Apple Development certificate in your keychain cannot be used for distribution. Steps: [signing-and-permissions.md](signing-and-permissions.md).

## Internal builds, no certificate needed

For colleagues and testers an ad-hoc signature is enough:

```bash
scripts/bundle.sh
scripts/dmg.sh --adhoc      # build/Vibar-<version>-<arch>-adhoc.dmg
```

`--adhoc` re-signs only the copy inside the disk image, so your own `build/Vibar.app` and its permissions stay as they are. The image also gets a bilingual *Read me first.txt* with the install steps, because nobody reads a README before double-clicking.

What to expect, checked on a real build:

| | Ad-hoc | Apple Development (what `bundle.sh` uses on your machine) |
|---|---|---|
| Signature is valid on any Mac | yes | yes |
| Gatekeeper on a quarantined download | rejected | rejected, the same |
| What macOS remembers permissions by | the hash of this exact build | the bundle id plus the certificate |
| After an update | **Bluetooth and Accessibility are asked again** | still granted |
| Depends on one person's certificate | no | yes, and it expires after a year |

Both are blocked by Gatekeeper in the same way, so the recipient needs one of the workarounds above, or a copy without a quarantine flag (USB stick, `scp`, a network share). Browsers, AirDrop and most chat apps set the flag.

Apple silicon only for now, see the open items.

## Open items before a public release

1. **Developer ID signing and notarization** (above). Not done, never run for Vibar.
2. **The bundled audio driver under notarization.** Notarization scans every executable inside the app, including `ReAIVibeBoard.driver`, so it must be signed with the Developer ID and the hardened runtime too. Today `bundle.sh` deliberately leaves the SDK's ad-hoc signature alone. That rule has to be revisited, and the microphone must be tested on a notarized build, because it is not certain that `coreaudiod` still loads the re-signed plug-in.
3. **Intel Macs.** The binary is arm64 only. A universal build needs `rustup target add x86_64-apple-darwin`, a build for each architecture and `lipo -create`. The driver is already universal.
4. **Hardware checks** that the README still lists as not verified: layers on hardware, knob calibration, microphone audio reaching a recording app, handing the keyboard to a second computer, launch at login.
5. **A clean-machine test**: a Mac that never had Vibar, DMG to Applications, first launch, both permission prompts, connect, press keys.

## Release checklist

1. `scripts/ci.sh` passes on a clean clone.
2. Version bumped in `Cargo.toml` (the only place) and an entry added to `CHANGELOG.md`.
3. Items above resolved, or listed honestly in the release notes.
4. `scripts/bundle.sh`, then sign with the Developer ID, notarize, staple.
5. `DEVELOPER_ID="…" scripts/dmg.sh`, then check it: `spctl --assess --type open --context context:primary-signature -v build/Vibar-<version>-<arch>.dmg`.
6. Install it on a second Mac that has never run Vibar.
7. `git tag v<version>`, push the tag, then create the GitHub release with the DMG, its SHA-256 and the notes.

## Quirks that affect DMG users

- Run from the disk image or from Downloads, the app lives at a temporary path. **Launch at login refuses to switch on** until the app is in `/Applications` or `~/Applications`, and says so, because the login item would point at a path that disappears.
- The audio driver is installed from the copy inside the app, so the microphone works from any location, but moving the app afterwards does not move an installed driver. Uninstall from Settings → Microphone first if you want a clean state.
