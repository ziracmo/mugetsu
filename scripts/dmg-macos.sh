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
hdiutil create -volname Mugetsu -srcfolder "$STAGE" -ov -format UDZO "$OUT"
if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
  codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$OUT"
fi
echo "Built $OUT"
