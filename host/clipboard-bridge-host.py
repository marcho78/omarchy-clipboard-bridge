#!/usr/bin/env python3
"""
omarchy-clipboard-bridge — macOS host agent.

Listens for guest VMs on the Parallels shared network and keeps the macOS
clipboard in sync with them. Text only. No third-party dependencies; uses the
pbpaste / pbcopy tools that ship with macOS.

Runs as a launchd LaunchAgent in the user's GUI session so it can see the
pasteboard.
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
POLL_INTERVAL = 0.4
PING_INTERVAL = 15
HANDSHAKE_TIMEOUT = 5

PB_ENV = dict(os.environ, LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8")


def log(msg):
    print(time.strftime("%H:%M:%S"), msg, flush=True)


def load_config():
    cfg = {"BIND": "0.0.0.0", "PORT": "52017", "TOKEN": ""}
    try:
        with open(CONFIG_PATH) as fh:
            for line in fh:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, val = line.split("=", 1)
                cfg[key.strip()] = val.strip().strip('"').strip("'")
    except FileNotFoundError:
        log(f"config not found at {CONFIG_PATH}")
        sys.exit(1)
    if not cfg["TOKEN"]:
        log("TOKEN missing from config")
        sys.exit(1)
    return cfg


def pbpaste():
    try:
        return subprocess.run(
            ["pbpaste", "-Prefer", "txt"], capture_output=True, env=PB_ENV, timeout=5
        ).stdout
    except Exception as exc:
        log(f"pbpaste failed: {exc}")
        return None


def pbcopy(data):
    try:
        subprocess.run(["pbcopy"], input=data, env=PB_ENV, timeout=5, check=True)
    except Exception as exc:
        log(f"pbcopy failed: {exc}")


class Client:
    def __init__(self, sock, addr):
        self.sock = sock
        self.addr = addr
        self.name = f"{addr[0]}:{addr[1]}"
        self.lock = threading.Lock()

    def send(self, obj):
        payload = (json.dumps(obj, separators=(",", ":")) + "\n").encode()
        with self.lock:
            try:
                self.sock.sendall(payload)
            except OSError:
                self.close()

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


class Server:
    def __init__(self, cfg):
        self.cfg = cfg
        self.clients = set()
        self.clients_lock = threading.Lock()
        self.state_lock = threading.Lock()
        self.last_content = None

    # ---------- local clipboard ----------

    def poll_local(self):
        self.last_content = pbpaste()
        while True:
            time.sleep(POLL_INTERVAL)
            data = pbpaste()
            if data is None or not data or len(data) > MAX_FRAME_BYTES:
                continue
            with self.state_lock:
                if data == self.last_content:
                    continue
                self.last_content = data
            self.broadcast(
                {"t": "clip", "mime": "text/plain", "b64": base64.b64encode(data).decode()},
                exclude=None,
            )

    def apply_local(self, data, origin):
        with self.state_lock:
            if data == self.last_content:
                return
            self.last_content = data
        pbcopy(data)
        # Fan out to any other connected guests as well.
        self.broadcast(
            {"t": "clip", "mime": "text/plain", "b64": base64.b64encode(data).decode()},
            exclude=origin,
        )

    # ---------- network ----------

    def broadcast(self, obj, exclude):
        with self.clients_lock:
            targets = [c for c in self.clients if c is not exclude]
        for client in targets:
            client.send(obj)

    def serve(self):
        bind, port = self.cfg["BIND"], int(self.cfg["PORT"])
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind((bind, port))
        srv.listen(8)
        log(f"listening on {bind}:{port}")
        while True:
            sock, addr = srv.accept()
            threading.Thread(target=self.handle, args=(sock, addr), daemon=True).start()

    def handle(self, sock, addr):
        client = Client(sock, addr)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        sock.settimeout(HANDSHAKE_TIMEOUT)
        reader = sock.makefile("rb")
        try:
            first = reader.readline(64 * 1024)
            msg = json.loads(first) if first else {}
        except ValueError:
            msg = {}
        if msg.get("t") != "hello" or msg.get("token") != self.cfg["TOKEN"]:
            log(f"rejected {client.name}: bad handshake")
            client.send({"t": "hello_err", "error": "unauthorized"})
            client.close()
            return
        if msg.get("v") != PROTOCOL_VERSION:
            client.send({"t": "hello_err", "error": f"protocol {msg.get('v')} unsupported"})
            client.close()
            return
        client.name = f"{msg.get('name', 'guest')} ({addr[0]})"
        client.send({"t": "hello_ok", "v": PROTOCOL_VERSION, "name": socket.gethostname()})
        sock.settimeout(PING_INTERVAL * 3)
        with self.clients_lock:
            self.clients.add(client)
        log(f"guest connected: {client.name}")
        threading.Thread(target=self.ping_loop, args=(client,), daemon=True).start()
        try:
            while True:
                line = reader.readline(MAX_FRAME_BYTES + 1024)
                if not line:
                    break
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
                        self.apply_local(data, origin=client)
                elif kind == "ping":
                    client.send({"t": "pong"})
        except OSError as exc:
            log(f"{client.name}: {exc}")
        finally:
            with self.clients_lock:
                self.clients.discard(client)
            client.close()
            log(f"guest disconnected: {client.name}")

    def ping_loop(self, client):
        while True:
            time.sleep(PING_INTERVAL)
            with self.clients_lock:
                if client not in self.clients:
                    return
            client.send({"t": "ping"})


def main():
    cfg = load_config()
    server = Server(cfg)
    threading.Thread(target=server.poll_local, daemon=True).start()
    try:
        server.serve()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
