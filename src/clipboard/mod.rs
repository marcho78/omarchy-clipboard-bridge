//! Platform clipboard backends. Each exposes:
//!   watch(tx)  — blocks forever, sending a Clip on every clipboard change
//!   set(clip)  — replaces the clipboard content
//!
//! Setting CLIPBOARD_BRIDGE_FAKE=<file> swaps in a file-backed clipboard, used
//! for testing both roles on one machine.

use crate::clip::Clip;
use crossbeam_channel::Sender;

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
use wayland as platform;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

mod fake;

fn fake_path() -> Option<String> {
    std::env::var("CLIPBOARD_BRIDGE_FAKE").ok().filter(|s| !s.is_empty())
}

pub fn watch(tx: Sender<Clip>) -> anyhow::Result<()> {
    match fake_path() {
        Some(p) => fake::watch(&p, tx),
        None => platform::watch(tx),
    }
}

pub fn set(clip: &Clip) -> anyhow::Result<()> {
    match fake_path() {
        Some(p) => fake::set(&p, clip),
        None => platform::set(clip),
    }
}
