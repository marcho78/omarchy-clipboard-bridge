//! Finding the host from the guest. Order: explicit config, mDNS browse,
//! Parallels shared-network convention (gateway .1 => host .2).

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::protocol::SERVICE_TYPE;

/// Advertise the host agent on the local network. Returns the daemon so the
/// caller can keep it alive.
pub fn advertise(name: &str, port: u16) -> Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new()?;
    let hostname = format!("{}.local.", name.trim_end_matches(".local"));
    let info = ServiceInfo::new(SERVICE_TYPE, name, &hostname, "", port, None)?.enable_addr_auto();
    daemon.register(info)?;
    Ok(daemon)
}

/// Browse for a host for up to `timeout`. Returns (addr, port, name).
pub fn browse(timeout: Duration) -> Option<(IpAddr, u16, String)> {
    let daemon = ServiceDaemon::new().ok()?;
    let rx = daemon.browse(SERVICE_TYPE).ok()?;
    let deadline = std::time::Instant::now() + timeout;
    let mut found = None;
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let addrs = info.get_addresses();
                if let Some(ip) = addrs.iter().find(|a| a.is_ipv4()).or_else(|| addrs.iter().next()) {
                    found = Some((ip.to_ip_addr(), info.get_port(), info.get_fullname().to_string()));
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    found
}

/// Parallels shared networking gives the guest a gateway at x.x.x.1 and puts
/// the macOS host at x.x.x.2.
pub fn gateway_guess() -> Option<IpAddr> {
    let route = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in route.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() > 2 && cols[1] == "00000000" {
            let gw = u32::from_str_radix(cols[2], 16).ok()?;
            let o = gw.to_le_bytes();
            return Some(IpAddr::V4(Ipv4Addr::new(o[0], o[1], o[2], 2)));
        }
    }
    None
}
