#!/bin/bash
# Wrap a command line binary into a throwaway .app with Bluetooth usage text, and run it.
# macOS attributes a permission request to the "responsible process". A bare binary started from
# another app (a terminal, an IDE) is killed on its first Bluetooth use. Starting a separate .app
# with `open` makes the permission belong to that .app.
#
# Side effect: the throwaway app appears in System Settings → Privacy & Security with a made-up
# name and no icon. For looking at the UI, copy the real Vibar.app instead, see AGENTS.md.
#
# Usage: scripts/devapp.sh <binary> [app-name] [args...]
#   stdout and stderr go to build/<app-name>.out and .err
set -euo pipefail

BIN="${1:?usage: devapp.sh <binary> [name] [args...]}"
NAME="${2:-$(basename "$BIN")}"
shift 2 || shift $#
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/build/$NAME.app"
EXE="reai-dev"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/$EXE"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.koala.vibar-dev.$NAME</string>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleExecutable</key><string>$EXE</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSUIElement</key><true/>
  <key>NSBluetoothAlwaysUsageDescription</key><string>Connects to the ReAI-Vibe-Board keyboard over Bluetooth.</string>
  <key>NSBluetoothPeripheralUsageDescription</key><string>Connects to the ReAI-Vibe-Board keyboard over Bluetooth.</string>
  <key>NSInputMonitoringUsageDescription</key><string>Reads keyboard keys and maps them to shortcuts.</string>
</dict></plist>
EOF

# Stable signing: use a developer certificate if there is one so the permissions survive rebuilds,
# else ad-hoc. Override with SIGN_ID.
SIGN="${SIGN_ID:-$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(.*\)".*/\1/p' | head -1)}"
codesign --force --sign "${SIGN:--}" "$APP" >/dev/null 2>&1 || codesign --force --sign - "$APP" >/dev/null 2>&1
echo "signed with: ${SIGN:-ad-hoc}"

OUT="$ROOT/build/$NAME.out"; ERR="$ROOT/build/$NAME.err"
: > "$OUT"; : > "$ERR"
echo "launching $APP (stdout -> $OUT)"
open -n -W --stdout "$OUT" --stderr "$ERR" "$APP" ${@:+--args "$@"}
echo "--- stdout ---"; cat "$OUT"
echo "--- stderr ---"; cat "$ERR"
