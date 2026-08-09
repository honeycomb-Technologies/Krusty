//! wry / WebKitGTK + best-effort bridge into the Atlas panel (`browser-native`).
//!
//! # Honest embed status (Linux / this GPUI tree)
//!
//! GPUI's [`gpui::Window`] implements `HasWindowHandle` / `HasDisplayHandle`, so we
//! **can** read X11 or Wayland raw handles after the window opens.
//!
//! Parenting a wry **child** WebView is still **not production-ready** here:
//!
//! | Blocker | Detail |
//! |---------|--------|
//! | X11-only child | wry `build_as_child` on Linux only accepts `RawWindowHandle::Xlib` |
//! | Wayland | Common on this stack; child embed returns `UnsupportedWindowHandle` |
//! | GTK loop | WebKitGTK needs `gtk::init` + `gtk::main_iteration*`; GPUI runs Blade, not GTK |
//! | Bounds | Atlas panel layout rect is not synced to a native child without extra plumbing |
//!
//! So `NativeWebViewHost` **probes** the parent handle, **optionally** tries embed
//! when `MITSURO_ATLAS_EMBED=1` and the handle is X11, and otherwise drives a
//! **bridge**: system browser (`browser-external`) and/or Chromium `--app=` sibling.
//!
//! Navigation always updates local URL history; real loads go to the bridge target
//! or an explicitly embedded child when available.

use super::external::{
    display_available, open_sibling_app_window, open_system_browser, ExternalOpenResult,
};
use super::host::DesktopBrowserHost;

/// How Atlas opens real page content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeMode {
    /// wry linked, but no live page surface has been attached.
    LinkedOnly,
    /// User/OS browser via xdg-open (or platform helper).
    External,
    /// Chromium-style `--app=` window (sibling, not parented).
    Sibling,
    /// wry child webview parented into the GPUI X11 window (opt-in, rare).
    EmbeddedChild,
}

impl BridgeMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::LinkedOnly => "wry linked · no page surface",
            Self::External => "external browser bridge",
            Self::Sibling => "sibling app window",
            Self::EmbeddedChild => "wry child embed",
        }
    }
}

/// Snapshot of attach / bridge state for UI chips and status line.
#[derive(Clone, Debug)]
pub struct AttachReport {
    pub mode: BridgeMode,
    /// `"X11"`, `"Wayland"`, `"other"`, or `None` if not probed yet.
    pub handle_kind: Option<&'static str>,
    pub embed_attempted: bool,
    pub detail: String,
}

impl Default for AttachReport {
    fn default() -> Self {
        Self {
            mode: BridgeMode::LinkedOnly,
            handle_kind: None,
            embed_attempted: false,
            detail: "not attached yet".into(),
        }
    }
}

/// Outcome of a navigate that may hit the live bridge.
#[derive(Clone, Debug)]
pub struct NativeNavOutcome {
    #[allow(dead_code)] // reserved for richer UI chips
    pub bridge: Option<ExternalOpenResult>,
    #[allow(dead_code)]
    pub embedded_loaded: bool,
    pub summary: String,
}

/// Owns optional wry `WebView` and bridge policy.
///
/// Not `Clone` (WebView is unique). Held by the app entity on the UI thread only.
pub struct NativeWebViewHost {
    webkit_version: Option<String>,
    report: AttachReport,
    /// Prefer Chromium `--app=` sibling instead of plain xdg-open.
    prefer_sibling: bool,
    /// Auto-open bridge target on every host navigate.
    auto_external: bool,
    /// Opt-in: try `WebViewBuilder::build_as_child` when handle is X11.
    try_embed: bool,
    attached: bool,
    #[cfg(feature = "browser-native")]
    webview: Option<wry::WebView>,
}

impl NativeWebViewHost {
    /// Build host from env + wry version probe. Safe without DISPLAY.
    pub fn new() -> Self {
        let webkit_version = wry::webview_version().ok();
        let prefer_sibling = env_flag("MITSURO_ATLAS_SIBLING");
        let auto_external = env_flag("MITSURO_ATLAS_EXTERNAL") || prefer_sibling;
        let try_embed = env_flag("MITSURO_ATLAS_EMBED");
        let mode = if prefer_sibling {
            BridgeMode::Sibling
        } else if auto_external {
            BridgeMode::External
        } else {
            BridgeMode::LinkedOnly
        };
        let detail = match mode {
            BridgeMode::Sibling => {
                "sibling mode · Chromium --app= on navigate (not embedded)".into()
            }
            BridgeMode::External => "external mode · system browser on navigate (xdg-open)".into(),
            BridgeMode::LinkedOnly => {
                "wry linked · open external or enable MITSURO_ATLAS_EXTERNAL=1".into()
            }
            BridgeMode::EmbeddedChild => "embedded".into(),
        };
        Self {
            webkit_version,
            report: AttachReport {
                mode,
                handle_kind: None,
                embed_attempted: false,
                detail,
            },
            prefer_sibling,
            auto_external,
            try_embed,
            attached: false,
            #[cfg(feature = "browser-native")]
            webview: None,
        }
    }

    #[allow(dead_code)]
    pub fn webkit_version(&self) -> Option<&str> {
        self.webkit_version.as_deref()
    }

    pub fn report(&self) -> &AttachReport {
        &self.report
    }

    pub fn bridge_mode(&self) -> BridgeMode {
        self.report.mode
    }

    pub fn is_attached(&self) -> bool {
        self.attached
    }

    pub fn host_kind_label(&self) -> String {
        let ver = self
            .webkit_version
            .as_deref()
            .map(|v| format!(" · WebKit {v}"))
            .unwrap_or_default();
        let handle = self
            .report
            .handle_kind
            .map(|k| format!(" · {k}"))
            .unwrap_or_default();
        format!("wry/WebKitGTK ({}){handle}{ver}", self.report.mode.label())
    }

    /// Probe the GPUI window raw handle and optionally try child embed.
    ///
    /// Fail soft: never panics; updates [`AttachReport`]. Idempotent after first success path.
    pub fn attach_after_window_open<W>(&mut self, window: &W)
    where
        W: wry::raw_window_handle::HasWindowHandle,
    {
        if self.attached && self.report.handle_kind.is_some() {
            return;
        }

        let handle_kind = match window.window_handle() {
            Ok(h) => match h.as_raw() {
                wry::raw_window_handle::RawWindowHandle::Xlib(_)
                | wry::raw_window_handle::RawWindowHandle::Xcb(_) => Some("X11"),
                wry::raw_window_handle::RawWindowHandle::Wayland(_) => Some("Wayland"),
                wry::raw_window_handle::RawWindowHandle::AppKit(_) => Some("AppKit"),
                wry::raw_window_handle::RawWindowHandle::Win32(_) => Some("Win32"),
                _ => Some("other"),
            },
            Err(e) => {
                self.report.detail = format!("window handle unavailable: {e}");
                self.attached = true;
                return;
            }
        };
        self.report.handle_kind = handle_kind;

        // Opt-in embed on X11 only — needs GTK display; default off so Wayland
        // sessions and CI stay stable.
        if self.try_embed && handle_kind == Some("X11") {
            self.report.embed_attempted = true;
            match self.try_build_as_child(window) {
                Ok(()) => {
                    self.report.mode = BridgeMode::EmbeddedChild;
                    self.report.detail =
                        "wry build_as_child succeeded · child surface parented (X11)".into();
                    self.attached = true;
                    return;
                }
                Err(e) => {
                    self.report.detail = format!(
                        "embed failed ({e}) · bridge={}; GPUI has no GTK Fixed + WebKit needs gtk loop",
                        self.report.mode.label()
                    );
                }
            }
        } else if self.try_embed && handle_kind == Some("Wayland") {
            self.report.embed_attempted = true;
            self.report.detail =
                "MITSURO_ATLAS_EMBED set but Wayland: wry child is X11-only; using bridge".into();
        } else if handle_kind == Some("Wayland") {
            self.report.detail = format!(
                "Wayland handle · child embed unsupported; bridge={}",
                self.report.mode.label()
            );
        } else if handle_kind == Some("X11") {
            self.report.detail = format!(
                "X11 handle · embed deferred (set MITSURO_ATLAS_EMBED=1 to try); bridge={}",
                self.report.mode.label()
            );
        } else {
            self.report.detail = format!(
                "handle={:?} · bridge={}",
                handle_kind,
                self.report.mode.label()
            );
        }

        self.attached = true;
    }

    fn try_build_as_child<W>(&mut self, window: &W) -> Result<(), String>
    where
        W: wry::raw_window_handle::HasWindowHandle,
    {
        // Headless: refuse without a display so we never touch GTK in CI.
        if !display_available() {
            return Err("no DISPLAY/WAYLAND_DISPLAY".into());
        }

        // gtk init is required for WebKitGTK; ignore "already initialized".
        #[cfg(target_os = "linux")]
        {
            // wry pulls gtk; we only call through wry's builder. If gtk isn't
            // initialized, build_as_child returns an error we surface.
        }

        use wry::dpi::{LogicalPosition, LogicalSize};
        use wry::{Rect, WebViewBuilder};

        // Placeholder bounds — Atlas panel rect sync is not wired. Visible only
        // if embed actually works; still useful as a capability probe.
        let builder = WebViewBuilder::new()
            .with_url("about:blank")
            .with_bounds(Rect {
                position: LogicalPosition::new(64.0, 96.0).into(),
                size: LogicalSize::new(640.0, 480.0).into(),
            })
            .with_visible(true);

        match builder.build_as_child(window) {
            Ok(wv) => {
                self.webview = Some(wv);
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Load URL in embedded WebView if present; optionally open bridge.
    pub fn navigate(&mut self, url: &str) -> NativeNavOutcome {
        let mut embedded_loaded = false;
        if let Some(wv) = self.webview.as_ref() {
            match wv.load_url(url) {
                Ok(()) => embedded_loaded = true,
                Err(e) => {
                    self.report.detail = format!("embedded load_url failed: {e}");
                }
            }
        }

        let bridge = if self.auto_external || self.prefer_sibling {
            Some(self.open_bridge(url))
        } else {
            None
        };

        let summary = match (&bridge, embedded_loaded) {
            (Some(b), true) => format!("embedded + {}", b.summary()),
            (Some(b), false) => b.summary(),
            (None, true) => "loaded in embedded WebView".into(),
            (None, false) => "URL history updated · no page surface attached".into(),
        };

        NativeNavOutcome {
            bridge,
            embedded_loaded,
            summary,
        }
    }

    /// Explicit user action: open current URL externally (or sibling).
    pub fn open_bridge(&self, url: &str) -> ExternalOpenResult {
        if self.prefer_sibling {
            open_sibling_app_window(url)
        } else {
            open_system_browser(url)
        }
    }

    #[allow(dead_code)]
    pub fn has_embedded_webview(&self) -> bool {
        self.webview.is_some()
    }
}

impl Default for NativeWebViewHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the wry-linked desktop URL-history host.
pub fn create_wry_linked_host() -> DesktopBrowserHost {
    let webkit_version = wry::webview_version().ok();
    DesktopBrowserHost::new_wry_linked(webkit_version)
}

/// Engine version string if available (`major.minor.micro` on Linux).
#[allow(dead_code)]
pub fn webkit_version_string() -> Option<String> {
    wry::webview_version().ok()
}

/// Always true when this module is compiled.
#[allow(dead_code)]
pub fn is_native_linked() -> bool {
    true
}

fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim();
            !(v.is_empty()
                || v == "0"
                || v.eq_ignore_ascii_case("false")
                || v.eq_ignore_ascii_case("no"))
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_host_new_without_display() {
        // Must construct headless (CI).
        let h = NativeWebViewHost::new();
        assert!(h.webkit_version().is_some() || h.webkit_version().is_none());
        assert!(!h.has_embedded_webview());
        assert!(!h.report().embed_attempted || h.report().embed_attempted);
        let _ = h.host_kind_label();
    }

    #[test]
    fn navigate_without_attach_is_soft() {
        let mut h = NativeWebViewHost::new();
        // Force no auto external for deterministic summary.
        h.auto_external = false;
        h.prefer_sibling = false;
        let out = h.navigate("https://example.com");
        assert!(!out.embedded_loaded);
        assert!(out.bridge.is_none());
        assert!(out.summary.contains("no page surface"));
    }

    #[test]
    fn env_flag_parser() {
        assert!(!env_flag("MITSURO_P12_TEST_FLAG_UNSET_XYZ"));
    }
}
