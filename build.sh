#!/bin/bash
# Compile le moteur Rust + l'interface Swift et assemble build/ThermoPilot.app
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
APP="$ROOT/build/ThermoPilot.app"
VERSION="${VERSION:-$(cat VERSION)}"
BUILD_NUMBER="${BUILD_NUMBER:-$(git rev-list --count HEAD 2>/dev/null || echo 1)}"

echo "▶ Moteur Rust"
(cd engine && cargo build --release)

echo "▶ Interface Swift"
(cd app && swift build -c release)
SWIFT_BIN="$(cd app && swift build -c release --show-bin-path)/ThermoPilot"

echo "▶ Bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$SWIFT_BIN" "$APP/Contents/MacOS/ThermoPilot"
cp engine/target/release/fanengine "$APP/Contents/MacOS/fanengine"
cp scripts/install-helper.sh scripts/uninstall-helper.sh "$APP/Contents/Resources/"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
chmod +x "$APP/Contents/Resources/"*.sh

cat > "$APP/Contents/Info.plist" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>ThermoPilot</string>
  <key>CFBundleDisplayName</key><string>ThermoPilot</string>
  <key>CFBundleIdentifier</key><string>com.lilian.thermopilot</string>
  <key>CFBundleExecutable</key><string>ThermoPilot</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIconName</key><string>AppIcon</string>
  <key>NSHumanReadableCopyright</key><string>ThermoPilot</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSUIElement</key><true/>
  <key>NSAppleEventsUsageDescription</key><string>Nécessaire pour installer le helper root qui pilote les ventilateurs.</string>
</dict>
</plist>
PL

# Signature : ad hoc par défaut, ou Developer ID si SIGN_IDENTITY est défini (requis pour la notarisation)
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
if [ "$SIGN_IDENTITY" = "-" ]; then
  codesign --force --sign - "$APP/Contents/MacOS/fanengine"
  codesign --force --sign - "$APP"
else
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP/Contents/MacOS/fanengine"
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
fi
echo "✅ $APP"
