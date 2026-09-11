//! On-disk configuration and runtime state files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::protocol::DEFAULT_PORT;

pub fn config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("CLIPBOARD_BRIDGE_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("clipboard-bridge")
}

fn write_private(path: &PathBuf, contents: &str) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ---------- guest ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GuestConfig {
    /// Host address override. Empty = discover via mDNS, then gateway .2.
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Assigned by the host during pairing.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub secret: String,
    /// Host name recorded at pairing time, for display only.
    #[serde(default)]
    pub host_name: String,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl GuestConfig {
    pub fn path() -> PathBuf {
        config_dir().join("guest.toml")
    }
    pub fn load() -> Result<Self> {
        match fs::read_to_string(Self::path()) {
            Ok(s) => Ok(toml::from_str(&s).context("parsing guest.toml")?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self { port: DEFAULT_PORT, ..Default::default() }),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self) -> Result<()> {
        write_private(&Self::path(), &toml::to_string_pretty(self)?)
    }
    pub fn paired(&self) -> bool {
        !self.id.is_empty() && !self.secret.is_empty()
    }
}

// ---------- host ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedGuest {
    pub id: String,
    pub name: String,
    pub secret: String,
    #[serde(default)]
    pub paired_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Accept pairing requests without asking. Only sensible when the port is
    /// reachable solely from your own VMs.
    #[serde(default)]
    pub auto_accept: bool,
    #[serde(default)]
    pub guests: Vec<PairedGuest>,
}

fn default_bind() -> String {
    "0.0.0.0".into()
}

impl Default for HostConfig {
    fn default() -> Self {
        Self { bind: default_bind(), port: DEFAULT_PORT, auto_accept: false, guests: vec![] }
    }
}

impl HostConfig {
    pub fn path() -> PathBuf {
        config_dir().join("host.toml")
    }
    pub fn load() -> Result<Self> {
        match fs::read_to_string(Self::path()) {
            Ok(s) => Ok(toml::from_str(&s).context("parsing host.toml")?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self) -> Result<()> {
        write_private(&Self::path(), &toml::to_string_pretty(self)?)
    }
}

// ---------- runtime state (for `status`) ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    pub role: String,
    pub connected: bool,
    pub peer: String,
    pub detail: String,
    pub updated: String,
}

impl State {
    pub fn path() -> PathBuf {
        config_dir().join("state.json")
    }
    pub fn write(&self) {
        let _ = fs::create_dir_all(config_dir());
        let _ = fs::write(Self::path(), serde_json::to_string(self).unwrap_or_default());
    }
    pub fn read() -> Option<Self> {
        serde_json::from_str(&fs::read_to_string(Self::path()).ok()?).ok()
    }
}

pub fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Simple UTC clock without pulling in chrono.
    let days = secs / 86400;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
