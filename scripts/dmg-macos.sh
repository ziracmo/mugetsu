#!/usr/bin/env bash
# Packs target/release/Mugetsu.app (see bundle-macos.sh) into a drag-to-install disk image with
# the eclipse background and icon layout from packaging/macos (dmgbuild writes the Finder layout
# without driving Finder, so it works headless in CI).
# Usage: scripts/dmg-macos.sh <output.dmg>
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=${1:-target/release/Mugetsu.dmg}
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"

if ! command -v dmgbuild >/dev/null; then
  python3 -m venv target/dmgbuild-venv
  target/dmgbuild-venv/bin/pip install -q 'dmgbuild==1.6.7'
  PATH="$PWD/target/dmgbuild-venv/bin:$PATH"
fi
dmgbuild -s packaging/macos/dmg-settings.py -D app=target/release/Mugetsu.app Mugetsu "$OUT"
echo "Built $OUT"
