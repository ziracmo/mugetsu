#!/usr/bin/env bash
# Builds Mugetsu.app (ad-hoc signed) and installs it in /Applications.
# Usage: scripts/bundle-macos.sh              → build + install in /Applications
#        scripts/bundle-macos.sh --no-install → build only (target/release/Mugetsu.app)
#        scripts/bundle-macos.sh --universal  → Apple silicon + Intel binary (implies --no-install)
set -euo pipefail
cd "$(dirname "$0")/.."

INSTALL=1
UNIVERSAL=0
for arg in "$@"; do
  case "$arg" in
    --no-install) INSTALL=0 ;;
    --universal) UNIVERSAL=1 INSTALL=0 ;;
  esac
done

APP=target/release/Mugetsu.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [[ $UNIVERSAL == 1 ]]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked -p mugetsu-app --target "$t"
  done
  lipo -create -output "$APP/Contents/MacOS/Mugetsu" \
    target/aarch64-apple-darwin/release/Mugetsu target/x86_64-apple-darwin/release/Mugetsu
else
  cargo build --release -p mugetsu-app
  cp target/release/Mugetsu "$APP/Contents/MacOS/Mugetsu"
fi

# App icon: every size macOS wants, from the 1024 px master.
ICONSET=target/release/Mugetsu.iconset
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
SRC=crates/mugetsu-app/assets/icon/mugetsu.png
for size in 16 32 128 256 512; do
  sips -z $size $size "$SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Mugetsu.icns"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/mugetsu-app/Cargo.toml | head -1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Mugetsu</string>
  <key>CFBundleDisplayName</key><string>Mugetsu</string>
  <key>CFBundleIdentifier</key><string>dev.mugetsu</string>
  <key>CFBundleExecutable</key><string>Mugetsu</string>
  <key>CFBundleIconFile</key><string>Mugetsu</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature so Gatekeeper lets a locally built app run.
codesign --force --deep --sign - "$APP"

if [[ $INSTALL == 1 ]]; then
  rm -rf /Applications/Mugetsu.app
  cp -R "$APP" /Applications/
  # Refresh the Dock / Launchpad icon cache for the new bundle.
  touch /Applications/Mugetsu.app
  echo "Installé : /Applications/Mugetsu.app (Launchpad, Spotlight, Dock)"
else
  echo "Built $APP"
fi
