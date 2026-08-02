use crate::error::{AuthError, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// Configuration for the auth library.
/// Most fields can be populated from environment variables (matching the
/// official grok CLI where possible) or a config file.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Path to the shared auth.json (defaults to ~/.grok/auth.json)
    pub auth_file: PathBuf,

    /// Direct API key (XAI_API_KEY or GROK_CODE_XAI_API_KEY env)
    pub api_key: Option<String>,

    // OIDC / Enterprise
    pub oidc_issuer: Option<String>,
    pub oidc_client_id: Option<String>,
    pub oidc_scopes: Vec<String>,

    /// External auth provider command (the powerful escape hatch for other harnesses)
    pub auth_provider_command: Option<String>,
    pub auth_provider_label: Option<String>,

    /// Seconds before real expiry that we consider the token "stale" and refresh.
    /// Matches GROK_AUTH_EARLY_INVALIDATION_SECS (default 300).
    pub early_invalidation_buffer: chrono::Duration,

    /// Client identification sent to the backend (important for attribution
    /// and any future per-client quotas/features).
    pub client_version: String,

    /// Optional override for the chat / model base URL when you want to talk
    /// directly to the Grok backend from your harness.
    pub chat_proxy_base_url: Option<String>,

    /// Whether to allow opening a browser for interactive login.
    pub allow_browser: bool,
}

impl AuthConfig {
    /// Load from environment variables (best effort, matches official CLI names).
    pub fn from_env() -> Result<Self> {
        let mut cfg = Self::default();

        if let Ok(v) = std::env::var("XAI_API_KEY") {
            cfg.api_key = Some(v);
        } else if let Ok(v) = std::env::var("GROK_CODE_XAI_API_KEY") {
            cfg.api_key = Some(v);
        }

        if let Ok(v) = std::env::var("GROK_OIDC_ISSUER") {
            cfg.oidc_issuer = Some(v);
        }
        if let Ok(v) = std::env::var("GROK_OIDC_CLIENT_ID") {
            cfg.oidc_client_id = Some(v);
        }

        if let Ok(v) = std::env::var("GROK_AUTH_PROVIDER_COMMAND") {
            cfg.auth_provider_command = Some(v);
        }
        if let Ok(v) = std::env::var("GROK_AUTH_PROVIDER_LABEL") {
            cfg.auth_provider_label = Some(v);
        }

        if let Ok(v) = std::env::var("GROK_AUTH_EARLY_INVALIDATION_SECS") {
            if let Ok(secs) = v.parse::<i64>() {
                cfg.early_invalidation_buffer = chrono::Duration::seconds(secs);
            }
        }

        if let Ok(v) = std::env::var("GROK_CLI_CHAT_PROXY_BASE_URL") {
            cfg.chat_proxy_base_url = Some(v);
        }

        // The Grok CLI chat proxy rejects outdated/non-version-like values here.
        cfg.client_version = std::env::var("GROK_CLIENT_VERSION")
            .unwrap_or_else(|_| crate::DEFAULT_CLIENT_VERSION.to_string());

        cfg.allow_browser = std::env::var("GROK_NO_BROWSER").is_err();

        if cfg.auth_file.as_os_str().is_empty() {
            cfg.auth_file = super::default_auth_path();
        }

        if cfg.early_invalidation_buffer.num_seconds() == 0 {
            cfg.early_invalidation_buffer = chrono::Duration::seconds(300);
        }

        if cfg.oidc_scopes.is_empty() {
            cfg.oidc_scopes = vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
                "grok-cli:access".to_string(),
                "api:access".to_string(),
            ];
        }

        Ok(cfg)
    }

    /// Merge in values from a partial config (e.g. from a toml file in Mitsuro).
    pub fn merge_toml(&mut self, toml: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct Partial {
            #[serde(default)]
            grok_com_config: Option<GrokComConfig>,
            #[serde(default)]
            auth: Option<AuthSection>,
        }
        #[derive(Deserialize)]
        struct GrokComConfig {
            oidc: Option<OidcSection>,
        }
        #[derive(Deserialize)]
        struct OidcSection {
            issuer: Option<String>,
            client_id: Option<String>,
        }
        #[derive(Deserialize)]
        struct AuthSection {
            auth_provider_command: Option<String>,
            auth_provider_label: Option<String>,
        }

        let p: Partial = toml::from_str(toml)
            .map_err(|e| AuthError::Config(format!("failed to parse config snippet: {}", e)))?;

        if let Some(g) = p.grok_com_config {
            if let Some(o) = g.oidc {
                if let Some(iss) = o.issuer {
                    self.oidc_issuer = Some(iss);
                }
                if let Some(cid) = o.client_id {
                    self.oidc_client_id = Some(cid);
                }
            }
        }

        if let Some(a) = p.auth {
            if let Some(cmd) = a.auth_provider_command {
                self.auth_provider_command = Some(cmd);
            }
            if let Some(label) = a.auth_provider_label {
                self.auth_provider_label = Some(label);
            }
        }

        Ok(())
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            auth_file: super::default_auth_path(),
            api_key: None,
            oidc_issuer: Some(crate::DEFAULT_OIDC_ISSUER.to_string()),
            oidc_client_id: Some(crate::DEFAULT_OIDC_CLIENT_ID.to_string()),
            oidc_scopes: vec![
                "openid".into(),
                "profile".into(),
                "email".into(),
                "offline_access".into(),
                "grok-cli:access".into(),
                "api:access".into(),
            ],
            auth_provider_command: None,
            auth_provider_label: None,
            early_invalidation_buffer: chrono::Duration::seconds(300),
            client_version: crate::DEFAULT_CLIENT_VERSION.to_string(),
            chat_proxy_base_url: None,
            allow_browser: true,
        }
    }
}
