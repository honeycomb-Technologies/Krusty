//! OAuth types and data structures

use serde::{Deserialize, Serialize};

use crate::ai::providers::ProviderId;

/// OAuth token data stored for a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenData {
    /// The access token used for API requests
    pub access_token: String,
    /// Optional refresh token for obtaining new access tokens
    pub refresh_token: Option<String>,
    /// Optional ID token (OpenID Connect)
    pub id_token: Option<String>,
    /// Unix timestamp when the token expires (if known)
    pub expires_at: Option<u64>,
    /// Unix timestamp of the last token refresh
    pub last_refresh: u64,
    /// Optional account ID from the OAuth provider
    pub account_id: Option<String>,
}

impl OAuthTokenData {
    /// Check if the token is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            now >= expires_at
        } else {
            false
        }
    }

    /// Check if the token needs refresh (older than specified days)
    pub fn needs_refresh(&self, refresh_days: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let refresh_threshold_secs = refresh_days * 24 * 60 * 60;
        now.saturating_sub(self.last_refresh) > refresh_threshold_secs
    }
}

/// OAuth configuration for a provider
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Provider this config belongs to
    pub provider_id: ProviderId,
    /// OAuth client ID
    pub client_id: String,
    /// Authorization endpoint URL
    pub authorization_url: String,
    /// Token endpoint URL
    pub token_url: String,
    /// Device authorization endpoint URL (for device code flow)
    pub device_auth_url: Option<String>,
    /// OAuth scopes to request
    pub scopes: Vec<String>,
    /// Days before token refresh is recommended
    pub refresh_days: u64,
    /// Extra query parameters to include in authorization URL (provider-specific)
    pub extra_auth_params: Vec<(String, String)>,
}

/// Authentication method supported by a provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Traditional API key authentication
    ApiKey,
    /// OAuth browser-based flow (opens browser, local redirect)
    OAuthBrowser,
    /// OAuth device code flow (for headless environments)
    OAuthDevice,
}

impl std::fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthMethod::ApiKey => write!(f, "API Key"),
            AuthMethod::OAuthBrowser => write!(f, "OAuth (Browser)"),
            AuthMethod::OAuthDevice => write!(f, "OAuth (Device Code)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_expiry() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token expiring in the future
        let token = OAuthTokenData {
            access_token: "test".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: Some(now + 3600),
            last_refresh: now,
            account_id: None,
        };
        assert!(!token.is_expired());

        // Token expired in the past
        let expired_token = OAuthTokenData {
            access_token: "test".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: Some(now - 3600),
            last_refresh: now - 7200,
            account_id: None,
        };
        assert!(expired_token.is_expired());

        // Token with no expiry
        let no_expiry = OAuthTokenData {
            access_token: "test".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            last_refresh: now,
            account_id: None,
        };
        assert!(!no_expiry.is_expired());
    }

    #[test]
    fn test_needs_refresh() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Token refreshed recently
        let fresh_token = OAuthTokenData {
            access_token: "test".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            last_refresh: now,
            account_id: None,
        };
        assert!(!fresh_token.needs_refresh(28));

        // Token refreshed 30 days ago (needs refresh with 28 day threshold)
        let old_token = OAuthTokenData {
            access_token: "test".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            last_refresh: now - (30 * 24 * 60 * 60),
            account_id: None,
        };
        assert!(old_token.needs_refresh(28));
    }
}
