//! Cross-platform clipboard read/write helpers
//!
//! On Linux, arboard has issues with Wayland (drops clipboard contents,
//! fails to read). This module uses wl-paste/wl-copy on Wayland and
//! xclip/xsel on X11, falling back to arboard on other platforms.

/// Read text from the system clipboard.
///
/// On Linux Wayland: uses `wl-paste`
/// On Linux X11: uses `xclip -selection clipboard -o` or `xsel --clipboard --output`
/// Other platforms: uses arboard
pub fn read_clipboard_text() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let is_wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|s| s == "wayland")
            .unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").is_ok();

        if is_wayland {
            // Wayland: use wl-paste
            if let Ok(output) = std::process::Command::new("wl-paste")
                .arg("--no-newline")
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        } else {
            // X11: try xclip first
            if let Ok(output) = std::process::Command::new("xclip")
                .args(["-selection", "clipboard", "-o"])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }

            // X11: fallback to xsel
            if let Ok(output) = std::process::Command::new("xsel")
                .args(["--clipboard", "--output"])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
    }

    // Fallback: arboard (works on macOS, Windows, and as Linux fallback)
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }

    None
}
