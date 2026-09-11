#!/bin/bash
# omarchy-clipboard-bridge — macOS host installer.
# Downloads the latest clipboard-bridge binary and installs it as a launchd agent.
#
#   curl -fsSL https://raw.githubusercontent.com/marcho78/omarchy-clipboard-bridge/main/host/install.sh | bash
#
# Options (as arguments or env):  --auto-accept   accept pairing without a dialog
set -euo pipefail

REPO="marcho78/omarchy-clipboard-bridge"
BIN_DIR="${CLIPBOARD_BRIDGE_BIN_DIR:-$HOME/.local/bin}"
AUTO_ACCEPT=""
for a in "$@"; do
  case "$a" in
    --auto-accept) AUTO_ACCEPT="--auto-accept" ;;
    -h|--help) sed -n '2,7p' "$0"; exit 0 ;;
    *) echo "unknown argument: $a" >&2; exit 1 ;;
  esac
done

[[ "$(uname -s)" == "Darwin" ]] || { echo "This installer is for the macOS host. In the Omarchy VM use: clipboard-bridge install" >&2; exit 1; }
case "$(uname -m)" in
  arm64) ASSET=clipboard-bridge-macos-arm64 ;;
  x86_64) ASSET=clipboard-bridge-macos-x86_64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

URL="https://github.com/$REPO/releases/latest/download/$ASSET"
mkdir -p "$BIN_DIR"
echo "downloading $URL"
curl -fsSL --retry 3 "$URL" -o "$BIN_DIR/clipboard-bridge.tmp"
chmod +x "$BIN_DIR/clipboard-bridge.tmp"
mv -f "$BIN_DIR/clipboard-bridge.tmp" "$BIN_DIR/clipboard-bridge"
xattr -d com.apple.quarantine "$BIN_DIR/clipboard-bridge" 2>/dev/null || true
# Ad-hoc signature gives the binary a stable identity so the firewall asks once, not on every start.
codesign --force -s - "$BIN_DIR/clipboard-bridge" 2>/dev/null || true

"$BIN_DIR/clipboard-bridge" install $AUTO_ACCEPT
[[ ":$PATH:" == *":$BIN_DIR:"* ]] || echo "Note: add $BIN_DIR to your PATH to use 'clipboard-bridge status' directly."
