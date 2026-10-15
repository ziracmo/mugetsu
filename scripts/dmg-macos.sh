#!/usr/bin/env bash
# Packs target/release/Mugetsu.app (see bundle-macos.sh) into a drag-to-install disk image.
# Usage: scripts/dmg-macos.sh <output.dmg>   (signed too when MACOS_SIGN_IDENTITY is set)
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=${1:-target/release/Mugetsu.dmg}
STAGE=target/release/dmg
rm -rf "$STAGE" "$OUT"
mkdir -p "$STAGE" "$(dirname "$OUT")"
cp -R target/release/Mugetsu.app "$STAGE/"
ln -s /Applications "$STAGE/Applications"
# hdiutil's own size estimate for -srcfolder can come up short ("No space left on device"):
# give the scratch image the app's size plus a margin.
SIZE_MB=$(($(du -sm "$STAGE" | cut -f1) * 3 / 2 + 20))
hdiutil create -volname Mugetsu -srcfolder "$STAGE" -size "${SIZE_MB}m" -ov -format UDZO "$OUT"
if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
  codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$OUT"
fi
echo "Built $OUT"
