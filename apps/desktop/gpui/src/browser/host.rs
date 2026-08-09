//! `BrowserHost` trait plus external-bridge / wry-linked URL history.

use crate::app::BrowserSessionStatus;

/// Contract for Atlas URL navigation (external or native WebView backend).
pub trait BrowserHost {
    fn navigate(&mut self, url: &str);
    fn go_back(&mut self) -> bool;
    fn go_forward(&mut self) -> bool;
    #[allow(dead_code)] // agent / toolbar reload hook
    fn reload(&mut self);
    fn url(&self) -> &str;
    fn title(&self) -> &str;
    /// Honest page-state summary. This host never fabricates remote page content.
    fn page_body(&self) -> &str;
    fn status(&self) -> BrowserSessionStatus;
    #[allow(dead_code)] // agent session status transitions
    fn set_status(&mut self, status: BrowserSessionStatus);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    /// Human-readable backend label for chips / status.
    fn host_kind(&self) -> &'static str;
    /// Optional WebKit / engine version when native is linked.
    fn engine_version(&self) -> Option<&str>;
}

/// One history entry. `body` describes the bridge state, not remote page content.
#[derive(Clone, Debug)]
pub struct PageEntry {
    pub url: String,
    pub title: String,
    pub body: String,
}

/// Backend kind for the active host.
#[derive(Clone, Debug)]
#[allow(dead_code)] // inspected via host_kind / tests
pub enum HostBackend {
    /// External browser bridge — no embedded renderer linked.
    Mock,
    /// wry compiled + WebKitGTK available; embedding still depends on the native bridge.
    #[cfg(feature = "browser-native")]
    WryLinked { webkit_version: Option<String> },
}

/// Desktop Atlas host: local URL history plus optional wry linkage.
#[derive(Clone, Debug)]
pub struct DesktopBrowserHost {
    history: Vec<PageEntry>,
    index: usize,
    status: BrowserSessionStatus,
    backend: HostBackend,
}

/// Alias kept for call sites / docs that say “MockBrowserHost”.
#[allow(dead_code)]
pub type MockBrowserHost = DesktopBrowserHost;

impl DesktopBrowserHost {
    /// External-bridge host (used when `browser-native` is off).
    #[cfg_attr(feature = "browser-native", allow(dead_code))]
    pub fn new_mock() -> Self {
        Self::with_backend(HostBackend::Mock, BrowserSessionStatus::NoNativeHost)
    }

    #[cfg(feature = "browser-native")]
    pub fn new_wry_linked(webkit_version: Option<String>) -> Self {
        // Native library is linked; session is idle until agent drives or user navigates.
        Self::with_backend(
            HostBackend::WryLinked { webkit_version },
            BrowserSessionStatus::Idle,
        )
    }

    fn with_backend(backend: HostBackend, status: BrowserSessionStatus) -> Self {
        let initial = page_for_url("about:blank");
        Self {
            history: vec![initial],
            index: 0,
            status,
            backend,
        }
    }

    #[allow(dead_code)]
    pub fn backend(&self) -> &HostBackend {
        &self.backend
    }

    #[allow(dead_code)] // tests + future history chrome
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    #[allow(dead_code)]
    pub fn history_index(&self) -> usize {
        self.index
    }

    fn current(&self) -> &PageEntry {
        &self.history[self.index]
    }
}

impl BrowserHost for DesktopBrowserHost {
    fn navigate(&mut self, url: &str) {
        let normalized = normalize_url(url);
        if normalized.is_empty() {
            return;
        }
        // Drop any forward entries, then push.
        self.history.truncate(self.index + 1);
        self.history.push(page_for_url(&normalized));
        self.index = self.history.len() - 1;
        // Keep explicit NoNativeHost for an external-only build; otherwise Ready after a nav.
        match &self.backend {
            HostBackend::Mock => {
                self.status = BrowserSessionStatus::NoNativeHost;
            }
            #[cfg(feature = "browser-native")]
            HostBackend::WryLinked { .. } => {
                self.status = BrowserSessionStatus::Ready;
            }
        }
    }

    fn go_back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.index -= 1;
        true
    }

    fn go_forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.index += 1;
        true
    }

    fn reload(&mut self) {
        let url = self.url().to_string();
        let entry = page_for_url(&url);
        self.history[self.index] = entry;
    }

    fn url(&self) -> &str {
        &self.current().url
    }

    fn title(&self) -> &str {
        &self.current().title
    }

    fn page_body(&self) -> &str {
        &self.current().body
    }

    fn status(&self) -> BrowserSessionStatus {
        self.status
    }

    fn set_status(&mut self, status: BrowserSessionStatus) {
        self.status = status;
    }

    fn can_go_back(&self) -> bool {
        self.index > 0
    }

    fn can_go_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }

    fn host_kind(&self) -> &'static str {
        match &self.backend {
            HostBackend::Mock => "System browser (external)",
            #[cfg(feature = "browser-native")]
            HostBackend::WryLinked { .. } => "wry/WebKitGTK (linked bridge)",
        }
    }

    fn engine_version(&self) -> Option<&str> {
        match &self.backend {
            HostBackend::Mock => None,
            #[cfg(feature = "browser-native")]
            HostBackend::WryLinked { webkit_version } => webkit_version.as_deref(),
        }
    }
}

/// Build the default host for this build (wry-linked when feature is on).
pub fn create_default_host() -> DesktopBrowserHost {
    #[cfg(feature = "browser-native")]
    {
        return crate::browser::native::create_wry_linked_host();
    }
    #[cfg(not(feature = "browser-native"))]
    {
        DesktopBrowserHost::new_mock()
    }
}

fn normalize_url(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.contains("://") {
        t.to_string()
    } else if t.starts_with("localhost") || t.starts_with("127.0.0.1") {
        format!("http://{t}")
    } else {
        format!("https://{t}")
    }
}

fn page_for_url(url: &str) -> PageEntry {
    let title = title_for_url(url);
    let body = body_for_url(url);
    PageEntry {
        url: url.to_string(),
        title,
        body,
    }
}

fn title_for_url(url: &str) -> String {
    if url == "about:blank" {
        return "New page".into();
    }
    if url.contains("mitsuro.local") {
        return "Mitsuro Atlas".into();
    }
    // Prefer host + short path for the tab title.
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = without_scheme.split('/').next().unwrap_or(without_scheme);
    let path = without_scheme
        .find('/')
        .map(|i| &without_scheme[i..])
        .unwrap_or("");
    if path.is_empty() || path == "/" {
        host.to_string()
    } else {
        let short: String = path.chars().take(32).collect();
        format!("{host}{short}")
    }
}

fn body_for_url(url: &str) -> String {
    if url == "about:blank" {
        return "No page selected.".into();
    }
    format!("URL stored locally: {url}. Remote page content is not loaded inside Mitsuro.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_navigate_back_forward() {
        let mut h = DesktopBrowserHost::new_mock();
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
        assert_eq!(h.url(), "about:blank");

        h.navigate("example.com");
        assert!(h.url().starts_with("https://example.com"));
        assert!(h.can_go_back());
        assert!(!h.can_go_forward());

        h.navigate("https://mitsuro.local/docs");
        assert!(h.can_go_back());
        assert_eq!(h.history_len(), 3);

        assert!(h.go_back());
        assert!(h.url().contains("example.com"));
        assert!(h.can_go_forward());

        assert!(h.go_forward());
        assert!(h.url().contains("mitsuro.local/docs"));

        // Navigate after back truncates forward stack.
        assert!(h.go_back());
        h.navigate("https://other.test/");
        assert!(!h.can_go_forward());
        assert!(h.url().contains("other.test"));
    }

    #[test]
    fn normalize_adds_scheme() {
        let mut h = DesktopBrowserHost::new_mock();
        h.navigate("  localhost:3000/app  ");
        assert_eq!(h.url(), "http://localhost:3000/app");
    }
}
