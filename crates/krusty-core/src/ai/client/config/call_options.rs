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
    /// Request provider priority/fast service tier without changing the selected model.
    pub fast_mode: bool,
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
            fast_mode: false,
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

        if provider == ProviderId::Grok {
            // Krusty's Grok provider targets the Grok CLI proxy, which can emit
            // reasoning blocks but rejects explicit `reasoning`/`reasoningEffort`
            // request controls. Keep parsing reasoning output, but never send a
            // provider-native thinking knob on this transport.
            options.reasoning_format = None;
            options.thinking = None;
            options.codex_reasoning_effort = None;
            options.anthropic_adaptive_effort = None;
        } else if inferred.reasoning_format.is_none() {
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

        let hosted_web_search = provider_hosted_web_search_supported(provider, api_format, &caps);
        let hosted_web_fetch = caps.web_fetch;

        if !hosted_web_search {
            options.web_search = None;
        }
        if !hosted_web_fetch {
            options.web_fetch = None;
        }

        // When a provider-native hosted web tool is active, remove the local
        // function tool with the same name to avoid duplicate tool definitions.
        // Unsupported providers keep the local portable web tools instead.
        // OpenAI transport selection happens after canonicalization; it removes
        // the local web_search function only on the standard hosted Responses path.
        if options.web_search.is_some() && provider != ProviderId::OpenAI {
            remove_ai_tool_named(&mut options.tools, "web_search");
        }
        if options.web_fetch.is_some() {
            remove_ai_tool_named(&mut options.tools, "web_fetch");
        }

        if !(matches!(provider, ProviderId::OpenAI | ProviderId::Grok)
            && matches!(api_format, ApiFormat::OpenAIResponses))
        {
            options.codex_parallel_tool_calls = false;
        }

        options
    }

    /// Map Krusty's provider-independent fast-mode toggle to provider wire values.
    ///
    /// This intentionally does not mutate the model ID: `gpt-5.5` and
    /// `gpt-5.5-mini` can both run in standard or fast service tiers.
    pub fn service_tier_for_provider(&self, provider: ProviderId) -> Option<&'static str> {
        match (provider, self.fast_mode) {
            (ProviderId::OpenAI | ProviderId::OpenRouter, true) => Some("priority"),
            (ProviderId::Anthropic, true) => Some("auto"),
            (ProviderId::Anthropic, false) => Some("standard_only"),
            _ => None,
        }
    }
}

fn provider_hosted_web_search_supported(
    provider: ProviderId,
    api_format: ApiFormat,
    caps: &ProviderCapabilities,
) -> bool {
    match provider {
        ProviderId::OpenAI => caps.web_search && matches!(api_format, ApiFormat::OpenAIResponses),
        ProviderId::OpenRouter => caps.web_plugins || caps.web_search,
        _ => caps.web_search,
    }
}

fn remove_ai_tool_named(tools: &mut Option<Vec<AiTool>>, name: &str) {
    if let Some(items) = tools {
        items.retain(|tool| tool.name != name);
        if items.is_empty() {
            *tools = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::CallOptions;
    use crate::ai::client::config::CodexReasoningEffort;
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::{ProviderId, ReasoningFormat};
    use crate::ai::types::{
        AiTool, ContextManagement, ThinkingConfig, WebFetchConfig, WebSearchConfig,
    };

    fn tool(name: &str) -> AiTool {
        AiTool {
            name: name.to_string(),
            description: format!("{name} description"),
            input_schema: json!({"type": "object"}),
            prompt: None,
        }
    }

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
    fn canonicalization_prefers_hosted_web_tools_when_supported() {
        let options = CallOptions {
            tools: Some(vec![tool("read"), tool("web_search"), tool("web_fetch")]),
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        let canonical = options.canonicalized_for(
            ProviderId::Anthropic,
            "claude-opus-4-6",
            ApiFormat::Anthropic,
        );

        assert!(canonical.web_search.is_some());
        assert!(canonical.web_fetch.is_some());
        let names = canonical
            .tools
            .unwrap()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["read"]);
    }

    #[test]
    fn canonicalization_keeps_local_web_fallbacks_when_hosted_is_unsupported() {
        let options = CallOptions {
            tools: Some(vec![tool("web_search"), tool("web_fetch")]),
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        let canonical =
            options.canonicalized_for(ProviderId::MiniMax, "MiniMax-M2.5", ApiFormat::Anthropic);

        assert!(canonical.web_search.is_none());
        assert!(canonical.web_fetch.is_none());
        let names = canonical
            .tools
            .unwrap()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search", "web_fetch"]);
    }

    #[test]
    fn canonicalization_preserves_openai_local_web_tool_until_transport_selection() {
        let options = CallOptions {
            tools: Some(vec![tool("web_search"), tool("web_fetch")]),
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        let canonical =
            options.canonicalized_for(ProviderId::OpenAI, "gpt-5.5", ApiFormat::OpenAIResponses);

        assert!(canonical.web_search.is_some());
        assert!(canonical.web_fetch.is_none());
        let names = canonical
            .tools
            .unwrap()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search", "web_fetch"]);
    }

    #[test]
    fn canonicalization_uses_openrouter_server_tools_for_search_and_fetch() {
        let options = CallOptions {
            tools: Some(vec![tool("read"), tool("web_search"), tool("web_fetch")]),
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        let canonical = options.canonicalized_for(
            ProviderId::OpenRouter,
            "openai/gpt-5.5",
            ApiFormat::Anthropic,
        );

        assert!(canonical.web_search.is_some());
        assert!(canonical.web_fetch.is_some());
        let names = canonical
            .tools
            .unwrap()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["read"]);
    }

    #[test]
    fn canonicalization_keeps_local_web_fallbacks_for_grok_proxy() {
        let options = CallOptions {
            tools: Some(vec![tool("web_search"), tool("web_fetch")]),
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        let canonical =
            options.canonicalized_for(ProviderId::Grok, "grok-build", ApiFormat::OpenAIResponses);

        assert!(canonical.web_search.is_none());
        assert!(canonical.web_fetch.is_none());
        let names = canonical
            .tools
            .unwrap()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search", "web_fetch"]);
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

    #[test]
    fn canonicalization_strips_grok_proxy_reasoning_controls() {
        let options = CallOptions {
            thinking: Some(ThinkingConfig::default()),
            codex_reasoning_effort: Some(CodexReasoningEffort::XHigh),
            codex_parallel_tool_calls: true,
            ..Default::default()
        };

        let canonical =
            options.canonicalized_for(ProviderId::Grok, "grok-build", ApiFormat::OpenAIResponses);

        assert!(canonical.reasoning_format.is_none());
        assert!(canonical.codex_reasoning_effort.is_none());
        assert!(canonical.codex_parallel_tool_calls);
        assert!(canonical.thinking.is_none());
    }

    #[test]
    fn fast_mode_maps_to_provider_service_tiers_without_changing_models() {
        let options = CallOptions {
            fast_mode: true,
            ..Default::default()
        };

        assert_eq!(
            options.service_tier_for_provider(ProviderId::OpenAI),
            Some("priority")
        );
        assert_eq!(
            options.service_tier_for_provider(ProviderId::Anthropic),
            Some("auto")
        );
        assert_eq!(
            options.service_tier_for_provider(ProviderId::OpenRouter),
            Some("priority")
        );
        assert_eq!(options.service_tier_for_provider(ProviderId::MiniMax), None);
        assert_eq!(options.service_tier_for_provider(ProviderId::ZAi), None);
    }

    #[test]
    fn standard_mode_does_not_request_priority_tiers() {
        let options = CallOptions::default();

        assert_eq!(options.service_tier_for_provider(ProviderId::OpenAI), None);
        assert_eq!(
            options.service_tier_for_provider(ProviderId::Anthropic),
            Some("standard_only")
        );
        assert_eq!(
            options.service_tier_for_provider(ProviderId::OpenRouter),
            None
        );
    }
}
