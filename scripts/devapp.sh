#!/bin/bash
# 把一个命令行二进制包成带蓝牙/输入监控权限说明的 .app 并运行。
# macOS 的 TCC 权限按"责任进程"计算,裸二进制从别的 App 里启动会被系统直接 abort;
# 用 `open` 启动独立 .app,权限才会记在这个 .app 头上。
#
# 用法: scripts/devapp.sh <二进制路径> [app名] [参数...]
#   输出写到 build/<app名>.out / .err
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
  <key>NSBluetoothAlwaysUsageDescription</key><string>通过蓝牙连接 ReAI Vibe Board 键盘。</string>
  <key>NSBluetoothPeripheralUsageDescription</key><string>通过蓝牙连接 ReAI Vibe Board 键盘。</string>
  <key>NSInputMonitoringUsageDescription</key><string>读取键盘按键并映射成快捷键。</string>
</dict></plist>
EOF

# 稳定签名:有开发者证书就用它(重编译后 TCC 授权不丢),否则 ad-hoc。可用 SIGN_ID 覆盖。
SIGN="${SIGN_ID:-$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(.*\)".*/\1/p' | head -1)}"
codesign --force --sign "${SIGN:--}" "$APP" >/dev/null 2>&1 || codesign --force --sign - "$APP" >/dev/null 2>&1
echo "signed with: ${SIGN:-ad-hoc}"

OUT="$ROOT/build/$NAME.out"; ERR="$ROOT/build/$NAME.err"
: > "$OUT"; : > "$ERR"
echo "launching $APP (stdout -> $OUT)"
open -n -W --stdout "$OUT" --stderr "$ERR" "$APP" ${@:+--args "$@"}
echo "--- stdout ---"; cat "$OUT"
echo "--- stderr ---"; cat "$ERR"
