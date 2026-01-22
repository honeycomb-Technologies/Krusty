//! AI Client configuration
//!
//! Provider-agnostic configuration for AI API clients.

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
    /// Custom headers to send with requests (e.g., User-Agent for Kimi)
    pub custom_headers: HashMap<String, String>,
}

impl Default for AiClientConfig {
    fn default() -> Self {
        Self {
            model: constants::ai::DEFAULT_MODEL.to_string(),
            max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
            base_url: None,
            auth_header: AuthHeader::XApiKey,
            provider_id: ProviderId::Anthropic,
            api_format: ApiFormat::Anthropic,
            custom_headers: HashMap::new(),
        }
    }
}

impl AiClientConfig {
    /// Get the API URL to use
    ///
    /// For OpenCode Zen, routes to correct endpoint based on model's API format:
    /// - Anthropic format → /v1/messages
    /// - OpenAI format → /v1/chat/completions
    /// - OpenAI Responses → /v1/responses
    /// - Google format → /v1/models/{model}:streamGenerateContent
    pub fn api_url(&self) -> String {
        const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";

        if let Some(base) = &self.base_url {
            // For OpenCode Zen, modify the endpoint based on format
            if self.provider_id == ProviderId::OpenCodeZen {
                let base_without_endpoint = base
                    .trim_end_matches("/messages")
                    .trim_end_matches("/chat/completions")
                    .trim_end_matches("/responses");

                return match self.api_format {
                    ApiFormat::Anthropic => format!("{}/messages", base_without_endpoint),
                    ApiFormat::OpenAI => format!("{}/chat/completions", base_without_endpoint),
                    ApiFormat::OpenAIResponses => format!("{}/responses", base_without_endpoint),
                    ApiFormat::Google => format!(
                        "{}/models/{}:streamGenerateContent",
                        base_without_endpoint, self.model
                    ),
                };
            }
            base.clone()
        } else {
            DEFAULT_API_URL.to_string()
        }
    }

    /// Check if this config is for the native Anthropic API
    pub fn is_anthropic(&self) -> bool {
        self.provider_id == ProviderId::Anthropic
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

    /// Check if this provider uses Anthropic-compatible API
    ///
    /// All providers (Anthropic, OpenRouter, Z.ai, MiniMax, Kimi) use Anthropic Messages API
    /// Exception: OpenCode Zen routes some models to OpenAI or Google format
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
        use crate::ai::providers::{AuthHeader, ProviderConfig, ProviderId};
        use crate::auth::detect_openai_auth_type;

        let auth_type = detect_openai_auth_type(credentials);
        let base_url = ProviderConfig::openai_url_for_auth(auth_type);
        let api_format = ProviderConfig::openai_format_for_auth(auth_type);

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
            custom_headers: HashMap::new(),
        }
    }
}

use crate::ai::providers::ReasoningFormat;
use crate::ai::types::{
    AiTool, ContextManagement, ThinkingConfig, WebFetchConfig, WebSearchConfig,
};

/// Call options for API requests
#[derive(Debug, Clone)]
pub struct CallOptions {
    pub max_tokens: Option<usize>,
    pub temperature: Option<f32>,
    pub tools: Option<Vec<AiTool>>,
    pub system_prompt: Option<String>,
    /// Extended thinking configuration (Anthropic-style)
    pub thinking: Option<ThinkingConfig>,
    /// Universal reasoning format - determines how to encode reasoning in requests
    /// When Some, enables reasoning for the model using the appropriate format
    pub reasoning_format: Option<ReasoningFormat>,
    /// Enable prompt caching (default: true)
    pub enable_caching: bool,
    /// Context management for automatic clearing of old content
    pub context_management: Option<ContextManagement>,
    /// Web search configuration (server-executed)
    pub web_search: Option<WebSearchConfig>,
    /// Web fetch configuration (server-executed, beta)
    pub web_fetch: Option<WebFetchConfig>,
}

impl Default for CallOptions {
    fn default() -> Self {
        Self {
            max_tokens: None,
            temperature: None,
            tools: None,
            system_prompt: None,
            thinking: None,
            reasoning_format: None,
            enable_caching: true,
            context_management: None,
            web_search: None,
            web_fetch: None,
        }
    }
}

// Type alias for backwards compatibility during migration
#[deprecated(note = "Use AiClientConfig instead")]
pub type AnthropicConfig = AiClientConfig;
