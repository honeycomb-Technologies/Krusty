//! AI Client configuration
//!
//! Provider-agnostic configuration for AI API clients.

use std::collections::HashMap;

use crate::ai::models::resolve_model_metadata;
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
        use crate::ai::providers::{AuthHeader, ProviderConfig, ProviderId};
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
}

impl AiClientConfig {
    /// Create config for Anthropic with automatic auth type detection
    ///
    /// Detects whether OAuth token or API key is being used:
    /// - OAuth (sk-ant-oat*): Bearer auth + CC identity headers
    /// - API Key (sk-ant-*): x-api-key auth
    pub fn for_anthropic_with_auth_detection(
        model: &str,
        credentials: &crate::storage::CredentialStore,
    ) -> Self {
        use crate::ai::providers::{ProviderConfig, ProviderId};
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
            // CC identity headers for OAuth path
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

/// Anthropic adaptive effort for Opus 4.6 thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicAdaptiveEffort {
    Low,
    Medium,
    High,
    Max,
}

impl AnthropicAdaptiveEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

use crate::ai::providers::ReasoningFormat;
use crate::ai::types::{
    AiTool, ContextManagement, ThinkingConfig, WebFetchConfig, WebSearchConfig,
};

/// Codex reasoning effort controls for OpenAI Responses API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl CodexReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

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
    /// Session-scoped identifier for provider-level caching (Codex prompt cache key)
    pub session_id: Option<String>,
    /// Codex-specific reasoning effort
    pub codex_reasoning_effort: Option<CodexReasoningEffort>,
    /// Codex tool parallelism toggle (disabled by default until parser hardening)
    pub codex_parallel_tool_calls: bool,
    /// Anthropic Opus 4.6 adaptive thinking effort
    pub anthropic_adaptive_effort: Option<AnthropicAdaptiveEffort>,
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
            session_id: None,
            codex_reasoning_effort: None,
            codex_parallel_tool_calls: false,
            anthropic_adaptive_effort: None,
        }
    }
}

impl CallOptions {
    /// Build a canonicalized options set for a specific provider/model pipeline.
    ///
    /// This prevents per-surface drift by enforcing provider capabilities and
    /// model reasoning constraints before request construction.
    pub fn canonicalized_for(
        &self,
        provider: ProviderId,
        model: &str,
        api_format: ApiFormat,
    ) -> Self {
        let caps = crate::ai::providers::ProviderCapabilities::for_provider(provider);
        let inferred = resolve_model_metadata(provider, model, api_format);
        let mut options = self.clone();

        // Keep reasoning fields aligned with model capability.
        if inferred.reasoning_format.is_none() {
            options.reasoning_format = None;
            options.thinking = None;
            options.codex_reasoning_effort = None;
            options.anthropic_adaptive_effort = None;
        } else {
            if options.reasoning_format != inferred.reasoning_format {
                options.reasoning_format = inferred.reasoning_format;
            }

            match options.reasoning_format {
                Some(crate::ai::providers::ReasoningFormat::OpenAI) => {
                    options.anthropic_adaptive_effort = None;
                }
                Some(crate::ai::providers::ReasoningFormat::Anthropic) => {
                    options.codex_reasoning_effort = None;
                }
                _ => {
                    options.codex_reasoning_effort = None;
                    options.anthropic_adaptive_effort = None;
                }
            }
        }

        if !caps.context_management {
            options.context_management = None;
        }
        if !caps.web_search {
            options.web_search = None;
        }
        if !caps.web_fetch {
            options.web_fetch = None;
        }

        // Codex parallel tool call flag applies only to OpenAI Responses pipeline.
        if !(provider == ProviderId::OpenAI && matches!(api_format, ApiFormat::OpenAIResponses)) {
            options.codex_parallel_tool_calls = false;
        }

        options
    }
}

#[cfg(test)]
mod tests {
    use super::{CallOptions, CodexReasoningEffort};
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::{ProviderId, ReasoningFormat};
    use crate::ai::types::{ContextManagement, ThinkingConfig, WebFetchConfig, WebSearchConfig};

    #[test]
    fn canonicalization_drops_unsupported_provider_features() {
        let options = CallOptions {
            context_management: Some(ContextManagement::default_tools_only()),
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            codex_parallel_tool_calls: true,
            ..Default::default()
        };

        let canonical =
            options.canonicalized_for(ProviderId::MiniMax, "MiniMax-M2.5", ApiFormat::Anthropic);

        assert!(canonical.context_management.is_none());
        assert!(canonical.web_search.is_none());
        assert!(canonical.web_fetch.is_none());
        assert!(!canonical.codex_parallel_tool_calls);
    }

    #[test]
    fn canonicalization_aligns_reasoning_controls_with_model_family() {
        let options = CallOptions {
            reasoning_format: Some(ReasoningFormat::OpenAI),
            codex_reasoning_effort: Some(CodexReasoningEffort::High),
            ..Default::default()
        };

        let canonical = options.canonicalized_for(
            ProviderId::Anthropic,
            "claude-opus-4.5",
            ApiFormat::Anthropic,
        );

        assert_eq!(canonical.reasoning_format, Some(ReasoningFormat::Anthropic));
        assert!(canonical.codex_reasoning_effort.is_none());
    }

    #[test]
    fn canonicalization_preserves_builtin_reasoning_models() {
        let options = CallOptions {
            thinking: Some(ThinkingConfig::default()),
            ..Default::default()
        };

        let canonical =
            options.canonicalized_for(ProviderId::MiniMax, "MiniMax-M2.5", ApiFormat::Anthropic);

        assert_eq!(canonical.reasoning_format, Some(ReasoningFormat::Anthropic));
        assert!(canonical.thinking.is_some());
    }
}
