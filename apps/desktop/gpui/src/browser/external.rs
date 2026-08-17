//! System-browser bridge (`browser-external`).
//!
//! Opens URLs with the desktop default handler (`xdg-open` on Linux). Used when
//! GPUI cannot parent a wry child surface (Wayland, no GTK loop, headless CI).

use std::process::Command;

/// Result of requesting the OS to open a URL in an external browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalOpenResult {
    Spawned { command: &'static str },
    Failed { detail: String },
}

impl ExternalOpenResult {
    pub fn ok(&self) -> bool {
        matches!(self, Self::Spawned { .. })
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Spawned { command } => format!("opened via {command}"),
            Self::Failed { detail } => format!("external open failed · {detail}"),
        }
    }
}

/// Open `url` in the system browser. Fail soft — never panics.
///
/// Linux: `xdg-open`, then `x-www-browser`, then common Chromium binaries.
/// Does **not** require a DISPLAY for the call itself (spawn may still fail
/// headless); safe to call from unit tests with a dummy URL.
pub fn open_system_browser(url: &str) -> ExternalOpenResult {
    // Refuse control characters before trim (trim would hide trailing `\n`).
    if url.chars().any(|c| matches!(c, '\n' | '\r' | '\0')) {
        return ExternalOpenResult::Failed {
            detail: "URL contains control characters".into(),
        };
    }
    let url = url.trim();
    if url.is_empty() {
        return ExternalOpenResult::Failed {
            detail: "empty URL".into(),
        };
    }

    #[cfg(target_os = "linux")]
    {
        open_linux(url)
    }
    #[cfg(target_os = "macos")]
    {
        return spawn_one("open", &["--", url]);
    }
    #[cfg(target_os = "windows")]
    {
        return spawn_one("cmd", &["/C", "start", "", url]);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        ExternalOpenResult::Failed {
            detail: "no system browser helper on this platform".into(),
        }
    }
}

/// Chromium-style app window (sibling surface, not embedded). Better UX than a
/// random tab when available. Falls back to [`open_system_browser`].
#[allow(dead_code)] // used from native host when browser-native is on
pub fn open_sibling_app_window(url: &str) -> ExternalOpenResult {
    let url = url.trim();
    if url.is_empty() {
        return ExternalOpenResult::Failed {
            detail: "empty URL".into(),
        };
    }
    let app_arg = format!("--app={url}");
    for bin in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "brave-browser",
    ] {
        match Command::new(bin)
            .arg(&app_arg)
            .arg("--new-window")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                return ExternalOpenResult::Spawned { command: bin };
            }
            Err(_) => continue,
        }
    }
    open_system_browser(url)
}

#[cfg(target_os = "linux")]
fn open_linux(url: &str) -> ExternalOpenResult {
    for (bin, args) in [
        ("xdg-open", vec![url.to_string()]),
        ("x-www-browser", vec![url.to_string()]),
        ("gio", vec!["open".into(), url.to_string()]),
    ] {
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = spawn_one(bin, &arg_refs);
        if r.ok() {
            return r;
        }
    }
    ExternalOpenResult::Failed {
        detail: "xdg-open / x-www-browser / gio all failed to spawn".into(),
    }
}

fn spawn_one(bin: &'static str, args: &[&str]) -> ExternalOpenResult {
    match Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => ExternalOpenResult::Spawned { command: bin },
        Err(e) => ExternalOpenResult::Failed {
            detail: format!("{bin}: {e}"),
        },
    }
}

/// Whether a display server looks available (not required for unit tests).
#[allow(dead_code)] // used from native host when browser-native is on
pub fn display_available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_url_fails_soft() {
        let r = open_system_browser("  ");
        assert!(!r.ok());
        assert!(r.summary().contains("empty"));
    }

    #[test]
    fn control_chars_rejected() {
        let r = open_system_browser("https://example.com/\n");
        assert!(!r.ok());
    }

    #[test]
    fn display_probe_is_callable() {
        // Must not panic headless.
        let _ = display_available();
    }
}
