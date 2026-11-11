#!/bin/bash
# Installe le moteur Rust comme LaunchDaemon root (contrôle des ventilateurs).
# Usage (en root) : install-helper.sh <chemin/vers/fanengine> <uid autorisé>
set -euo pipefail
SRC="$1"
ALLOW_UID="$2"
LABEL="com.lilian.fanengine"
BIN="/Library/PrivilegedHelperTools/$LABEL"
PLIST="/Library/LaunchDaemons/$LABEL.plist"

[[ "$(id -u)" == "0" ]] || { echo "Doit être lancé en root"; exit 1; }
[[ -x "$SRC" ]] || { echo "Binaire introuvable : $SRC"; exit 1; }
[[ "$ALLOW_UID" =~ ^[0-9]+$ ]] || { echo "UID invalide"; exit 1; }

launchctl bootout "system/$LABEL" 2>/dev/null || true
# bootout est asynchrone : attendre que l'ancienne instance soit déchargée
for _ in $(seq 1 50); do
  launchctl print "system/$LABEL" >/dev/null 2>&1 || break
  sleep 0.2
done

install -d -m 755 -o root -g wheel /Library/PrivilegedHelperTools
install -m 755 -o root -g wheel "$SRC" "$BIN"

cat > "$PLIST" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
    <string>serve</string>
    <string>--socket</string><string>/var/run/fanengine.sock</string>
    <string>--allow-uid</string><string>$ALLOW_UID</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardErrorPath</key><string>/var/log/fanengine.log</string>
</dict>
</plist>
PL
chown root:wheel "$PLIST"
chmod 644 "$PLIST"
for attempt in 1 2 3 4 5; do
  if launchctl bootstrap system "$PLIST" 2>/dev/null; then break; fi
  if launchctl print "system/$LABEL" >/dev/null 2>&1; then
    launchctl kickstart -k "system/$LABEL"   # déjà chargé : redémarre avec le nouveau binaire
    break
  fi
  [[ $attempt == 5 ]] && { echo "launchctl bootstrap a échoué"; exit 1; }
  sleep 1
done
echo "Helper installé."
