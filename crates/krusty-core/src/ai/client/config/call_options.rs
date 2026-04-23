use crate::ai::models::{resolve_model_metadata, ApiFormat};
use crate::ai::providers::{ProviderCapabilities, ProviderId, ReasoningFormat};
use crate::ai::types::{
    AiTool, ContextManagement, ThinkingConfig, WebFetchConfig, WebSearchConfig,
};

use super::effort::{AnthropicAdaptiveEffort, CodexReasoningEffort};

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
        let caps = ProviderCapabilities::for_provider(provider);
        let inferred = resolve_model_metadata(provider, model, api_format);
        let mut options = self.clone();

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
                Some(ReasoningFormat::OpenAI) => {
                    options.anthropic_adaptive_effort = None;
                }
                Some(ReasoningFormat::Anthropic) => {
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

        if !(provider == ProviderId::OpenAI && matches!(api_format, ApiFormat::OpenAIResponses)) {
            options.codex_parallel_tool_calls = false;
        }

        options
    }
}

#[cfg(test)]
mod tests {
    use super::CallOptions;
    use crate::ai::client::config::CodexReasoningEffort;
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
