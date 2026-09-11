//! Host-side approval of a pairing request.

use std::process::Command;

/// Ask the person at the host whether to accept `guest_name`. On macOS this is
/// a native dialog; elsewhere it is a terminal prompt if stdin is a TTY.
pub fn approve(guest_name: &str, addr: &str, code: &str, auto_accept: bool) -> bool {
    if auto_accept {
        eprintln!("auto-accepting pairing from {guest_name} ({addr}), code {code}");
        return true;
    }
    if cfg!(target_os = "macos") {
        let script = format!(
            "display dialog \"{guest_name} ({addr}) wants to share your clipboard.\\n\\nPairing code: {code}\\n\\nThe VM shows the same code. Allow?\" \
             with title \"Clipboard Bridge\" buttons {{\"Deny\", \"Allow\"}} default button \"Allow\" \
             cancel button \"Deny\" with icon caution giving up after 90"
        );
        return match Command::new("osascript").arg("-e").arg(&script).output() {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout);
                s.contains("button returned:Allow") && !s.contains("gave up:true")
            }
            Err(e) => {
                eprintln!("could not show pairing dialog: {e}; denying");
                false
            }
        };
    }
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        eprint!("Pair with {guest_name} ({addr}), code {code}? [y/N] ");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        return matches!(line.trim(), "y" | "Y" | "yes");
    }
    eprintln!("pairing request from {guest_name} ({addr}) denied: no way to ask (run `serve --auto-accept` or approve interactively)");
    false
}
