# omarchy-clipboard-bridge

Shared clipboard between an [Omarchy](https://omarchy.org) VM and its macOS host
under Parallels Desktop.

Parallels Tools' clipboard sharing only works with X11. Omarchy runs Hyprland on
Wayland, so copy/paste between the Mac and the VM silently does nothing. This
tool fixes that with a small daemon on each side talking over the Parallels
shared network. Text clipboard, both directions, no dependencies beyond what
Omarchy and macOS already ship.

## How it works

```
 macOS host                                Omarchy VM (Hyprland)
 ┌──────────────────────────┐              ┌──────────────────────────┐
 │ clipboard-bridge-host.py │  TCP :52017  │ clipboard-bridge-guest.py│
 │  launchd agent           │◄────────────►│  systemd --user service  │
 │  pbpaste / pbcopy        │  token auth  │  wl-paste / wl-copy      │
 └──────────────────────────┘              └──────────────────────────┘
```

* The host listens; the guest connects and reconnects automatically. Only the
  host needs a stable address, and on Parallels shared networking it always has
  one (`10.211.55.2`), which the guest auto-detects from its default gateway.
* A random token generated at install time authenticates the guest.
* Each side remembers the last clipboard content it applied, so a change
  arriving from the peer is not echoed back.
* Omarchy's own clipboard history (`wl-paste --watch cliphist`) keeps working;
  synced content shows up there too.

## Install

### 1. In the Omarchy VM

```bash
git clone https://github.com/marcho78/omarchy-clipboard-bridge
cd omarchy-clipboard-bridge
./install.sh
```

This installs `omarchy-clipboard-bridge` into `~/.local/bin`, generates a
config with a fresh token in `~/.config/clipboard-bridge/config`, and enables a
`systemd --user` service that starts with your Hyprland session.

### 2. On the Mac

Still inside the VM, run:

```bash
omarchy-clipboard-bridge host-setup
```

It prints a `curl ... | bash` one-liner and serves the macOS installer (with
the token baked in) from the VM for ten minutes. Paste that line into Terminal
on the Mac. The installer needs `python3`, which comes with the Xcode Command
Line Tools (`xcode-select --install`).

If macOS asks whether `python3` may accept incoming connections, click Allow.

Alternatively, from a clone of this repo on the Mac:

```bash
./host/install.sh --token "$(the token shown by 'omarchy-clipboard-bridge token' in the VM)"
```

### 3. Check

```bash
omarchy-clipboard-bridge status
```

`link: connected` means both sides are talking. Copy something on either side
and paste on the other.

## Commands (VM)

| Command | Purpose |
|---|---|
| `install` | Install / upgrade the guest daemon |
| `host-setup` | Serve the macOS installer from the VM |
| `status` | Service, link and recent log lines |
| `logs` | Follow the daemon log |
| `start` / `stop` / `restart` | Control the service |
| `token` | Print the shared secret |
| `config` | Edit the config, then `restart` |
| `uninstall [--purge]` | Remove the daemon (and config with `--purge`) |

## Configuration

Guest `~/.config/clipboard-bridge/config`:

```
HOST=            # empty = auto-detect (gateway .1 -> host .2). Set for Bridged networking.
PORT=52017
TOKEN=...
```

Host `~/.config/clipboard-bridge/config`:

```
BIND=0.0.0.0     # or the Parallels vnic address to keep it off your LAN
PORT=52017
TOKEN=...
```

## Troubleshooting

* `link: not connected` in the VM: check the host log with
  `tail -f ~/Library/Logs/clipboard-bridge.log` on the Mac. If nothing is
  listening, run `launchctl kickstart -k gui/$(id -u)/com.omarchy.clipboard-bridge`.
* Using Bridged networking instead of Shared: set `HOST=` in the guest config to
  the Mac's LAN address.
* Paste in the VM gives nothing after copying on the Mac: the content was not
  text. Images and files are not synced yet.

## Uninstall

VM: `omarchy-clipboard-bridge uninstall --purge`
Mac: `~/.local/share/clipboard-bridge/uninstall.sh --purge`

## Limitations

* Text only. Images (`image/png`) are a natural next step; the protocol already
  carries a `mime` field.
* The host agent polls `pbpaste` a few times per second. It is cheap but not
  event-driven, because macOS ships no CLI hook for pasteboard changes.
* Traffic is unencrypted. It stays on the virtual network between the Mac and
  the VM and is authenticated with the token, but do not point it across a real
  LAN.

## License

MIT

## Author

[@devsec_ai](https://x.com/devsec_ai) on X
