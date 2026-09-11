//! Host mode: listen for guests, pair them, mirror the clipboard.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};

use crate::clip::{Clip, Echo};
use crate::clipboard;
use crate::config::{now, HostConfig, PairedGuest, State};
use crate::net::{Reader, Writer};
use crate::pairing;
use crate::protocol::{hmac_hex, pairing_code, random_hex, Frame, VERSION};
use crate::util::{local_name, log};

const PING: Duration = Duration::from_secs(15);

struct Peer {
    name: String,
    writer: Writer,
}

struct Shared {
    cfg: Mutex<HostConfig>,
    peers: Mutex<Vec<Arc<Peer>>>,
    echo: Mutex<Echo>,
    name: String,
}

impl Shared {
    fn broadcast(&self, clip: &Clip, except: Option<&Arc<Peer>>) {
        let frame = Frame::Clip { mime: clip.mime.clone(), b64: B64.encode(&clip.data) };
        let peers = self.peers.lock().unwrap().clone();
        for p in peers {
            if except.map_or(true, |e| !Arc::ptr_eq(e, &p)) {
                if let Err(e) = p.writer.send(&frame) {
                    log(format!("send to {} failed: {e}", p.name));
                    p.writer.shutdown();
                }
            }
        }
    }

    fn write_state(&self) {
        let peers = self.peers.lock().unwrap();
        let names: Vec<String> = peers.iter().map(|p| p.name.clone()).collect();
        State {
            role: "host".into(),
            connected: !names.is_empty(),
            peer: names.join(", "),
            detail: format!("{} guest(s) connected", names.len()),
            updated: now(),
        }
        .write();
    }
}

pub fn run(mut cfg: HostConfig, auto_accept: bool) -> Result<()> {
    cfg.auto_accept |= auto_accept;
    let name = local_name();
    let addr: SocketAddr = format!("{}:{}", cfg.bind, cfg.port).parse().context("invalid bind address")?;
    let listener = TcpListener::bind(addr).with_context(|| format!("listening on {addr}"))?;
    log(format!("listening on {addr} as \"{name}\""));

    let _mdns = match crate::discovery::advertise(&name, cfg.port) {
        Ok(d) => Some(d),
        Err(e) => {
            log(format!("mDNS advertisement failed ({e}); guests will use the gateway fallback"));
            None
        }
    };

    let shared = Arc::new(Shared {
        cfg: Mutex::new(cfg),
        peers: Mutex::new(Vec::new()),
        echo: Mutex::new(Echo::default()),
        name,
    });
    shared.write_state();

    // Local clipboard -> guests
    {
        let shared = shared.clone();
        let (tx, rx) = crossbeam_channel::unbounded::<Clip>();
        thread::spawn(move || {
            if let Err(e) = clipboard::watch(tx) {
                log(format!("clipboard watcher stopped: {e:#}"));
                std::process::exit(1);
            }
        });
        thread::spawn(move || {
            for clip in rx {
                if shared.echo.lock().unwrap().note(&clip) {
                    log(format!("local change: {}", clip.describe()));
                    shared.broadcast(&clip, None);
                }
            }
        });
    }

    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let shared = shared.clone();
                thread::spawn(move || {
                    if let Err(e) = handle(stream, shared) {
                        log(format!("connection ended: {e:#}"));
                    }
                });
            }
            Err(e) => log(format!("accept failed: {e}")),
        }
    }
    Ok(())
}

fn handle(stream: TcpStream, shared: Arc<Shared>) -> Result<()> {
    let addr = stream.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let writer = Writer::new(stream.try_clone()?);
    let mut reader = Reader::new(stream.try_clone()?);

    let nonce = random_hex(16);
    writer.send(&Frame::Challenge { v: VERSION, name: shared.name.clone(), nonce: nonce.clone() })?;

    let guest_name = match reader.next()? {
        Some(Frame::Auth { id, name, mac }) => {
            let ok = {
                let cfg = shared.cfg.lock().unwrap();
                cfg.guests.iter().any(|g| g.id == id && hmac_hex(&g.secret, &nonce) == mac)
            };
            if !ok {
                writer.send(&Frame::Err { error: "unknown guest; pair again".into() })?;
                log(format!("rejected {name} ({addr}): unknown id or bad mac"));
                return Ok(());
            }
            name
        }
        Some(Frame::Pair { name }) => {
            let code = pairing_code();
            writer.send(&Frame::PairCode { code: code.clone() })?;
            log(format!("pairing request from {name} ({addr}), code {code}"));
            let auto = shared.cfg.lock().unwrap().auto_accept;
            // Approval may take a while; allow the guest to wait.
            stream.set_read_timeout(Some(Duration::from_secs(120)))?;
            if !pairing::approve(&name, &addr, &code, auto) {
                writer.send(&Frame::PairDenied { reason: "denied on host".into() })?;
                log(format!("pairing with {name} denied"));
                return Ok(());
            }
            let guest = PairedGuest { id: random_hex(8), name: name.clone(), secret: random_hex(32), paired_at: now() };
            {
                let mut cfg = shared.cfg.lock().unwrap();
                cfg.guests.retain(|g| g.name != name);
                cfg.guests.push(guest.clone());
                cfg.save()?;
            }
            writer.send(&Frame::Paired { id: guest.id, secret: guest.secret })?;
            log(format!("paired with {name}"));
            name
        }
        Some(other) => {
            log(format!("unexpected frame during handshake from {addr}: {other:?}"));
            return Ok(());
        }
        None => return Ok(()),
    };

    writer.send(&Frame::Ok { name: shared.name.clone() })?;
    stream.set_read_timeout(Some(PING * 3))?;
    let peer = Arc::new(Peer { name: format!("{guest_name} ({addr})"), writer: writer.clone() });
    shared.peers.lock().unwrap().push(peer.clone());
    shared.write_state();
    log(format!("guest connected: {}", peer.name));

    {
        let w = writer.clone();
        let shared = shared.clone();
        let peer = peer.clone();
        thread::spawn(move || {
            while shared.peers.lock().unwrap().iter().any(|p| Arc::ptr_eq(p, &peer)) {
                thread::sleep(PING);
                if w.send(&Frame::Ping).is_err() {
                    break;
                }
            }
        });
    }

    let result = (|| -> Result<()> {
        while let Some(frame) = reader.next()? {
            match frame {
                Frame::Clip { mime, b64 } => {
                    let clip = Clip { mime, data: B64.decode(b64)? };
                    if clip.data.is_empty() {
                        continue;
                    }
                    if shared.echo.lock().unwrap().note(&clip) {
                        log(format!("from {}: {}", peer.name, clip.describe()));
                        if let Err(e) = clipboard::set(&clip) {
                            log(format!("setting clipboard failed: {e:#}"));
                        }
                        shared.broadcast(&clip, Some(&peer));
                    }
                }
                Frame::Ping => writer.send(&Frame::Pong)?,
                _ => {}
            }
        }
        Ok(())
    })();

    shared.peers.lock().unwrap().retain(|p| !Arc::ptr_eq(p, &peer));
    shared.write_state();
    writer.shutdown();
    log(format!("guest disconnected: {}", peer.name));
    result
}
