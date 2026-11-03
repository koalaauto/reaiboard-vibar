# Signing and permissions

## Why you may have to grant permissions again

macOS stores a permission (Bluetooth, Accessibility) against an app's **bundle id and code signature**. If the signature changes, the system treats the app as a new one and the permission is gone, so the toggle in System Settings has to be switched on again.

| Signing | Effect on permissions |
|---|---|
| A stable identity (Apple Development, Developer ID, or your own certificate) | Kept across rebuilds and updates, as long as the bundle id and certificate stay the same |
| Ad-hoc (`codesign --sign -`) | The signature is a hash of the binary, so it changes on every build. **Permissions are lost on every rebuild.** |

Changing the bundle id (`dev.koala.vibar`, set in `scripts/bundle.sh`) also resets permissions once.

## What `scripts/bundle.sh` does

1. Builds the release binary and assembles `build/Vibar.app`.
2. Copies the virtual microphone driver (built by the SDK) into `Contents/Resources`. **It is not re-signed**: the SDK's ad-hoc signature is the tested way for it to load.
3. Signs the app with `$SIGN_ID` if set, otherwise with the first valid code-signing identity in your keychain, otherwise ad-hoc.

List your identities with `security find-identity -v -p codesigning`.

## Getting a stable identity for local work

- **You already have an Apple Development certificate** (from Xcode with a free or paid Apple ID): it is picked up automatically.
- **You have nothing**: create a local one. Keychain Access → Certificate Assistant → Create a Certificate, name it `Vibar Dev`, Identity Type *Self Signed Root*, Certificate Type *Code Signing*. Open the certificate, expand Trust, set Code Signing to *Always Trust*. Then:

  ```bash
  SIGN_ID="Vibar Dev" scripts/bundle.sh --run
  ```

## Resetting permissions

```bash
tccutil reset Accessibility dev.koala.vibar
tccutil reset Bluetooth dev.koala.vibar
```

Then start Vibar again and grant them when asked. The Accessibility prompt only opens System Settings; you switch the toggle yourself. Vibar cannot and should not do it for you.

## Why a bare binary dies on Bluetooth

If you run `target/release/vibar` from a terminal or an IDE, macOS attributes the Bluetooth request to *that* program, which has no `NSBluetoothAlwaysUsageDescription`, and aborts the process. Always test through the `.app` bundle. `scripts/devapp.sh` wraps any binary into a throwaway `.app` for this purpose.

## Releasing to other people (not done yet)

Anyone downloading a build needs it to be signed with a **Developer ID Application** certificate and **notarized**, otherwise Gatekeeper blocks it. This requires a paid Apple Developer Program membership. Outline:

1. Sign with the hardened runtime: `codesign --force --options runtime --sign "Developer ID Application: …" build/Vibar.app`.
2. Zip and submit: `xcrun notarytool submit Vibar.zip --keychain-profile … --wait`.
3. Staple: `xcrun stapler staple build/Vibar.app`.

This outline has not been run for Vibar. Treat it as a starting point and expect to adjust entitlements. The bundled audio driver must keep working after signing, so test the microphone on a notarized build before publishing it.
