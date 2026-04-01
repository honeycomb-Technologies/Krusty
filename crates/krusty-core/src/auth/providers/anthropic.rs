//! Anthropic OAuth configuration
//!
//! Anthropic uses OAuth 2.0 with PKCE for CLI applications.
//! Uses a paste-code flow (no localhost redirect) where the user
//! completes authorization in the browser and pastes back the code.
//!
//! Uses Claude Code's OAuth client ID for compatibility with
//! Claude Pro/Max subscriptions.

use crate::ai::providers::ProviderId;
use crate::auth::types::OAuthConfig;

/// Claude Code's public OAuth client ID
pub const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Get the OAuth configuration for Anthropic
///
/// Uses Claude Code's OAuth client with paste-code flow.
/// Anthropic does not support localhost redirects or device code flow.
pub fn anthropic_oauth_config() -> OAuthConfig {
    OAuthConfig {
        provider_id: ProviderId::Anthropic,
        client_id: ANTHROPIC_CLIENT_ID.to_string(),
        // Anthropic OAuth endpoints (confirmed working via pi-mono reference)
        authorization_url: "https://claude.ai/oauth/authorize".to_string(),
        token_url: "https://console.anthropic.com/v1/oauth/token".to_string(),
        // Anthropic doesn't support device code flow
        device_auth_url: None,
        // Anthropic OAuth scopes (matches pi-mono working implementation)
        scopes: vec![
            "org:create_api_key".to_string(),
            "user:profile".to_string(),
            "user:inference".to_string(),
        ],
        // Refresh tokens after 28 days
        refresh_days: 28,
        // Anthropic-specific: use code callback redirect and S256 challenge
        extra_auth_params: vec![
            ("code".to_string(), "true".to_string()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_config() {
        let config = anthropic_oauth_config();
        assert_eq!(config.provider_id, ProviderId::Anthropic);
        assert!(!config.client_id.is_empty());
        assert!(config.authorization_url.starts_with("https://"));
        assert!(config.token_url.starts_with("https://"));
        assert!(config.device_auth_url.is_none());
        assert!(!config.scopes.is_empty());
    }
}
