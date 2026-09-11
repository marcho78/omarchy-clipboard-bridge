#!/bin/bash
# omarchy-clipboard-bridge — macOS host installer.
#
# Run on the Mac that hosts the Omarchy VM. Two ways:
#   1. From the VM:   omarchy-clipboard-bridge host-setup   (prints a curl one-liner)
#   2. From a clone:  ./host/install.sh --token TOKEN [--port PORT]
#
set -euo pipefail

BASE="${CLIPBOARD_BRIDGE_BASE:-__BASE__}"   # substituted by `host-setup`; unset when run from a clone
TOKEN="${CLIPBOARD_BRIDGE_TOKEN:-}"
PORT="${CLIPBOARD_BRIDGE_PORT:-52017}"
LABEL="com.omarchy.clipboard-bridge"
SHARE_DIR="$HOME/.local/share/clipboard-bridge"
CONFIG_DIR="$HOME/.config/clipboard-bridge"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --token) TOKEN="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This installer is for the macOS host. In the Omarchy VM run: omarchy-clipboard-bridge install" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1 || ! python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 8) else 1)' 2>/dev/null; then
  cat >&2 <<MSG
python3 is required. Install the Xcode Command Line Tools and re-run:
    xcode-select --install
MSG
  exit 1
fi
PYTHON="$(command -v python3)"

fetch() {  # fetch <name> <dest>
  if [[ "$BASE" == "__BASE__" ]]; then
    cp "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$1" "$2"
  else
    curl -fsSL "$BASE/$1" -o "$2"
  fi
}

mkdir -p "$SHARE_DIR" "$CONFIG_DIR" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
fetch clipboard-bridge-host.py "$SHARE_DIR/clipboard-bridge-host.py"
fetch com.omarchy.clipboard-bridge.plist "$SHARE_DIR/$LABEL.plist.template"
fetch uninstall.sh "$SHARE_DIR/uninstall.sh"
chmod +x "$SHARE_DIR/clipboard-bridge-host.py" "$SHARE_DIR/uninstall.sh"

# Config: served token wins over an existing one; otherwise keep what is there.
if [[ -z "$TOKEN" && "$BASE" != "__BASE__" ]]; then
  TOKEN="$(curl -fsSL "$BASE/token")"
fi
if [[ -z "$TOKEN" && -f "$CONFIG_DIR/config" ]]; then
  TOKEN="$(sed -n 's/^TOKEN=//p' "$CONFIG_DIR/config" | tr -d '"'"'")"
fi
if [[ -z "$TOKEN" ]]; then
  echo "No token. Pass --token (shown by 'omarchy-clipboard-bridge token' in the VM)." >&2
  exit 1
fi
umask 077
cat > "$CONFIG_DIR/config" <<CFG
# omarchy-clipboard-bridge host config
# BIND: address to listen on. 0.0.0.0 works for Shared and Bridged networking.
BIND=0.0.0.0
PORT=$PORT
TOKEN=$TOKEN
CFG
umask 022

sed -e "s|__PYTHON__|$PYTHON|g" -e "s|__HOME__|$HOME|g" \
  "$SHARE_DIR/$LABEL.plist.template" > "$PLIST"

launchctl bootout "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
launchctl kickstart -k "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true

sleep 1
if launchctl print "gui/$(id -u)/$LABEL" 2>/dev/null | grep -q 'state = running'; then
  echo "clipboard-bridge host agent is running on port $PORT."
else
  echo "Agent installed but not reported as running yet. Check: tail -f ~/Library/Logs/clipboard-bridge.log" >&2
fi
cat <<MSG

If macOS asks whether "python3" may accept incoming network connections, click Allow.
Log:       ~/Library/Logs/clipboard-bridge.log
Config:    $CONFIG_DIR/config
Uninstall: $SHARE_DIR/uninstall.sh --purge
MSG
