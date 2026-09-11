# omarchy-clipboard-bridge

Shared clipboard between an [Omarchy](https://omarchy.org) VM and its macOS host
under Parallels Desktop. Text and images, both directions, no typing of tokens.

Parallels Tools' clipboard sharing is built on X11. Omarchy runs Hyprland on
Wayland, so copy and paste between the Mac and the VM silently does nothing.
This tool fixes that with one small binary that runs on both sides and talks
over the Parallels virtual network.

## Install

### 1. In the Omarchy VM

```bash
curl -fsSL https://raw.githubusercontent.com/marcho78/omarchy-clipboard-bridge/main/install.sh | bash
```

or from the AUR (`yay -S omarchy-clipboard-bridge` then `clipboard-bridge install`),
or from a clone with `./install.sh --build` if you have cargo.

This puts `clipboard-bridge` in `~/.local/bin` and enables a `systemd --user`
service that starts with your Hyprland session.

### 2. On the Mac

Open Terminal and run:

```bash
curl -fsSL https://raw.githubusercontent.com/marcho78/omarchy-clipboard-bridge/main/host/install.sh | bash
```

This downloads the binary and installs a launchd agent that starts at login.
If macOS asks whether `clipboard-bridge` may accept incoming connections,
click Allow.

### 3. Pair

Within a few seconds the VM finds the Mac and a dialog appears on the Mac:

> omarchy-vm (10.211.55.26) wants to share your clipboard. Pairing code: 4821. Allow?

The VM shows the same code in its log. Click **Allow**. That is the whole
setup. Copy on either side, paste on the other.

Check with `clipboard-bridge status` on either machine. `link: connected`
means both sides are talking.

## How it works

```
 macOS host                                   Omarchy VM (Hyprland)
 ┌────────────────────────────┐               ┌────────────────────────────┐
 │ clipboard-bridge serve     │  mDNS + TCP   │ clipboard-bridge connect   │
 │  launchd agent             │◄─────────────►│  systemd --user service    │
 │  NSPasteboard              │  :52017       │  wlr-data-control          │
 └────────────────────────────┘               └────────────────────────────┘
```

* **Discovery.** The Mac advertises `_clipbridge._tcp` with mDNS. The VM
  browses for it and, if multicast is blocked, falls back to the Parallels
  shared-network convention (gateway `.1`, host `.2`). You can also set
  `host = "..."` in the guest config for bridged networking.
* **Pairing.** On first contact the host shows a dialog with a four-digit code
  and the guest prints the same code. Approval generates a 256-bit secret that
  both sides store with mode 600. Later connections authenticate with an
  HMAC-SHA256 challenge-response, so the secret never crosses the wire again.
* **Clipboard.** On Wayland the guest uses the wlr-data-control protocol, so
  it is told about every new selection instead of polling. On macOS the agent
  polls the pasteboard change counter in-process, which is what every native
  tool does since there is no notification API. Text is sent as-is; images
  travel as PNG.
* **No echo.** Each side remembers a hash of the last clip it applied, so a
  change that arrived from the peer is not sent back.
* Omarchy's own clipboard history keeps working and shows synced content too.

## Commands

Same binary on both sides. The role follows the OS.

| Command | Purpose |
|---|---|
| `install [--auto-accept]` | Install and start the user service (`--auto-accept` is macOS only) |
| `uninstall [--purge]` | Stop and remove the service; `--purge` also deletes config and secrets |
| `status` | Service, pairing and link state |
| `logs` | Follow the daemon log |
| `pair [--host IP]` | Guest: forget the current pairing and pair again now |
| `forget` | Drop all pairings on this side |
| `serve` / `connect` | Run the host or guest daemon in the foreground |

## Configuration

Guest, `~/.config/clipboard-bridge/guest.toml`:

```toml
host = ""        # empty = mDNS, then gateway .2; set an IP for bridged networking
port = 52017
```

Host, `~/.config/clipboard-bridge/host.toml`:

```toml
bind = "0.0.0.0" # or the Parallels vnic address to keep it off your LAN
port = 52017
auto_accept = false
```

Pairing ids and secrets are stored in the same files.

## Troubleshooting

* **No pairing dialog on the Mac.** Check `clipboard-bridge logs` on the Mac.
  If the guest never connects, the firewall may have blocked the binary: System
  Settings, Network, Firewall, Options, and allow `clipboard-bridge`.
* **`link: not connected` in the VM.** Run `clipboard-bridge logs` in the VM.
  "connection refused" means the host agent is not running on the Mac.
* **Bridged networking.** mDNS usually still finds the host. If not, set
  `host = "<Mac LAN IP>"` in `guest.toml` and `systemctl --user restart clipboard-bridge`.
* **Pairing denied by accident.** Run `clipboard-bridge pair` in the VM to try again.

## Uninstall

VM: `clipboard-bridge uninstall --purge`. Mac: `clipboard-bridge uninstall --purge`,
then delete `~/.local/bin/clipboard-bridge`.

## Building

```bash
cargo build --release
cargo test
```

Releases are built by GitHub Actions for macOS arm64 and x86_64 and Linux
aarch64 and x86_64. The Linux build has no C dependencies; `wl-copy` from
wl-clipboard is used at runtime to serve the selection.

## Security notes

Traffic is unencrypted but authenticated. It is meant for the virtual link
between a Mac and its own VMs. Binding to `0.0.0.0` exposes the port to your
LAN, where a stranger could only trigger a pairing dialog that you can deny.
Set `bind` to the Parallels vnic address if you prefer.

## License

MIT

## Author

[@devsec_ai](https://x.com/devsec_ai) on X
