#!/bin/bash
# Arrête le helper (il rend les ventilateurs à macOS en s'arrêtant) et le supprime.
set -uo pipefail
LABEL="com.lilian.fanengine"
launchctl bootout "system/$LABEL" 2>/dev/null
sleep 1
/Library/PrivilegedHelperTools/$LABEL auto 2>/dev/null || true
rm -f "/Library/LaunchDaemons/$LABEL.plist" "/Library/PrivilegedHelperTools/$LABEL" /var/run/fanengine.sock
echo "Helper désinstallé."
