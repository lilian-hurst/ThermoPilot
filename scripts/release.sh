#!/bin/bash
# Construit l'app et prépare les fichiers à publier dans dist/ :
#   ThermoPilot-<version>.dmg  (glisser dans Applications)
#   ThermoPilot-<version>.zip
#   SHA256SUMS.txt
# Variables optionnelles :
#   VERSION=1.2.0                      (sinon le fichier VERSION)
#   SIGN_IDENTITY="Developer ID Application: Nom (TEAMID)"   signature Apple
#   NOTARY_PROFILE=thermopilot         profil créé avec `xcrun notarytool store-credentials`
set -euo pipefail
cd "$(dirname "$0")/.."
export VERSION="${VERSION:-$(cat VERSION)}"
./build.sh
APP=build/ThermoPilot.app
DIST=dist
rm -rf "$DIST" && mkdir -p "$DIST"

# DMG : l'app + un raccourci vers /Applications
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
DMG="$DIST/ThermoPilot-$VERSION.dmg"
hdiutil create -volname "ThermoPilot $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
if [ "${SIGN_IDENTITY:--}" != "-" ]; then
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"
fi

# Notarisation Apple (supprime l'avertissement Gatekeeper chez les utilisateurs)
if [ -n "${NOTARY_PROFILE:-}" ]; then
  echo "▶ Notarisation (quelques minutes)…"
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler staple "$APP"
fi

# ZIP (ditto conserve signatures et attributs)
ditto -c -k --sequesterRsrc --keepParent "$APP" "$DIST/ThermoPilot-$VERSION.zip"
(cd "$DIST" && shasum -a 256 ThermoPilot-* > SHA256SUMS.txt)
echo "✅ Fichiers prêts :"
ls -lh "$DIST"
