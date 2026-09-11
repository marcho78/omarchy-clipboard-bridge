#!/bin/bash
# Remove the omarchy-clipboard-bridge host agent from this Mac.
set -euo pipefail
LABEL="com.omarchy.clipboard-bridge"
launchctl bootout "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
rm -rf "$HOME/.local/share/clipboard-bridge"
if [[ "${1:-}" == "--purge" ]]; then
  rm -rf "$HOME/.config/clipboard-bridge" "$HOME/Library/Logs/clipboard-bridge.log"
fi
echo "clipboard-bridge host agent removed."
