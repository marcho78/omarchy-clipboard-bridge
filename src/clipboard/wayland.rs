//! Wayland backend using the wlr-data-control protocol (supported by Hyprland
//! and other wlroots-style compositors). Watching is event-driven: the
//! compositor tells us about every new selection and we read it through a pipe.
//! Setting goes through `wl-copy`, which stays alive to serve the selection.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{delegate_noop, event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};

use crate::clip::{Clip, MIME_PNG};

const TEXT_MIMES: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];

struct State {
    conn: Connection,
    tx: Sender<Clip>,
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwlrDataControlManagerV1>,
    device: Option<ZwlrDataControlDeviceV1>,
    offers: HashMap<ObjectId, Vec<String>>,
}

pub fn watch(tx: Sender<Clip>) -> Result<()> {
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    display.get_registry(&qh, ());

    let mut state = State { conn: conn.clone(), tx, seat: None, manager: None, device: None, offers: HashMap::new() };
    queue.roundtrip(&mut state)?;

    let manager = state
        .manager
        .clone()
        .context("compositor does not support zwlr_data_control_manager_v1")?;
    let seat = state.seat.clone().context("no wl_seat found")?;
    state.device = Some(manager.get_data_device(&seat, &qh, ()));

    loop {
        queue.blocking_dispatch(&mut state)?;
    }
}

pub fn set(clip: &Clip) -> Result<()> {
    // wl-copy forks a server that keeps offering the selection. Put it in its
    // own process group so it outlives this daemon if we are killed or
    // restarted (for example when omarchy-shell reloads plugins); otherwise
    // the clipboard content would vanish with us.
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg(if clip.mime == MIME_PNG { MIME_PNG } else { "text/plain;charset=utf-8" })
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning wl-copy (install wl-clipboard)")?;
    child.stdin.take().unwrap().write_all(&clip.data)?;
    let status = child.wait()?;
    anyhow::ensure!(status.success(), "wl-copy exited with {status}");
    Ok(())
}

impl State {
    fn take_selection(&mut self, offer: ZwlrDataControlOfferV1) {
        let mimes = self.offers.remove(&offer.id()).unwrap_or_default();
        let choice = if mimes.iter().any(|m| m == MIME_PNG) {
            Some((MIME_PNG, true))
        } else {
            TEXT_MIMES.iter().find(|m| mimes.iter().any(|x| x == *m)).map(|m| (*m, false))
        };
        if let Some((mime, is_png)) = choice {
            match self.receive(&offer, mime) {
                Ok(data) if !data.is_empty() => {
                    let clip = if is_png { Clip::png(data) } else { Clip::text(data) };
                    let _ = self.tx.send(clip);
                }
                Ok(_) => {}
                Err(e) => eprintln!("reading selection failed: {e:#}"),
            }
        }
        offer.destroy();
    }

    fn receive(&self, offer: &ZwlrDataControlOfferV1, mime: &str) -> Result<Vec<u8>> {
        let (read_end, write_end) = rustix::pipe::pipe()?;
        offer.receive(mime.to_string(), write_end.as_fd());
        drop(write_end);
        self.conn.flush()?;
        let mut buf = Vec::new();
        File::from(read_end).read_to_end(&mut buf)?;
        Ok(buf)
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(4), qh, ()));
                }
                "zwlr_data_control_manager_v1" => {
                    state.manager =
                        Some(registry.bind::<ZwlrDataControlManagerV1, _, _>(name, version.min(2), qh, ()));
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ZwlrDataControlManagerV1);

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if let Some(offer) = id {
                    state.take_selection(offer);
                }
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                if let Some(offer) = id {
                    state.offers.remove(&offer.id());
                    offer.destroy();
                }
            }
            zwlr_data_control_device_v1::Event::Finished => {
                eprintln!("data control device finished; compositor went away?");
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}
