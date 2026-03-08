//! Authentication for Krusty
//!
//! This module provides OAuth authentication support for providers that support it,
//! as well as the types and utilities needed for authentication flows.
//!
//! API key storage is handled by the credentials module in storage/

pub mod browser_flow;
pub mod device_flow;
pub mod hosted_browser_flow;
pub mod openai_device_flow;
pub mod pkce;
pub mod providers;
pub mod storage;
pub mod types;

// Re-exports for convenience
pub use browser_flow::{
    open_browser, run_callback_server, BrowserOAuthFlow, CallbackResult, PasteCodeOAuthFlow,
    DEFAULT_CALLBACK_PORT,
};
pub use device_flow::{DeviceCodeFlow, DeviceCodeResponse};
pub use hosted_browser_flow::HostedBrowserOAuthFlow;
pub use openai_device_flow::{OpenAIDeviceAuthFlow, OpenAIDeviceCodeResponse};
pub use pkce::{PkceChallenge, PkceVerifier};
pub use providers::{anthropic_oauth_config, openai_oauth_config};
pub use storage::OAuthTokenStore;
pub use types::{AuthMethod, OAuthConfig, OAuthTokenData};

use anyhow::{Context, Result};
use base64::Engine;
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

/// OpenAI auth selection mode.
///
/// - `Auto`: Prefer OAuth for Codex models, otherwise prefer API key.
/// - `OAuth`: Require ChatGPT OAuth token.
/// - `ApiKey`: Require API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAIAuthMode {
    Auto,
    OAuth,
    ApiKey,
}

impl OpenAIAuthMode {
    /// Parse auth mode from `KRUSTY_OPENAI_AUTH_MODE`.
    ///
    /// Supported values: `auto`, `oauth`, `api_key`.
    pub fn from_env() -> Self {
        match std::env::var("KRUSTY_OPENAI_AUTH_MODE")
            .ok()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "oauth" => Self::OAuth,
            "api_key" => Self::ApiKey,
            _ => Self::Auto,
        }
    }
}

/// Resolved OpenAI auth information for a model.
#[derive(Debug, Clone)]
pub struct OpenAIAuthResolution {
    pub auth_type: OpenAIAuthType,
    pub credential: Option<String>,
    pub account_id: Option<String>,
}

/// Auth type for Anthropic - determines auth header format
///
/// OAuth tokens (sk-ant-oat*) use Bearer auth and require CC identity headers.
/// API keys (sk-ant-*) use x-api-key header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicAuthType {
    /// Using OAuth token from Claude - requires Bearer auth + CC identity
    OAuth,
    /// Using API key - uses x-api-key header
    ApiKey,
    /// No authentication configured
    None,
}

/// Resolved Anthropic auth information.
#[derive(Debug, Clone)]
pub struct AnthropicAuthResolution {
    pub auth_type: AnthropicAuthType,
    pub credential: Option<String>,
}

/// Check if a credential is an Anthropic OAuth token
pub fn is_anthropic_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat")
}

/// Resolve Anthropic auth type + credential.
///
/// Checks OAuth token store first, falls back to API key from credential store.
pub fn resolve_anthropic_auth(credentials: &CredentialStore) -> AnthropicAuthResolution {
    // Check OAuth token store first
    if let Some((access_token, _)) = load_anthropic_oauth_credential() {
        return AnthropicAuthResolution {
            auth_type: AnthropicAuthType::OAuth,
            credential: Some(access_token),
        };
    }

    // Fall back to API key
    if let Some(key) = credentials.get(&ProviderId::Anthropic).cloned() {
        return AnthropicAuthResolution {
            auth_type: AnthropicAuthType::ApiKey,
            credential: Some(key),
        };
    }

    AnthropicAuthResolution {
        auth_type: AnthropicAuthType::None,
        credential: None,
    }
}

/// Resolve OpenAI auth type + credential for a specific model.
pub fn resolve_openai_auth(credentials: &CredentialStore, model: &str) -> OpenAIAuthResolution {
    let mode = OpenAIAuthMode::from_env();
    let is_codex_model = model.to_ascii_lowercase().contains("codex");

    let api_key = credentials.get(&ProviderId::OpenAI).cloned();
    let oauth = load_openai_oauth_credential();

    match mode {
        OpenAIAuthMode::OAuth => {
            if let Some((access_token, account_id)) = oauth {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ChatGptOAuth,
                    credential: Some(access_token),
                    account_id,
                }
            } else {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::None,
                    credential: None,
                    account_id: None,
                }
            }
        }
        OpenAIAuthMode::ApiKey => {
            if let Some(key) = api_key {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ApiKey,
                    credential: Some(key),
                    account_id: None,
                }
            } else {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::None,
                    credential: None,
                    account_id: None,
                }
            }
        }
        OpenAIAuthMode::Auto => {
            // Codex performs best with ChatGPT OAuth path when available.
            if is_codex_model {
                if let Some((access_token, account_id)) = oauth.clone() {
                    return OpenAIAuthResolution {
                        auth_type: OpenAIAuthType::ChatGptOAuth,
                        credential: Some(access_token),
                        account_id,
                    };
                }
                if let Some(key) = api_key {
                    return OpenAIAuthResolution {
                        auth_type: OpenAIAuthType::ApiKey,
                        credential: Some(key),
                        account_id: None,
                    };
                }
            } else if let Some(key) = api_key {
                return OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ApiKey,
                    credential: Some(key),
                    account_id: None,
                };
            }

            if let Some((access_token, account_id)) = oauth {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ChatGptOAuth,
                    credential: Some(access_token),
                    account_id,
                }
            } else {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::None,
                    credential: None,
                    account_id: None,
                }
            }
        }
    }
}

/// Extract ChatGPT account id from OpenAI JWT-like tokens.
///
/// Expected claim path:
/// `https://api.openai.com/auth.chatgpt_account_id`
pub fn extract_openai_account_id(token: &str) -> Option<String> {
    let payload = decode_jwt_payload(token)?;
    let auth_obj = payload.get("https://api.openai.com/auth")?;
    let account_id = auth_obj.get("chatgpt_account_id")?.as_str()?;
    if account_id.is_empty() {
        None
    } else {
        Some(account_id.to_string())
    }
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _signature = parts.next()?;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| {
            // JWT payload can include optional padding depending on issuer/runtime.
            let mut padded = payload_b64.to_string();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            base64::engine::general_purpose::URL_SAFE.decode(padded)
        })
        .ok()?;

    serde_json::from_slice::<serde_json::Value>(&decoded).ok()
}

fn load_openai_oauth_credential() -> Option<(String, Option<String>)> {
    let oauth_store = OAuthTokenStore::load().ok()?;
    let token = oauth_store.get(&ProviderId::OpenAI)?;

    if token.is_expired() {
        if token.refresh_token.is_some() {
            let refreshed = try_refresh_oauth_token_blocking(ProviderId::OpenAI)?;
            let account_id = refreshed
                .account_id
                .clone()
                .or_else(|| extract_openai_account_id(&refreshed.access_token))
                .or_else(|| {
                    refreshed
                        .id_token
                        .as_deref()
                        .and_then(extract_openai_account_id)
                });
            return Some((refreshed.access_token, account_id));
        }
        return None;
    }

    let account_id = token
        .account_id
        .clone()
        .or_else(|| extract_openai_account_id(&token.access_token))
        .or_else(|| {
            token
                .id_token
                .as_deref()
                .and_then(extract_openai_account_id)
        });
    Some((token.access_token.clone(), account_id))
}

fn load_anthropic_oauth_credential() -> Option<(String, Option<String>)> {
    let oauth_store = OAuthTokenStore::load().ok()?;
    let token = oauth_store.get(&ProviderId::Anthropic)?;

    if token.is_expired() {
        if token.refresh_token.is_some() {
            let refreshed = try_refresh_oauth_token_blocking(ProviderId::Anthropic)?;
            return Some((refreshed.access_token, refreshed.account_id));
        }
        return None;
    }

    Some((token.access_token.clone(), token.account_id.clone()))
}

/// Detect which type of OpenAI authentication is configured.
///
/// Uses `resolve_openai_auth` with codex-aware defaults.
pub fn detect_openai_auth_type(credentials: &CredentialStore) -> OpenAIAuthType {
    resolve_openai_auth(credentials, "gpt-5.3-codex").auth_type
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
    // Dispatch to provider-specific refresh
    match provider_id {
        ProviderId::Anthropic => refresh_anthropic_oauth_token().await,
        _ => refresh_openai_oauth_token(provider_id).await,
    }
}

/// Refresh an OpenAI OAuth token
async fn refresh_openai_oauth_token(provider_id: ProviderId) -> Result<OAuthTokenData> {
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

    let account_id = extract_openai_account_id(&token_response.access_token)
        .or_else(|| {
            token_response
                .id_token
                .as_deref()
                .and_then(extract_openai_account_id)
        })
        .or(token.account_id.clone());

    let refreshed = OAuthTokenData {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token.or(token.refresh_token.clone()),
        id_token: token_response.id_token.or(token.id_token),
        expires_at: token_response.expires_in.map(|secs| now + secs),
        last_refresh: now,
        account_id,
    };

    let mut store = OAuthTokenStore::load().context("Failed to reload OAuth token store")?;
    store.set(provider_id, refreshed.clone());
    store
        .save()
        .context("Failed to save refreshed OAuth token")?;

    tracing::info!("Successfully refreshed OAuth token for {}", provider_id);
    Ok(refreshed)
}

/// Refresh an Anthropic OAuth token
///
/// Anthropic uses JSON body (not form-encoded) for token requests.
async fn refresh_anthropic_oauth_token() -> Result<OAuthTokenData> {
    let oauth_store = OAuthTokenStore::load().context("Failed to load OAuth token store")?;
    let token = oauth_store
        .get(&ProviderId::Anthropic)
        .context("No OAuth token stored for Anthropic")?
        .clone();
    let refresh_token = token
        .refresh_token
        .as_ref()
        .context("No refresh token available")?;

    let config = anthropic_oauth_config();

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": config.client_id,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .context("Failed to send Anthropic token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic token refresh failed ({}): {}", status, body);
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .context("Failed to parse Anthropic token refresh response")?;

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
        account_id: token.account_id.clone(),
    };

    let mut store = OAuthTokenStore::load().context("Failed to reload OAuth token store")?;
    store.set(ProviderId::Anthropic, refreshed.clone());
    store
        .save()
        .context("Failed to save refreshed Anthropic OAuth token")?;

    tracing::info!("Successfully refreshed Anthropic OAuth token");
    Ok(refreshed)
}

/// Sync wrapper for refreshing an OAuth token from non-async code paths
pub fn try_refresh_oauth_token_blocking(provider_id: ProviderId) -> Option<OAuthTokenData> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    tokio::task::block_in_place(|| handle.block_on(refresh_oauth_token(provider_id)).ok())
}
