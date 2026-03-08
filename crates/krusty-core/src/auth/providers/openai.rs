//! OpenAI OAuth configuration
//!
//! OpenAI uses standard OAuth 2.0 with PKCE for CLI applications.
//! Supports both browser flow and device code flow.
//!
//! Uses the same OAuth client as OpenAI's Codex CLI for compatibility.

use crate::ai::providers::ProviderId;
use crate::auth::types::OAuthConfig;

/// OpenAI's public OAuth client ID (used by Codex CLI)
pub const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Default first-party originator used by the Codex CLI login flow.
pub const OPENAI_OAUTH_ORIGINATOR: &str = "codex_cli_rs";

/// Get the OAuth configuration for OpenAI
///
/// Uses the same OAuth client as OpenAI's Codex CLI for compatibility
/// with ChatGPT Plus/Pro subscriptions.
pub fn openai_oauth_config() -> OAuthConfig {
    OAuthConfig {
        provider_id: ProviderId::OpenAI,
        // Use OpenAI's public Codex CLI client ID
        client_id: OPENAI_CLIENT_ID.to_string(),
        // OpenAI's OAuth endpoints
        authorization_url: "https://auth.openai.com/oauth/authorize".to_string(),
        token_url: "https://auth.openai.com/oauth/token".to_string(),
        // Device code endpoint for headless environments
        device_auth_url: Some("https://auth.openai.com/device/code".to_string()),
        // OpenID Connect scopes
        scopes: vec![
            "openid".to_string(),
            "profile".to_string(),
            "email".to_string(),
            "offline_access".to_string(), // For refresh tokens
        ],
        // Refresh tokens after 28 days
        refresh_days: 28,
        // OpenAI-specific parameters used by Codex CLI
        extra_auth_params: vec![
            ("id_token_add_organizations".to_string(), "true".to_string()),
            ("codex_cli_simplified_flow".to_string(), "true".to_string()),
            (
                "originator".to_string(),
                OPENAI_OAUTH_ORIGINATOR.to_string(),
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_config() {
        let config = openai_oauth_config();
        assert_eq!(config.provider_id, ProviderId::OpenAI);
        assert!(!config.client_id.is_empty());
        assert!(config.authorization_url.starts_with("https://"));
        assert!(config.token_url.starts_with("https://"));
        assert!(config.device_auth_url.is_some());
        assert!(!config.scopes.is_empty());
    }
}
