//! Installing the daemon as a user service: systemd on Linux, launchd on macOS.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::{config_dir, State};

pub const UNIT: &str = "clipboard-bridge.service";
#[allow(dead_code)]
pub const LABEL: &str = "com.omarchy.clipboard-bridge";

fn exe() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.canonicalize()?)
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("running {:?}", cmd.get_program()))?;
    if !status.success() {
        bail!("{:?} exited with {status}", cmd.get_program());
    }
    Ok(())
}

pub fn is_host() -> bool {
    cfg!(target_os = "macos")
}

// ---------- Linux / systemd ----------

#[cfg(target_os = "linux")]
pub fn install(_auto_accept: bool) -> Result<()> {
    let system_unit = PathBuf::from("/usr/lib/systemd/user").join(UNIT);
    let user_dir = dirs::config_dir().unwrap().join("systemd/user");
    let user_unit = user_dir.join(UNIT);
    if system_unit.exists() {
        println!("using packaged unit {}", system_unit.display());
        let _ = fs::remove_file(&user_unit);
    } else {
        fs::create_dir_all(&user_dir)?;
        fs::write(
            &user_unit,
            format!(
                "[Unit]\nDescription=Clipboard bridge to the macOS host (Parallels)\nPartOf=graphical-session.target\nAfter=graphical-session.target\n\n\
                 [Service]\nType=simple\nExecStart={} connect\nRestart=always\nRestartSec=3\n\n\
                 [Install]\nWantedBy=graphical-session.target\n",
                exe()?.display()
            ),
        )?;
        println!("wrote {}", user_unit.display());
    }
    run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run(Command::new("systemctl").args(["--user", "enable", "--now", UNIT]))?;
    println!("guest service enabled and started.");
    println!();
    println!("Next, on your Mac, run:");
    println!("    curl -fsSL https://raw.githubusercontent.com/marcho78/omarchy-clipboard-bridge/main/host/install.sh | bash");
    println!("then approve the pairing dialog that appears. Check with: clipboard-bridge status");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall(purge: bool) -> Result<()> {
    let _ = Command::new("systemctl").args(["--user", "disable", "--now", UNIT]).status();
    let _ = fs::remove_file(dirs::config_dir().unwrap().join("systemd/user").join(UNIT));
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    if purge {
        let _ = fs::remove_dir_all(config_dir());
    }
    println!("guest service removed{}", if purge { " (config purged)" } else { "" });
    Ok(())
}

#[cfg(target_os = "linux")]
fn service_state() -> String {
    Command::new("systemctl")
        .args(["--user", "is-active", UNIT])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg(target_os = "linux")]
pub fn logs() -> Result<()> {
    let err = std::process::Command::new("journalctl").args(["--user", "-u", UNIT, "-f", "-o", "cat"]).exec_replace();
    bail!("could not run journalctl: {err}")
}

// ---------- macOS / launchd ----------

#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    dirs::home_dir().unwrap().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn log_path() -> PathBuf {
    dirs::home_dir().unwrap().join("Library/Logs/clipboard-bridge.log")
}

#[cfg(target_os = "macos")]
fn uid() -> String {
    Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub fn install(auto_accept: bool) -> Result<()> {
    let mut cfg = crate::config::HostConfig::load()?;
    if auto_accept {
        cfg.auto_accept = true;
    }
    cfg.save()?;
    let plist = plist_path();
    fs::create_dir_all(plist.parent().unwrap())?;
    fs::create_dir_all(log_path().parent().unwrap())?;
    let exe = exe()?;
    fs::write(
        &plist,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key><array><string>{}</string><string>serve</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>ThrottleInterval</key><integer>3</integer>
    <key>ProcessType</key><string>Interactive</string>
    <key>StandardOutPath</key><string>{}</string>
    <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
            exe.display(),
            log_path().display(),
            log_path().display()
        ),
    )?;
    let target = format!("gui/{}", uid());
    let _ = Command::new("launchctl").args(["bootout", &format!("{target}/{LABEL}")]).output();
    run(Command::new("launchctl").args(["bootstrap", &target, plist.to_str().unwrap()]))?;
    let _ = Command::new("launchctl").args(["kickstart", "-k", &format!("{target}/{LABEL}")]).output();
    println!("host agent installed and started (log: {})", log_path().display());
    println!("If macOS asks whether clipboard-bridge may accept incoming connections, click Allow.");
    println!("When your VM connects, a pairing dialog will appear. Click Allow there too.");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall(purge: bool) -> Result<()> {
    let _ = Command::new("launchctl").args(["bootout", &format!("gui/{}/{LABEL}", uid())]).output();
    let _ = fs::remove_file(plist_path());
    if purge {
        let _ = fs::remove_dir_all(config_dir());
        let _ = fs::remove_file(log_path());
    }
    println!("host agent removed{}", if purge { " (config purged)" } else { "" });
    Ok(())
}

#[cfg(target_os = "macos")]
fn service_state() -> String {
    match Command::new("launchctl").args(["print", &format!("gui/{}/{LABEL}", uid())]).output() {
        Ok(o) if o.status.success() => {
            if String::from_utf8_lossy(&o.stdout).contains("state = running") { "running".into() } else { "loaded".into() }
        }
        _ => "not installed".into(),
    }
}

#[cfg(target_os = "macos")]
pub fn logs() -> Result<()> {
    let err = std::process::Command::new("tail").args(["-n", "50", "-f", log_path().to_str().unwrap()]).exec_replace();
    bail!("could not run tail: {err}")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn install(_: bool) -> Result<()> { bail!("unsupported platform") }
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn uninstall(_: bool) -> Result<()> { bail!("unsupported platform") }
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn service_state() -> String { "unknown".into() }
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn logs() -> Result<()> { bail!("unsupported platform") }

trait ExecReplace {
    fn exec_replace(&mut self) -> std::io::Error;
}
impl ExecReplace for Command {
    #[cfg(unix)]
    fn exec_replace(&mut self) -> std::io::Error {
        use std::os::unix::process::CommandExt;
        self.exec()
    }
    #[cfg(not(unix))]
    fn exec_replace(&mut self) -> std::io::Error {
        std::io::Error::other("exec unsupported")
    }
}

// ---------- status ----------

pub fn status() -> Result<()> {
    let state = State::read();
    let host_role = state.as_ref().map_or(is_host(), |s| s.role == "host");
    println!("role:     {}", if host_role { "host" } else { "guest" });
    println!("service:  {}", service_state());
    if host_role {
        let cfg = crate::config::HostConfig::load()?;
        println!("listen:   {}:{}", cfg.bind, cfg.port);
        println!("paired:   {}", if cfg.guests.is_empty() { "none".to_string() } else { cfg.guests.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ") });
    } else {
        let cfg = crate::config::GuestConfig::load()?;
        println!("host:     {}", if cfg.host.is_empty() { "auto (mDNS, then gateway .2)".to_string() } else { cfg.host.clone() });
        println!("paired:   {}", if cfg.paired() { format!("yes, with \"{}\"", cfg.host_name) } else { "no".into() });
    }
    match state {
        Some(s) => {
            println!("link:     {}{}", if s.connected { "connected" } else { "not connected" }, if s.peer.is_empty() { String::new() } else { format!(" ({})", s.peer) });
            println!("detail:   {} (as of {})", s.detail, s.updated);
        }
        None => println!("link:     no state yet (daemon not running?)"),
    }
    println!("config:   {}", config_dir().display());
    Ok(())
}
