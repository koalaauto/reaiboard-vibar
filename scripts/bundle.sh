#!/bin/bash
# 构建并打包菜单栏 App:build/Vibar.app
# 用法: scripts/bundle.sh [--run]
#   --run  打包后直接启动
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$ROOT/build/Vibar.app"
BUNDLE_ID="dev.koala.vibar"

(cd "$ROOT" && cargo build --release)

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp "$ROOT/target/release/vibar" "$APP/Contents/MacOS/vibar"

# 虚拟麦克风驱动由 SDK 的构建脚本编译,随 App 一起带上(用户启用麦克风时才会安装到系统)。
# 不要对它重新签名:SDK 产出的 ad-hoc 签名是经过验证的加载方式。
DRIVER="$(ls -td "$ROOT"/target/release/build/reai-board-sdk-*/out/driver-build/ReAIVibeBoard.driver 2>/dev/null | head -1)"
[ -n "$DRIVER" ] || { echo "找不到虚拟麦克风驱动产物(需要 cmake)" >&2; exit 1; }
cp -R "$DRIVER" "$APP/Contents/Resources/"

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
  <key>NSHumanReadableCopyright</key><string>非官方的 ReAI Vibe Board 菜单栏客户端,与 ReAI 公司无隶属关系。</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSBluetoothAlwaysUsageDescription</key><string>通过蓝牙连接 ReAI Vibe Board 键盘,接收按键和旋钮事件。</string>
  <key>NSBluetoothPeripheralUsageDescription</key><string>通过蓝牙连接 ReAI Vibe Board 键盘。</string>
</dict></plist>
EOF

# Standard About panel shows Credits.html from Resources.
REPO="$(sed -n 's/^repository = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
cat > "$APP/Contents/Resources/Credits.html" <<HTML
<html><body style="font-family: -apple-system; font-size: 11px; text-align: center;">
<p>非官方的 ReAI-Vibe-Board 菜单栏客户端。</p>
<p><a href="$REPO">$REPO</a></p>
</body></html>
HTML

SIGN="${SIGN_ID:-$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(.*\)".*/\1/p' | head -1)}"
codesign --force --sign "${SIGN:--}" "$APP" >/dev/null 2>&1 || codesign --force --sign - "$APP" >/dev/null 2>&1
echo "built: $APP  (signed with: ${SIGN:-ad-hoc})"

if [ "${1:-}" = "--run" ]; then
  pkill -f "Vibar.app/Contents/MacOS/vibar" 2>/dev/null || true
  sleep 1
  open "$APP"
  echo "launched"
fi
