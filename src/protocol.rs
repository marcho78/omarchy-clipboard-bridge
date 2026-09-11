//! Wire protocol: newline-delimited JSON frames over one TCP connection.
//!
//! Handshake (host is the server):
//!   host  -> guest : Challenge { nonce }
//!   guest -> host  : Auth { id, mac = HMAC-SHA256(secret, nonce) }   (already paired)
//!                or  Pair { name }                                    (first contact)
//!   host  -> guest : PairCode { code }  -> user approves on the host -> Paired { id, secret }
//!   host  -> guest : Ok
//! Then either side sends Clip frames whenever its clipboard changes.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const VERSION: u32 = 2;
pub const DEFAULT_PORT: u16 = 52017;
pub const MAX_FRAME: usize = 24 * 1024 * 1024;
pub const SERVICE_TYPE: &str = "_clipbridge._tcp.local.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Frame {
    Challenge { v: u32, name: String, nonce: String },
    Auth { id: String, name: String, mac: String },
    Pair { name: String },
    PairCode { code: String },
    Paired { id: String, secret: String },
    PairDenied { reason: String },
    Ok { name: String },
    Err { error: String },
    Clip { mime: String, b64: String },
    Ping,
    Pong,
}

pub fn hmac_hex(secret_hex: &str, nonce: &str) -> String {
    let key = hex::decode(secret_hex).unwrap_or_default();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("hmac accepts any key length");
    mac.update(nonce.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    hex::encode(buf)
}

pub fn pairing_code() -> String {
    format!("{:04}", rand::random::<u32>() % 10_000)
}

/// Encode one frame as a line.
pub fn encode(frame: &Frame) -> Vec<u8> {
    let mut v = serde_json::to_vec(frame).expect("frame serializes");
    v.push(b'\n');
    v
}
