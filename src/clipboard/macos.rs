//! macOS backend. NSPasteboard has no change notification, only a change
//! counter, so we poll the counter (cheap, in-process) and read the content
//! through `arboard` only when it moves.

use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use objc2_app_kit::NSPasteboard;

use crate::clip::{png_to_rgba, rgba_to_png, Clip, Rgba, MIME_PNG};

const POLL: Duration = Duration::from_millis(150);

fn change_count() -> isize {
    NSPasteboard::generalPasteboard().changeCount()
}

pub fn watch(tx: Sender<Clip>) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening the pasteboard")?;
    let mut last = change_count();
    loop {
        sleep(POLL);
        let now = change_count();
        if now == last {
            continue;
        }
        last = now;
        if let Some(clip) = read(&mut cb) {
            let _ = tx.send(clip);
        }
    }
}

fn read(cb: &mut arboard::Clipboard) -> Option<Clip> {
    if let Ok(img) = cb.get_image() {
        let rgba = Rgba { width: img.width as u32, height: img.height as u32, bytes: img.bytes.into_owned() };
        return rgba_to_png(&rgba).ok().map(Clip::png);
    }
    match cb.get_text() {
        Ok(t) if !t.is_empty() => Some(Clip::text(t.into_bytes())),
        _ => None,
    }
}

pub fn set(clip: &Clip) -> Result<()> {
    let mut cb = arboard::Clipboard::new().context("opening the pasteboard")?;
    if clip.mime == MIME_PNG {
        let rgba = png_to_rgba(&clip.data)?;
        cb.set_image(arboard::ImageData {
            width: rgba.width as usize,
            height: rgba.height as usize,
            bytes: rgba.bytes.into(),
        })?;
    } else {
        cb.set_text(String::from_utf8_lossy(&clip.data).into_owned())?;
    }
    Ok(())
}
