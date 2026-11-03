#!/bin/bash
# Package build/Vibar.app into a disk image: build/Vibar-<version>-<arch>.dmg
#
#   scripts/dmg.sh                       build the app first if needed, then the dmg
#   DEVELOPER_ID="Developer ID Application: Name (TEAM)" scripts/dmg.sh
#                                        also sign the dmg itself
#
# This script only packages. Whether other people can open the result without a warning depends
# on how the app was signed and whether it was notarized, see docs/releasing.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$ROOT/build/Vibar.app"

[ -d "$APP" ] || "$ROOT/scripts/bundle.sh"

# Name the file after what is really inside.
ARCHS="$(lipo -archs "$APP/Contents/MacOS/vibar")"
case "$ARCHS" in
  *arm64*x86_64* | *x86_64*arm64*) ARCH="universal" ;;
  *) ARCH="${ARCHS// /-}" ;;
esac
OUT="$ROOT/build/Vibar-$VERSION-$ARCH.dmg"

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE:?}"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"   # so the user can drag the app onto it

rm -f "$OUT"
hdiutil create -volname "Vibar $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$OUT" >/dev/null

if [ -n "${DEVELOPER_ID:-}" ]; then
  codesign --force --sign "$DEVELOPER_ID" "$OUT"
fi

echo "built: $OUT"
shasum -a 256 "$OUT"
