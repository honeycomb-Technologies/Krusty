//! Atlas browser host: URL history, optional wry link, and external bridge.
//!
//! # Architecture
//!
//! - [`BrowserHost`] — navigate / history / URL / title / status contract
//! - [`DesktopBrowserHost`] — in-process URL history; never fabricated page content
//! - `browser-native` feature — links **wry** (WebKitGTK on Linux); [`NativeWebViewHost`]
//!   probes GPUI `HasWindowHandle` and may try child embed (`MITSURO_ATLAS_EMBED=1`, X11 only).
//!   Full production embed is blocked on Wayland + missing GTK loop (see module docs).
//! - `browser-external` feature (default) — documents system-browser bridge; the
//!   [`external`] helpers always compile (no DISPLAY required). Env:
//!   `MITSURO_ATLAS_EXTERNAL`, `MITSURO_ATLAS_SIBLING`, `MITSURO_ATLAS_EMBED`

mod external;
mod host;

#[cfg(feature = "browser-native")]
mod native;

#[allow(unused_imports)] // public API for callers / tests
pub use external::{
    display_available, open_sibling_app_window, open_system_browser, ExternalOpenResult,
};
pub use host::{create_default_host, BrowserHost, DesktopBrowserHost};

#[cfg(feature = "browser-native")]
#[allow(unused_imports)] // public API for callers / tests
pub use native::{AttachReport, BridgeMode, NativeNavOutcome, NativeWebViewHost};
