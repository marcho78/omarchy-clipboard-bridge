#!/bin/bash
# Installer. In the Omarchy VM it installs the guest service; on macOS it hands
# off to host/install.sh so either URL works. Downloads the latest release
# binary (or builds from this clone if cargo is available and --build is given),
# then installs and starts the systemd user service.
set -euo pipefail

REPO="marcho78/omarchy-clipboard-bridge"
BIN_DIR="${CLIPBOARD_BRIDGE_BIN_DIR:-$HOME/.local/bin}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "macOS detected: running the host installer instead."
  exec bash -c "$(curl -fsSL "https://raw.githubusercontent.com/$REPO/main/host/install.sh")" -- "$@"
fi
command -v wl-copy >/dev/null || { echo "wl-clipboard is required: sudo pacman -S wl-clipboard" >&2; exit 1; }
mkdir -p "$BIN_DIR"

if [[ "${1:-}" == "--build" ]]; then
  command -v cargo >/dev/null || { echo "cargo not found; install rustup or drop --build" >&2; exit 1; }
  (cd "$HERE" && cargo build --release --locked)
  install -m755 "$HERE/target/release/clipboard-bridge" "$BIN_DIR/clipboard-bridge"
else
  case "$(uname -m)" in
    aarch64|arm64) ASSET=clipboard-bridge-linux-aarch64 ;;
    x86_64) ASSET=clipboard-bridge-linux-x86_64 ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
  esac
  URL="https://github.com/$REPO/releases/latest/download/$ASSET"
  echo "downloading $URL"
  curl -fsSL --retry 3 "$URL" -o "$BIN_DIR/clipboard-bridge.tmp"
  chmod +x "$BIN_DIR/clipboard-bridge.tmp"
  mv -f "$BIN_DIR/clipboard-bridge.tmp" "$BIN_DIR/clipboard-bridge"
fi

"$BIN_DIR/clipboard-bridge" install
[[ ":$PATH:" == *":$BIN_DIR:"* ]] || echo "Note: add $BIN_DIR to your PATH to use 'clipboard-bridge status' directly."
