#!/usr/bin/env python3
"""
omarchy-clipboard-bridge — guest daemon.

Keeps the Wayland clipboard of an Omarchy (Hyprland) VM in sync with the
macOS host it runs on, over the Parallels shared network. Parallels Tools'
own clipboard sharing only works under X11, so this replaces it for Wayland.

Guest side is the TCP *client*: it connects to the host agent (the host has a
stable address on the Parallels shared network, the guest usually does not),
pushes local clipboard changes up, and applies host clipboard changes locally.

No third-party dependencies. Needs wl-clipboard (wl-paste / wl-copy).
"""

import base64
import json
import os
import socket
import subprocess
import sys
import threading
import time

PROTOCOL_VERSION = 1
CONFIG_PATH = os.environ.get(
    "CLIPBOARD_BRIDGE_CONFIG", os.path.expanduser("~/.config/clipboard-bridge/config")
)
MAX_FRAME_BYTES = 16 * 1024 * 1024
PING_INTERVAL = 15
RECONNECT_MIN = 1
RECONNECT_MAX = 15


def log(msg):
    print(msg, flush=True)


def load_config():
    cfg = {"HOST": "", "PORT": "52017", "TOKEN": ""}
    try:
        with open(CONFIG_PATH) as fh:
            for line in fh:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, val = line.split("=", 1)
                cfg[key.strip()] = val.strip().strip('"').strip("'")
    except FileNotFoundError:
        log(f"config not found at {CONFIG_PATH}; run: omarchy-clipboard-bridge install")
        sys.exit(1)
    if not cfg["TOKEN"]:
        log("TOKEN missing from config")
        sys.exit(1)
    if not cfg["HOST"]:
        cfg["HOST"] = detect_host()
        log(f"HOST not set, auto-detected macOS host at {cfg['HOST']}")
    return cfg


def detect_host():
    """Parallels shared networking: gateway is x.x.x.1, the macOS host is x.x.x.2."""
    try:
        out = subprocess.run(
            ["ip", "-4", "route", "show", "default"], capture_output=True, text=True, check=True
        ).stdout.split()
        gateway = out[out.index("via") + 1]
        parts = gateway.split(".")
        parts[-1] = "2"
        return ".".join(parts)
    except Exception:
        return "10.211.55.2"


class Bridge:
    def __init__(self, cfg):
        self.cfg = cfg
        self.sock = None
        self.send_lock = threading.Lock()
        self.state_lock = threading.Lock()
        self.last_content = None  # last bytes we set or saw locally, to break echo loops
        self.connected = threading.Event()

    # ---------- local clipboard ----------

    def watch_local(self):
        """Run `wl-paste --watch`; each change arrives as one base64 line."""
        cmd = ["wl-paste", "-n", "-t", "text", "--watch", "sh", "-c", "base64 -w0; echo"]
        while True:
            try:
                proc = subprocess.Popen(
                    cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True
                )
            except FileNotFoundError:
                log("wl-paste not found; install wl-clipboard")
                sys.exit(1)
            log("watching local clipboard")
            for line in proc.stdout:
                line = line.strip()
                if not line:
                    continue
                try:
                    data = base64.b64decode(line)
                except Exception:
                    continue
                self.on_local_change(data)
            proc.wait()
            log(f"wl-paste exited ({proc.returncode}); restarting in 3s")
            time.sleep(3)

    def on_local_change(self, data):
        if not data or len(data) > MAX_FRAME_BYTES:
            return
        with self.state_lock:
            if data == self.last_content:
                return  # this is the change we just applied from the host
            self.last_content = data
        if self.connected.is_set():
            self.send({"t": "clip", "mime": "text/plain", "b64": base64.b64encode(data).decode()})

    def apply_local(self, data):
        with self.state_lock:
            if data == self.last_content:
                return
            self.last_content = data
        try:
            subprocess.run(["wl-copy"], input=data, check=True, timeout=5)
        except Exception as exc:
            log(f"wl-copy failed: {exc}")

    # ---------- network ----------

    def send(self, obj):
        payload = (json.dumps(obj, separators=(",", ":")) + "\n").encode()
        with self.send_lock:
            sock = self.sock
            if sock is None:
                return
            try:
                sock.sendall(payload)
            except OSError as exc:
                log(f"send failed: {exc}")
                try:
                    sock.close()
                except OSError:
                    pass

    def run_network(self):
        delay = RECONNECT_MIN
        while True:
            try:
                self.session()
                delay = RECONNECT_MIN
            except (OSError, ValueError) as exc:
                log(f"connection error: {exc}")
            finally:
                self.connected.clear()
                self.sock = None
            log(f"reconnecting in {delay}s")
            time.sleep(delay)
            delay = min(delay * 2, RECONNECT_MAX)

    def session(self):
        host, port = self.cfg["HOST"], int(self.cfg["PORT"])
        log(f"connecting to {host}:{port}")
        sock = socket.create_connection((host, port), timeout=10)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock = sock
        self.send(
            {
                "t": "hello",
                "v": PROTOCOL_VERSION,
                "token": self.cfg["TOKEN"],
                "name": socket.gethostname(),
            }
        )
        reader = sock.makefile("rb")
        first = reader.readline()
        if not first:
            raise OSError("host closed connection during handshake")
        msg = json.loads(first)
        if msg.get("t") != "hello_ok":
            raise OSError(f"handshake rejected: {msg.get('error', msg)}")
        sock.settimeout(PING_INTERVAL * 3)
        self.connected.set()
        log(f"connected to host {msg.get('name', host)}")

        pinger = threading.Thread(target=self.ping_loop, args=(sock,), daemon=True)
        pinger.start()

        while True:
            line = reader.readline(MAX_FRAME_BYTES + 1024)
            if not line:
                raise OSError("host disconnected")
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            kind = msg.get("t")
            if kind == "clip":
                try:
                    data = base64.b64decode(msg.get("b64", ""))
                except Exception:
                    continue
                if data:
                    self.apply_local(data)
            elif kind == "ping":
                self.send({"t": "pong"})

    def ping_loop(self, sock):
        while self.sock is sock and self.connected.is_set():
            time.sleep(PING_INTERVAL)
            if self.sock is sock:
                self.send({"t": "ping"})


def main():
    cfg = load_config()
    if not os.environ.get("WAYLAND_DISPLAY"):
        log("WAYLAND_DISPLAY is not set; this must run inside the Wayland session")
        sys.exit(1)
    bridge = Bridge(cfg)
    threading.Thread(target=bridge.watch_local, daemon=True).start()
    try:
        bridge.run_network()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
