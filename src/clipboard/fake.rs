//! File-backed clipboard for tests. The file holds the raw bytes; a sibling
//! `<file>.mime` names the type (defaults to text/plain).

use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::Sender;

use crate::clip::{Clip, MIME_TEXT};

fn read(path: &str) -> Option<Clip> {
    let data = std::fs::read(path).ok()?;
    if data.is_empty() {
        return None;
    }
    let mime = std::fs::read_to_string(format!("{path}.mime")).map(|s| s.trim().to_string()).unwrap_or_else(|_| MIME_TEXT.into());
    Some(Clip { mime, data })
}

pub fn watch(path: &str, tx: Sender<Clip>) -> Result<()> {
    let mut last = read(path);
    loop {
        sleep(Duration::from_millis(100));
        let now = read(path);
        if now != last {
            last = now.clone();
            if let Some(c) = now {
                let _ = tx.send(c);
            }
        }
    }
}

pub fn set(path: &str, clip: &Clip) -> Result<()> {
    std::fs::write(format!("{path}.mime"), &clip.mime)?;
    std::fs::write(path, &clip.data)?;
    Ok(())
}
