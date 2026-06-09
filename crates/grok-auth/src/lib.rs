//! Modular xAI / Grok authentication library.
//!
//! Designed to be dropped into other Rust harnesses (Krusty, custom agents, etc.)
//! so you can share login state with the official `grok` CLI and use Grok models
//! / agentic capabilities outside of the Grok Build TUI.
//!
//! # Key goals
//! - Full login flows matching the official client (browser OIDC PKCE, device code, external provider, API key).
//! - **Robust caching**: in-memory hot cache + on-disk `~/.grok/auth.json` (exact format for interoperability),
//!   advisory file locking for multi-process safety (grok CLI + Krusty + other harnesses),
//!   atomic writes, proactive refresh with configurable buffer.
//! - Easy to get an authenticated `reqwest::Client` with the correct `Authorization: Bearer ...`
//!   and `x-grok-client-*` headers the backend expects.
//! - Non-interactive friendly (external providers, device code, direct key).
//!
//! # Caching strategy (important)
//! - `AuthStore` owns the on-disk state (the source of truth for sharing).
//! - A `tokio::sync::RwLock` protects an in-memory `CurrentToken` (hot path, no disk on every call).
//! - `ensure_fresh()` is called before returning a token / building a request.
//!   It checks `expires_at - buffer`. If needed, it attempts refresh using `refresh_token`.
//!   If refresh fails (or no refresh token), it can trigger re-auth (interactive or via external provider).
//! - File locking (via `fs2`) is taken briefly during load/save to prevent torn reads/writes
//!   when `grok` CLI and your harness run at the same time.
//! - Writes are done to `auth.json.tmp` then renamed (atomic on POSIX).
//! - A small "key_prefix" is kept for logs (never log full tokens).
//!
//! See `examples/krusty_auth.rs` and the README for integration patterns.

pub mod client;
pub mod config;
pub mod error;
pub mod login;
pub mod oidc;
pub mod store;
pub mod token;

pub use client::{AuthenticatedClient, ClientBuilder};
pub use config::AuthConfig;
pub use error::{AuthError, Result};
pub use login::run_interactive_login as authenticate;
pub use store::AuthStore;
pub use token::{AuthEntry, AuthToken};

use std::path::PathBuf;

/// Default location of the shared auth file (same as official `grok` CLI).
pub fn default_auth_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
        .join("auth.json")
}

/// Default OIDC issuer used by the official Grok CLI.
pub const DEFAULT_OIDC_ISSUER: &str = "https://auth.x.ai";

/// Public OIDC client ID used by the official Grok CLI.
pub const DEFAULT_OIDC_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// Recommended default client version header value.
///
/// The Grok CLI chat proxy currently rejects versions older than 0.1.202, and
/// expects this header to be a CLI-compatible version string rather than an
/// arbitrary product name. Identify your harness with an additional header if
/// needed, but keep `x-grok-client-version` semver-like.
pub const DEFAULT_CLIENT_VERSION: &str = "0.2.33";

/// Convenience entry point: load (or obtain) credentials and return a ready-to-use
/// authenticated HTTP client.
///
/// This is the main thing most harnesses will call:
/// ```ignore
/// let client = grok_auth::authenticated_client(AuthConfig::from_env()?).await?;
/// let resp = client.post("https://...").json(&body).send().await?;
/// ```
pub async fn authenticated_client(config: AuthConfig) -> Result<AuthenticatedClient> {
    let store = AuthStore::new(config.auth_file.clone(), config.clone()).await?;
    let token = store.ensure_fresh().await?;
    ClientBuilder::new()
        .with_token(token)
        .with_client_version(&config.client_version)
        .build()
}
