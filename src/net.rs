//! Line-framed JSON over TCP with a shared writer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

use crate::protocol::{encode, Frame, MAX_FRAME};

#[derive(Clone)]
pub struct Writer(Arc<Mutex<TcpStream>>);

impl Writer {
    pub fn new(stream: TcpStream) -> Self {
        Self(Arc::new(Mutex::new(stream)))
    }
    pub fn send(&self, frame: &Frame) -> Result<()> {
        let bytes = encode(frame);
        let mut s = self.0.lock().unwrap();
        s.write_all(&bytes)?;
        Ok(())
    }
    pub fn shutdown(&self) {
        if let Ok(s) = self.0.lock() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

pub struct Reader(BufReader<TcpStream>);

impl Reader {
    pub fn new(stream: TcpStream) -> Self {
        Self(BufReader::with_capacity(64 * 1024, stream))
    }
    /// Read one frame. Returns Ok(None) on clean EOF.
    pub fn next(&mut self) -> Result<Option<Frame>> {
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = (&mut self.0).take(MAX_FRAME as u64 + 1).read_until(b'\n', &mut line)?;
            if n == 0 {
                return Ok(None);
            }
            if line.len() > MAX_FRAME {
                bail!("frame exceeds {MAX_FRAME} bytes");
            }
            match serde_json::from_slice::<Frame>(&line) {
                Ok(f) => return Ok(Some(f)),
                Err(_) => continue, // tolerate unknown frames from newer peers
            }
        }
    }
}
