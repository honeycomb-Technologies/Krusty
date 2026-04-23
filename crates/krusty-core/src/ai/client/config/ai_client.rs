use std::collections::HashMap;

use crate::ai::models::ApiFormat;
use crate::ai::providers::{AuthHeader, ProviderId};
use crate::constants;

/// Configuration for the AI client
#[derive(Debug, Clone)]
pub struct AiClientConfig {
    /// Model ID to use for API calls
    pub model: String,
    /// Maximum output tokens
    pub max_tokens: usize,
    /// Optional base URL override (defaults to provider default)
    pub base_url: Option<String>,
    /// How to send authentication header
    pub auth_header: AuthHeader,
    /// Which provider this config is for
    pub provider_id: ProviderId,
    /// API format for this model (Anthropic, OpenAI, Google)
    pub api_format: ApiFormat,
    /// Custom headers to send with requests
    pub custom_headers: HashMap<String, String>,
}

impl Default for AiClientConfig {
    fn default() -> Self {
        Self {
            model: constants::ai::DEFAULT_MODEL.to_string(),
            max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
            base_url: None,
            auth_header: AuthHeader::XApiKey,
            provider_id: ProviderId::MiniMax,
            api_format: ApiFormat::Anthropic,
            custom_headers: HashMap::new(),
        }
    }
}

impl AiClientConfig {
    /// Get the API URL to use
    pub fn api_url(&self) -> String {
        const DEFAULT_API_URL: &str = "https://api.minimax.io/anthropic/v1/messages";

        if let Some(base) = &self.base_url {
            base.clone()
        } else {
            DEFAULT_API_URL.to_string()
        }
    }

    /// Get the provider ID
    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    /// Check if this config uses OpenAI chat/completions format
    pub fn uses_openai_format(&self) -> bool {
        matches!(
            self.api_format,
            ApiFormat::OpenAI | ApiFormat::OpenAIResponses
        )
    }

    /// Check if this config uses Google/Gemini format
    pub fn uses_google_format(&self) -> bool {
        matches!(self.api_format, ApiFormat::Google)
    }

    /// Check if this config uses ChatGPT Codex (Responses API) format
    pub fn uses_chatgpt_codex_format(&self) -> bool {
        matches!(self.api_format, ApiFormat::OpenAIResponses)
    }

    /// Check if this provider uses Anthropic-compatible API
    ///
    /// All providers (OpenRouter, Z.ai, MiniMax) use Anthropic Messages API
    /// Exception: OpenAI uses its own format
    pub fn uses_anthropic_api(&self) -> bool {
        !self.uses_openai_format() && !self.uses_google_format()
    }

    /// Create config for OpenAI with automatic auth type detection
    ///
    /// Detects whether OAuth token or API key is being used and routes to
    /// the correct endpoint:
    /// - OAuth (ChatGPT): chatgpt.com/backend-api/codex/v1/responses (Responses API)
    /// - API Key: api.openai.com/v1/chat/completions (Chat Completions API)
    pub fn for_openai_with_auth_detection(
        model: &str,
        credentials: &crate::storage::CredentialStore,
    ) -> Self {
        use crate::ai::providers::ProviderConfig;
        use crate::auth::resolve_openai_auth;

        let auth_resolution = resolve_openai_auth(credentials, model);
        let auth_type = auth_resolution.auth_type;
        let base_url = ProviderConfig::openai_url_for_auth(model, auth_type);
        let api_format = ProviderConfig::openai_format_for_auth(model, auth_type);

        tracing::info!(
            "OpenAI auth detection: {:?} -> {} (format: {:?})",
            auth_type,
            base_url,
            api_format
        );

        Self {
            model: model.to_string(),
            max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
            base_url: Some(base_url.to_string()),
            auth_header: AuthHeader::Bearer,
            provider_id: ProviderId::OpenAI,
            api_format,
            custom_headers: {
                let mut headers = HashMap::new();
                if matches!(auth_type, crate::auth::OpenAIAuthType::ChatGptOAuth) {
                    if let Some(account_id) = auth_resolution.account_id {
                        headers.insert("ChatGPT-Account-Id".to_string(), account_id);
                    }
                }
                headers
            },
        }
    }

    /// Create config for Anthropic with automatic auth type detection
    ///
    /// Detects whether OAuth token or API key is being used:
    /// - OAuth (sk-ant-oat*): Bearer auth + CC identity headers
    /// - API Key (sk-ant-*): x-api-key auth
    pub fn for_anthropic_with_auth_detection(
        model: &str,
        credentials: &crate::storage::CredentialStore,
    ) -> Self {
        use crate::ai::providers::ProviderConfig;
        use crate::auth::resolve_anthropic_auth;

        let auth_resolution = resolve_anthropic_auth(credentials);
        let auth_type = auth_resolution.auth_type;
        let auth_header = ProviderConfig::anthropic_auth_header_for_auth(auth_type);

        tracing::info!(
            "Anthropic auth detection: {:?} -> auth_header={:?}",
            auth_type,
            auth_header,
        );

        let mut custom_headers = HashMap::new();
        if matches!(auth_type, crate::auth::AnthropicAuthType::OAuth) {
            custom_headers.insert(
                "user-agent".to_string(),
                "claude-cli/2.1.2 (external, cli)".to_string(),
            );
            custom_headers.insert("x-app".to_string(), "cli".to_string());
        }

        Self {
            model: model.to_string(),
            max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
            base_url: Some("https://api.anthropic.com/v1/messages".to_string()),
            auth_header,
            provider_id: ProviderId::Anthropic,
            api_format: ApiFormat::Anthropic,
            custom_headers,
        }
    }
}
