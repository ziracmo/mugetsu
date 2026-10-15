#!/usr/bin/env bash
# Builds target/release/Mugetsu.app (unsigned). Usage: scripts/bundle-macos.sh
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p mugetsu-app
APP=target/release/Mugetsu.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/mugetsu-app "$APP/Contents/MacOS/Mugetsu"
VERSION=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="mugetsu-app"))')
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Mugetsu</string>
  <key>CFBundleDisplayName</key><string>Mugetsu</string>
  <key>CFBundleIdentifier</key><string>dev.mugetsu</string>
  <key>CFBundleExecutable</key><string>Mugetsu</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature so Gatekeeper lets a locally built app run.
codesign --force --deep --sign - "$APP" 2>/dev/null || true
echo "Built $APP — drag it to /Applications"
