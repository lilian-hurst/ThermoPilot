#!/bin/bash
# Régénère l'icône : assets/icon-1024.png puis assets/AppIcon.icns (toutes les tailles macOS)
set -euo pipefail
cd "$(dirname "$0")/.."
swift scripts/make-icon.swift assets/icon-1024.png
SET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon-1024.png --out "$SET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon-1024.png --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o assets/AppIcon.icns
rm -rf "$(dirname "$SET")"
echo "✅ assets/AppIcon.icns"
