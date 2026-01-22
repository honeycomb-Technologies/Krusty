//! Authentication for Krusty
//!
//! This module provides OAuth authentication support for providers that support it,
//! as well as the types and utilities needed for authentication flows.
//!
//! API key storage is handled by the credentials module in storage/

pub mod browser_flow;
pub mod device_flow;
pub mod pkce;
pub mod providers;
pub mod storage;
pub mod types;

// Re-exports for convenience
pub use browser_flow::{open_browser, BrowserOAuthFlow, DEFAULT_CALLBACK_PORT};
pub use device_flow::{DeviceCodeFlow, DeviceCodeResponse};
pub use pkce::{PkceChallenge, PkceVerifier};
pub use providers::openai_oauth_config;
pub use storage::OAuthTokenStore;
pub use types::{AuthMethod, OAuthConfig, OAuthTokenData};

use crate::ai::providers::ProviderId;
use crate::storage::CredentialStore;

/// Auth type for OpenAI - determines which API endpoint to use
///
/// ChatGPT OAuth tokens require the Responses API at chatgpt.com,
/// while API keys use the standard Chat Completions API at api.openai.com.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAIAuthType {
    /// Using OAuth token from ChatGPT - requires Responses API
    ChatGptOAuth,
    /// Using API key - uses Chat Completions API
    ApiKey,
    /// No authentication configured
    None,
}

/// Detect which type of OpenAI authentication is configured
///
/// Checks for API key first (takes precedence), then OAuth token.
/// Returns the auth type that determines which endpoint to use.
pub fn detect_openai_auth_type(credentials: &CredentialStore) -> OpenAIAuthType {
    // Check for API key first (takes precedence)
    if credentials.get(&ProviderId::OpenAI).is_some() {
        return OpenAIAuthType::ApiKey;
    }

    // Check for OAuth token
    if let Ok(oauth_store) = OAuthTokenStore::load() {
        if let Some(token) = oauth_store.get(&ProviderId::OpenAI) {
            if !token.is_expired() {
                return OpenAIAuthType::ChatGptOAuth;
            }
        }
    }

    OpenAIAuthType::None
}
