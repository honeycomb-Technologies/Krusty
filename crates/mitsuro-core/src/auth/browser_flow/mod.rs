//! Browser-based OAuth flow
//!
//! Implements the authorization code flow with PKCE:
//! 1. Generate PKCE verifier and challenge
//! 2. Start local HTTP server for callback
//! 3. Open browser to authorization URL
//! 4. Wait for callback with authorization code
//! 5. Exchange code for tokens

mod callback_server;
mod flows;

pub use callback_server::{open_browser, run_callback_server, CallbackResult};
pub use flows::{BrowserOAuthFlow, PasteCodeOAuthFlow};

/// Default port for the local OAuth callback server (matches Codex CLI)
pub const DEFAULT_CALLBACK_PORT: u16 = 1455;
