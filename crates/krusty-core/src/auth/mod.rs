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

use anyhow::{Context, Result};
use serde::Deserialize;

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
            if token.refresh_token.is_some()
                && try_refresh_oauth_token_blocking(ProviderId::OpenAI).is_some()
            {
                return OpenAIAuthType::ChatGptOAuth;
            }
        }
    }

    OpenAIAuthType::None
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Refresh an expired OAuth token using the stored refresh token
pub async fn refresh_oauth_token(provider_id: ProviderId) -> Result<OAuthTokenData> {
    let oauth_store = OAuthTokenStore::load().context("Failed to load OAuth token store")?;
    let token = oauth_store
        .get(&provider_id)
        .context("No OAuth token stored for provider")?
        .clone();
    let refresh_token = token
        .refresh_token
        .as_ref()
        .context("No refresh token available")?;

    let config = openai_oauth_config();

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &config.client_id),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("Failed to send token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Token refresh failed ({}): {}", status, body);
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .context("Failed to parse token refresh response")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let refreshed = OAuthTokenData {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token.or(token.refresh_token.clone()),
        id_token: token_response.id_token.or(token.id_token),
        expires_at: token_response.expires_in.map(|secs| now + secs),
        last_refresh: now,
        account_id: token.account_id,
    };

    let mut store = OAuthTokenStore::load().context("Failed to reload OAuth token store")?;
    store.set(provider_id, refreshed.clone());
    store
        .save()
        .context("Failed to save refreshed OAuth token")?;

    tracing::info!("Successfully refreshed OAuth token for {}", provider_id);
    Ok(refreshed)
}

/// Sync wrapper for refreshing an OAuth token from non-async code paths
pub fn try_refresh_oauth_token_blocking(provider_id: ProviderId) -> Option<OAuthTokenData> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    tokio::task::block_in_place(|| handle.block_on(refresh_oauth_token(provider_id)).ok())
}
