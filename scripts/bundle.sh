#!/bin/bash
# Build and package the menu bar app: build/Vibar.app
#   scripts/bundle.sh          build, sign, done
#   scripts/bundle.sh --run    ... and launch it
#
# Signing: $SIGN_ID if set, else the first valid code-signing identity in the keychain, else
# ad-hoc. An ad-hoc signature changes on every build, which makes macOS forget the Bluetooth and
# Accessibility permissions each time. See docs/signing-and-permissions.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
REPO="$(sed -n 's/^repository = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$ROOT/build/Vibar.app"
RES="$APP/Contents/Resources"
BUNDLE_ID="dev.koala.vibar"

(cd "$ROOT" && cargo build --release)

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$RES/en.lproj" "$RES/zh-Hans.lproj"
cp "$ROOT/assets/AppIcon.icns" "$RES/AppIcon.icns"
cp "$ROOT/target/release/vibar" "$APP/Contents/MacOS/vibar"

# The virtual microphone driver is compiled by the SDK's build script and shipped inside the app.
# It is only installed into the system when the user turns the microphone on.
# Do not re-sign it: the ad-hoc signature from the SDK build is the tested way for it to load.
DRIVER="$(ls -td "$ROOT"/target/release/build/reai-board-sdk-*/out/driver-build/ReAIVibeBoard.driver 2>/dev/null | head -1)"
[ -n "$DRIVER" ] || { echo "virtual microphone driver not found (needs cmake)" >&2; exit 1; }
cp -R "$DRIVER" "$RES/"

# Info.plist carries the English text. The zh-Hans strings below override it for Chinese users.
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>Vibar</string>
  <key>CFBundleDisplayName</key><string>Vibar</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleExecutable</key><string>vibar</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>zh-Hans</string></array>
  <key>NSHumanReadableCopyright</key><string>An unofficial client for the ReAI-Vibe-Board keyboard, not affiliated with ReAI.</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSBluetoothAlwaysUsageDescription</key><string>Vibar uses Bluetooth to connect to the ReAI-Vibe-Board keyboard and receive its key and knob events.</string>
  <key>NSBluetoothPeripheralUsageDescription</key><string>Vibar uses Bluetooth to connect to the ReAI-Vibe-Board keyboard.</string>
</dict></plist>
EOF

cat > "$RES/en.lproj/InfoPlist.strings" <<'EOF'
"NSHumanReadableCopyright" = "An unofficial client for the ReAI-Vibe-Board keyboard, not affiliated with ReAI.";
"NSBluetoothAlwaysUsageDescription" = "Vibar uses Bluetooth to connect to the ReAI-Vibe-Board keyboard and receive its key and knob events.";
"NSBluetoothPeripheralUsageDescription" = "Vibar uses Bluetooth to connect to the ReAI-Vibe-Board keyboard.";
EOF
cat > "$RES/zh-Hans.lproj/InfoPlist.strings" <<'EOF'
"NSHumanReadableCopyright" = "非官方的 ReAI-Vibe-Board 键盘客户端,与 ReAI 公司没有隶属关系。";
"NSBluetoothAlwaysUsageDescription" = "Vibar 通过蓝牙连接 ReAI-Vibe-Board 键盘,接收按键和旋钮事件。";
"NSBluetoothPeripheralUsageDescription" = "Vibar 通过蓝牙连接 ReAI-Vibe-Board 键盘。";
EOF

# The standard About panel shows Credits.html from the matching .lproj folder.
credits() {  # $1 = text
  printf '<html><body style="font-family: -apple-system; font-size: 11px; text-align: center;">\n<p>%s</p>\n<p><a href="%s">%s</a></p>\n</body></html>\n' "$1" "$REPO" "$REPO"
}
credits "An unofficial menu bar client for the ReAI-Vibe-Board keyboard." > "$RES/en.lproj/Credits.html"
credits "非官方的 ReAI-Vibe-Board 键盘菜单栏客户端。" > "$RES/zh-Hans.lproj/Credits.html"

SIGN="${SIGN_ID:-$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(.*\)".*/\1/p' | head -1)}"
codesign --force --sign "${SIGN:--}" "$APP" >/dev/null 2>&1 || codesign --force --sign - "$APP" >/dev/null 2>&1
echo "built: $APP  (signed with: ${SIGN:-ad-hoc})"
[ -n "$SIGN" ] || echo "note: ad-hoc signature, permissions will be asked again after every build" >&2

if [ "${1:-}" = "--run" ]; then
  pkill -f "Vibar.app/Contents/MacOS/vibar" 2>/dev/null || true
  sleep 1
  open "$APP"
  echo "launched"
fi
