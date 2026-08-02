//! Cross-platform clipboard read/write helpers
//!
//! On Linux, arboard has issues with Wayland (drops clipboard contents,
//! fails to read). This module uses wl-paste/wl-copy on Wayland and
//! xclip/xsel on X11, falling back to arboard on other platforms.

/// Clipboard image data normalized to RGBA bytes.
#[derive(Debug, Clone)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba_bytes: Vec<u8>,
}

/// Read an image from the system clipboard.
pub fn read_clipboard_image() -> Option<ClipboardImage> {
    if let Some(image) = read_clipboard_image_platform() {
        return Some(image);
    }

    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;

    Some(ClipboardImage {
        width: image.width,
        height: image.height,
        rgba_bytes: image.bytes.into_owned(),
    })
}

#[cfg(target_os = "linux")]
fn read_clipboard_image_platform() -> Option<ClipboardImage> {
    let is_wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|s| s == "wayland")
        .unwrap_or(false)
        || std::env::var("WAYLAND_DISPLAY").is_ok();

    if is_wayland {
        if let Ok(output) = std::process::Command::new("wl-paste")
            .args(["--type", "image/png"])
            .output()
        {
            if output.status.success() && !output.stdout.is_empty() {
                if let Some(image) = decode_clipboard_image(&output.stdout) {
                    return Some(image);
                }
            }
        }
    } else if let Ok(output) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
    {
        if output.status.success() && !output.stdout.is_empty() {
            if let Some(image) = decode_clipboard_image(&output.stdout) {
                return Some(image);
            }
        }
    }

    None
}

#[cfg(not(target_os = "linux"))]
fn read_clipboard_image_platform() -> Option<ClipboardImage> {
    None
}

#[cfg(target_os = "linux")]
fn decode_clipboard_image(bytes: &[u8]) -> Option<ClipboardImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();

    Some(ClipboardImage {
        width: width as usize,
        height: height as usize,
        rgba_bytes: image.into_raw(),
    })
}

/// Save clipboard RGBA bytes to a temporary PNG for UI preview/open flows.
pub fn save_clipboard_image_preview(
    width: usize,
    height: usize,
    rgba_bytes: &[u8],
    clipboard_id: &str,
) -> anyhow::Result<std::path::PathBuf> {
    use image::{ImageBuffer, Rgba};

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width as u32, height as u32, rgba_bytes.to_vec())
            .ok_or_else(|| anyhow::anyhow!("Failed to create clipboard preview image"))?;

    let preview_dir = std::env::temp_dir().join("mitsuro-clipboard-previews");
    std::fs::create_dir_all(&preview_dir)?;
    let preview_path = preview_dir.join(format!("clipboard-{}.png", clipboard_id));
    img.save(&preview_path)?;
    Ok(preview_path)
}

/// Detect paste payloads that are not meaningful text.
///
/// Terminals can surface image clipboard data as bracketed paste text containing
/// escape sequences or invalid UTF-8 replacement characters. Those bytes should
/// not be inserted into the message input.
pub fn looks_like_non_text_paste(text: &str) -> bool {
    if text.starts_with("\u{FFFD}PNG") || text.contains("PNG\r\n\u{1A}\n") {
        return true;
    }

    text.chars()
        .any(|ch| ch == '\u{FFFD}' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')))
}

fn usable_clipboard_text(text: String) -> Option<String> {
    if text.is_empty() || looks_like_non_text_paste(&text) {
        None
    } else {
        Some(text)
    }
}

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
            // Wayland: explicitly request text/plain so image clipboards are not
            // coerced into unreadable bytes.
            if let Ok(output) = std::process::Command::new("wl-paste")
                .args(["--type", "text/plain", "--no-newline"])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if let Some(text) = usable_clipboard_text(text) {
                        return Some(text);
                    }
                }
            }
        } else {
            // X11: try xclip first, explicitly requesting text/plain.
            if let Ok(output) = std::process::Command::new("xclip")
                .args(["-selection", "clipboard", "-t", "text/plain", "-o"])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if let Some(text) = usable_clipboard_text(text) {
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
                    if let Some(text) = usable_clipboard_text(text) {
                        return Some(text);
                    }
                }
            }
        }
    }

    // Fallback: arboard (works on macOS, Windows, and as Linux fallback)
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        if let Ok(text) = clipboard.get_text() {
            if let Some(text) = usable_clipboard_text(text) {
                return Some(text);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::looks_like_non_text_paste;

    #[test]
    fn plain_text_paste_is_text() {
        assert!(!looks_like_non_text_paste("hello\nworld\t!"));
    }

    #[test]
    fn escape_sequences_are_not_text() {
        assert!(looks_like_non_text_paste("\u{1b}]1337;File=inline=1:abc"));
    }

    #[test]
    fn replacement_characters_are_not_text() {
        assert!(looks_like_non_text_paste("\u{FFFD}PNG\r\n"));
    }
}
