use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The rich entry stored under each issuer::client key in auth.json.
/// This structure is designed to be (mostly) compatible with what the official
/// `grok` CLI writes so that `grok` and your harness (Krusty, etc.) can share
/// the same login session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthEntry {
    /// The actual access token (JWT or opaque). In the official file this lives
    /// under the "key" field.
    #[serde(rename = "key")]
    pub access_token: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>, // "oidc", "api_key", "external", ...

    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time: Option<DateTime<Utc>>,

    // Profile / principal info (preserved for full compatibility with grok CLI)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oidc_client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coding_data_retention_opt_out: Option<bool>,

    // Extra fields the official client may write (we round-trip them).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl AuthEntry {
    pub fn is_expired(&self, buffer: chrono::Duration) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() + buffer >= exp,
            None => false, // no expiry info → assume still good until a 401
        }
    }

    /// First few chars of the access token (for logging, never log the real token).
    pub fn key_prefix(&self) -> String {
        self.access_token.chars().take(8).collect()
    }
}

/// Lightweight view returned to callers.
#[derive(Debug, Clone)]
pub struct AuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub issuer_key: String, // the "https://auth.x.ai::xxxx" key
}

impl From<(String, AuthEntry)> for AuthToken {
    fn from((issuer_key, entry): (String, AuthEntry)) -> Self {
        Self {
            access_token: entry.access_token,
            refresh_token: entry.refresh_token,
            expires_at: entry.expires_at,
            issuer_key,
        }
    }
}
