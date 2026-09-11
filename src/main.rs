//! clipboard-bridge: shared clipboard between an Omarchy (Hyprland) VM and its
//! macOS host under Parallels Desktop.

mod clip;
mod clipboard;
mod config;
mod discovery;
mod guest;
mod host;
mod net;
mod pairing;
mod protocol;
mod service;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clipboard-bridge", version, about = "Shared clipboard between an Omarchy VM and its macOS host")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Host mode: listen for guests and mirror the macOS clipboard (runs on the Mac)
    Serve {
        /// Accept pairing requests without a dialog
        #[arg(long)]
        auto_accept: bool,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Guest mode: find the host, pair, and mirror the Wayland clipboard (runs in the VM)
    Connect {
        /// Host IP address (default: mDNS discovery, then Parallels gateway .2)
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Guest: forget any existing pairing and pair with the host now, then exit
    Pair {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Install as a user service (systemd on Linux, launchd on macOS) and start it
    Install {
        /// macOS only: accept pairing requests without a dialog
        #[arg(long)]
        auto_accept: bool,
    },
    /// Stop and remove the user service
    Uninstall {
        /// Also delete the configuration and pairing secrets
        #[arg(long)]
        purge: bool,
    },
    /// Show service, pairing and link state
    Status,
    /// Follow the daemon log
    Logs,
    /// Forget all pairings (host: all guests; guest: its host)
    Forget,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve { auto_accept, bind, port } => {
            let mut cfg = config::HostConfig::load()?;
            if let Some(b) = bind {
                cfg.bind = b;
            }
            if let Some(p) = port {
                cfg.port = p;
            }
            host::run(cfg, auto_accept)
        }
        Cmd::Connect { host, port } => guest::run(config::GuestConfig::load()?, guest::Options { host, port, once: false }),
        Cmd::Pair { host, port } => {
            let mut cfg = config::GuestConfig::load()?;
            cfg.id.clear();
            cfg.secret.clear();
            cfg.save()?;
            let r = guest::run(cfg, guest::Options { host, port, once: true });
            if r.is_ok() && !service::is_host() {
                let _ = std::process::Command::new("systemctl").args(["--user", "try-restart", service::UNIT]).status();
            }
            r
        }
        Cmd::Install { auto_accept } => service::install(auto_accept),
        Cmd::Uninstall { purge } => service::uninstall(purge),
        Cmd::Status => service::status(),
        Cmd::Logs => service::logs(),
        Cmd::Forget => {
            if service::is_host() {
                let mut cfg = config::HostConfig::load()?;
                cfg.guests.clear();
                cfg.save()?;
                println!("forgot all paired guests");
            } else {
                let mut cfg = config::GuestConfig::load()?;
                cfg.id.clear();
                cfg.secret.clear();
                cfg.host_name.clear();
                cfg.save()?;
                println!("forgot host pairing; the service will pair again on next connect");
            }
            Ok(())
        }
    }
}
