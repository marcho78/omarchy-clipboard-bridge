use std::process::Command;

/// Human-facing name of this machine.
pub fn local_name() -> String {
    if cfg!(target_os = "macos") {
        if let Ok(out) = Command::new("scutil").args(["--get", "ComputerName"]).output() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string("/etc/hostname") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

pub fn log(msg: impl AsRef<str>) {
    eprintln!("{}", msg.as_ref());
}

/// Desktop notification on Linux via notify-send; silently a no-op elsewhere
/// or when notify-send is missing.
pub fn notify(summary: &str, body: &str) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let _ = Command::new("notify-send")
        .args(["-a", "Clipboard Bridge", "-i", "edit-paste", summary, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Hold an exclusive lock for the lifetime of the returned file so only one
/// daemon per role runs at a time.
pub fn single_instance(role: &str) -> anyhow::Result<std::fs::File> {
    use rustix::fs::{flock, FlockOperation};
    let dir = crate::config::config_dir();
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::File::create(dir.join(format!("{role}.lock")))?;
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| anyhow::anyhow!("another clipboard-bridge {role} is already running (lock held in {})", dir.display()))?;
    Ok(file)
}
