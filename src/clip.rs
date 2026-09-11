//! Clipboard payloads and helpers shared by both platforms.

use sha2::{Digest, Sha256};

pub const MIME_TEXT: &str = "text/plain";
pub const MIME_PNG: &str = "image/png";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clip {
    pub mime: String,
    pub data: Vec<u8>,
}

impl Clip {
    pub fn text(data: Vec<u8>) -> Self {
        Self { mime: MIME_TEXT.into(), data }
    }
    pub fn png(data: Vec<u8>) -> Self {
        Self { mime: MIME_PNG.into(), data }
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.mime.as_bytes());
        h.update([0]);
        h.update(&self.data);
        h.finalize().into()
    }
    pub fn describe(&self) -> String {
        if self.mime == MIME_TEXT {
            let s = String::from_utf8_lossy(&self.data);
            let short: String = s.chars().take(40).collect();
            let short = short.replace('\n', "⏎");
            if s.chars().count() > 40 { format!("text \"{short}…\"") } else { format!("text \"{short}\"") }
        } else {
            format!("{} ({} bytes)", self.mime, self.data.len())
        }
    }
}

/// Remembers the last clip applied or observed locally so a change that came
/// from the peer is not echoed back to it.
#[derive(Default)]
pub struct Echo {
    last: Option<[u8; 32]>,
}

impl Echo {
    /// Returns true if `clip` is new (and records it).
    pub fn note(&mut self, clip: &Clip) -> bool {
        let fp = clip.fingerprint();
        if self.last == Some(fp) {
            return false;
        }
        self.last = Some(fp);
        true
    }
}

// ---------- PNG <-> RGBA, used by the macOS backend, tested everywhere ----------

#[allow(dead_code)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[allow(dead_code)]
pub fn rgba_to_png(img: &Rgba) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header()?;
        w.write_image_data(&img.bytes)?;
        w.finish()?;
    }
    Ok(out)
}

#[allow(dead_code)]
pub fn png_to_rgba(data: &[u8]) -> anyhow::Result<Rgba> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let bytes = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        other => anyhow::bail!("unexpected png color type after transform: {other:?}"),
    };
    Ok(Rgba { width: info.width, height: info.height, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip() {
        let img = Rgba { width: 2, height: 2, bytes: vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 9, 9, 9, 9] };
        let png = rgba_to_png(&img).unwrap();
        let back = png_to_rgba(&png).unwrap();
        assert_eq!((back.width, back.height), (2, 2));
        assert_eq!(back.bytes, img.bytes);
    }

    #[test]
    fn echo_suppresses_repeat() {
        let mut e = Echo::default();
        let c = Clip::text(b"hi".to_vec());
        assert!(e.note(&c));
        assert!(!e.note(&c));
        assert!(e.note(&Clip::text(b"other".to_vec())));
    }
}
