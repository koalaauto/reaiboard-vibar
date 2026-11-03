#!/bin/bash
# Package build/Vibar.app into a disk image: build/Vibar-<version>-<arch>[-adhoc].dmg
#
#   scripts/dmg.sh                build the app first if needed, then the dmg, keep its signature
#   scripts/dmg.sh --adhoc        internal build: re-sign the copy inside the dmg ad-hoc and add
#                                 install notes. Your own build/Vibar.app is left untouched.
#   DEVELOPER_ID="Developer ID Application: Name (TEAM)" scripts/dmg.sh
#                                 also sign the dmg itself
#
# This script only packages. Whether other people can open the result without a warning depends
# on how the app was signed and whether it was notarized, see docs/releasing.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$ROOT/build/Vibar.app"

ADHOC=0
for arg in "$@"; do
  case "$arg" in
    --adhoc) ADHOC=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

[ -d "$APP" ] || "$ROOT/scripts/bundle.sh"

# Name the file after what is really inside.
ARCHS="$(lipo -archs "$APP/Contents/MacOS/vibar")"
case "$ARCHS" in
  *arm64*x86_64* | *x86_64*arm64*) ARCH="universal" ;;
  *) ARCH="${ARCHS// /-}" ;;
esac
SUFFIX=""
[ "$ADHOC" = 1 ] && SUFFIX="-adhoc"
OUT="$ROOT/build/Vibar-$VERSION-$ARCH$SUFFIX.dmg"

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE:?}"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"   # so the user can drag the app onto it

if [ "$ADHOC" = 1 ]; then
  # An ad-hoc signature does not depend on anyone's certificate, so it opens the same on every
  # Mac. The price: it is a hash of this exact build, so permissions are asked again after each
  # update. The audio driver inside Resources keeps its own signature on purpose.
  codesign --force --sign - "$STAGE/Vibar.app"
  codesign --verify --deep --strict "$STAGE/Vibar.app"
  sed "s/@VERSION@/$VERSION/g" "$ROOT/scripts/dmg-readme.txt" > "$STAGE/Read me first.txt"
fi

rm -f "$OUT"
hdiutil create -volname "Vibar $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$OUT" >/dev/null

if [ -n "${DEVELOPER_ID:-}" ]; then
  codesign --force --sign "$DEVELOPER_ID" "$OUT"
fi

echo "built: $OUT"
shasum -a 256 "$OUT"
