//! Guest mode: find the host, pair on first contact, then mirror the clipboard.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use crossbeam_channel::{select, unbounded, Receiver};

use crate::clip::{Clip, Echo};
use crate::clipboard;
use crate::config::{now, GuestConfig, State};
use crate::discovery;
use crate::net::{Reader, Writer};
use crate::protocol::{hmac_hex, Frame};
use crate::util::{local_name, log};

const PING: Duration = Duration::from_secs(15);

pub struct Options {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Exit once paired and connected (used by `pair`).
    pub once: bool,
}

pub fn run(mut cfg: GuestConfig, opts: Options) -> Result<()> {
    if let Some(h) = &opts.host {
        cfg.host = h.clone();
    }
    if let Some(p) = opts.port {
        cfg.port = p;
    }
    let cfg = Arc::new(Mutex::new(cfg));
    let echo = Arc::new(Mutex::new(Echo::default()));

    let (tx, clips) = unbounded::<Clip>();
    thread::spawn(move || {
        if let Err(e) = clipboard::watch(tx) {
            log(format!("clipboard watcher stopped: {e:#}"));
            std::process::exit(1);
        }
    });

    let mut backoff = 1;
    loop {
        set_state(false, "", "connecting");
        match session(&cfg, &clips, &echo, opts.once) {
            Ok(true) => return Ok(()),
            Ok(false) => backoff = 1,
            Err(e) => {
                log(format!("{e:#}"));
                if opts.once {
                    return Err(e);
                }
                if e.to_string().contains("denied") {
                    backoff = 60;
                }
            }
        }
        set_state(false, "", &format!("reconnecting in {backoff}s"));
        thread::sleep(Duration::from_secs(backoff));
        backoff = (backoff * 2).min(15);
    }
}

fn set_state(connected: bool, peer: &str, detail: &str) {
    State { role: "guest".into(), connected, peer: peer.into(), detail: detail.into(), updated: now() }.write();
}

fn resolve(cfg: &GuestConfig) -> Result<(SocketAddr, String)> {
    if !cfg.host.is_empty() {
        let ip: IpAddr = cfg.host.parse().context("HOST must be an IP address")?;
        return Ok((SocketAddr::new(ip, cfg.port), "config".into()));
    }
    if let Some((ip, port, name)) = discovery::browse(Duration::from_millis(1500)) {
        return Ok((SocketAddr::new(ip, port), format!("mDNS {name}")));
    }
    if let Some(ip) = discovery::gateway_guess() {
        return Ok((SocketAddr::new(ip, cfg.port), "gateway fallback".into()));
    }
    bail!("could not determine host address; set host = \"...\" in guest.toml")
}

/// Returns Ok(true) if `once` was requested and the session reached the
/// connected state; Ok(false) after a normal disconnect.
fn session(cfg: &Arc<Mutex<GuestConfig>>, clips: &Receiver<Clip>, echo: &Arc<Mutex<Echo>>, once: bool) -> Result<bool> {
    let snapshot = cfg.lock().unwrap().clone();
    let (addr, how) = resolve(&snapshot)?;
    log(format!("connecting to {addr} ({how})"));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).with_context(|| format!("connecting to {addr}"))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let writer = Writer::new(stream.try_clone()?);
    let mut reader = Reader::new(stream.try_clone()?);

    let (host_name, nonce) = match reader.next()? {
        Some(Frame::Challenge { v, name, nonce }) => {
            if v != crate::protocol::VERSION {
                bail!("host speaks protocol v{v}, this build speaks v{}", crate::protocol::VERSION);
            }
            (name, nonce)
        }
        other => bail!("unexpected handshake from host: {other:?}"),
    };

    let my_name = local_name();
    if snapshot.paired() {
        writer.send(&Frame::Auth { id: snapshot.id.clone(), name: my_name.clone(), mac: hmac_hex(&snapshot.secret, &nonce) })?;
    } else {
        pair(&writer, &mut reader, &stream, cfg, &my_name, &host_name)?;
    }

    match reader.next()? {
        Some(Frame::Ok { .. }) => {}
        Some(Frame::Err { error }) if error.contains("unknown guest") => {
            log("host no longer knows this guest; pairing again");
            let mut c = cfg.lock().unwrap();
            c.id.clear();
            c.secret.clear();
            c.save()?;
            return Ok(false);
        }
        Some(Frame::Err { error }) => bail!("host refused: {error}"),
        other => bail!("unexpected reply: {other:?}"),
    }
    stream.set_read_timeout(Some(PING * 3))?;
    set_state(true, &host_name, &format!("connected to {addr}"));
    log(format!("connected to \"{host_name}\" at {addr}"));
    if once {
        log("paired and connected");
        writer.shutdown();
        return Ok(true);
    }

    // Reader thread -> channel so we can select over it together with clipboard changes.
    let (net_tx, net_rx) = unbounded::<Result<Frame>>();
    thread::spawn(move || loop {
        match reader.next() {
            Ok(Some(f)) => {
                if net_tx.send(Ok(f)).is_err() {
                    break;
                }
            }
            Ok(None) => {
                let _ = net_tx.send(Err(anyhow!("host closed the connection")));
                break;
            }
            Err(e) => {
                let _ = net_tx.send(Err(e));
                break;
            }
        }
    });

    let ticker = crossbeam_channel::tick(PING);
    let result = loop {
        select! {
            recv(clips) -> clip => {
                let clip = match clip { Ok(c) => c, Err(_) => break Err(anyhow!("clipboard watcher gone")) };
                if echo.lock().unwrap().note(&clip) {
                    log(format!("local change: {}", clip.describe()));
                    if let Err(e) = writer.send(&Frame::Clip { mime: clip.mime.clone(), b64: B64.encode(&clip.data) }) {
                        break Err(e);
                    }
                }
            }
            recv(net_rx) -> msg => {
                match msg {
                    Ok(Ok(Frame::Clip { mime, b64 })) => {
                        let clip = Clip { mime, data: B64.decode(b64)? };
                        if !clip.data.is_empty() && echo.lock().unwrap().note(&clip) {
                            log(format!("from host: {}", clip.describe()));
                            if let Err(e) = clipboard::set(&clip) {
                                log(format!("setting clipboard failed: {e:#}"));
                            }
                        }
                    }
                    Ok(Ok(Frame::Ping)) => { if let Err(e) = writer.send(&Frame::Pong) { break Err(e); } }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => break Err(e),
                    Err(_) => break Err(anyhow!("reader thread gone")),
                }
            }
            recv(ticker) -> _ => {
                if let Err(e) = writer.send(&Frame::Ping) { break Err(e); }
            }
        }
    };
    writer.shutdown();
    set_state(false, "", "disconnected");
    log("disconnected from host");
    result.map(|_: ()| false)
}

fn pair(
    writer: &Writer,
    reader: &mut Reader,
    stream: &TcpStream,
    cfg: &Arc<Mutex<GuestConfig>>,
    my_name: &str,
    host_name: &str,
) -> Result<()> {
    writer.send(&Frame::Pair { name: my_name.to_string() })?;
    match reader.next()? {
        Some(Frame::PairCode { code }) => {
            log(format!("Pairing code: {code}  —  click Allow in the dialog on \"{host_name}\""));
        }
        other => bail!("expected pairing code, got {other:?}"),
    }
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    match reader.next()? {
        Some(Frame::Paired { id, secret }) => {
            let mut c = cfg.lock().unwrap();
            c.id = id;
            c.secret = secret;
            c.host_name = host_name.to_string();
            c.save()?;
            log(format!("paired with \"{host_name}\""));
            Ok(())
        }
        Some(Frame::PairDenied { reason }) => bail!("pairing denied: {reason}"),
        other => bail!("unexpected reply while pairing: {other:?}"),
    }
}
