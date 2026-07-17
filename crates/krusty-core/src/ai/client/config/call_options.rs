use crate::ai::models::{resolve_model_metadata, ApiFormat};
use crate::ai::providers::{ProviderCapabilities, ProviderId, ReasoningFormat};
use crate::ai::types::{
    AiTool, ContextManagement, ThinkingConfig, WebFetchConfig, WebSearchConfig,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::effort::{AnthropicAdaptiveEffort, CodexReasoningEffort};

/// Provider prompt-cache lifetime preference.
///
/// `Standard` uses OpenAI's model default (30 minutes minimum on GPT-5.6+) and
/// Anthropic's default 5 minutes. `Extended` requests 24 hours only on the
/// earlier OpenAI families that support it and 1 hour on Anthropic. GPT-5.6+
/// remains fixed at its only supported `30m` TTL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PromptCacheRetention {
    #[default]
    Standard,
    Extended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenAiPromptCacheMode {
    Implicit,
    Explicit,
}

impl OpenAiPromptCacheMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Implicit => "implicit",
            Self::Explicit => "explicit",
        }
    }
}

impl PromptCacheRetention {
    fn from_env() -> Self {
        std::env::var("KRUSTY_CACHE_RETENTION")
            .ok()
            .as_deref()
            .map(Self::from_config_value)
            .unwrap_or_default()
    }

    fn from_config_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "long" | "extended" | "24h" | "1h" => Self::Extended,
            _ => Self::Standard,
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
    /// Requested provider prompt-cache lifetime. Defaults from
    /// `KRUSTY_CACHE_RETENTION` (`long`/`extended` opt in to extended caching).
    pub prompt_cache_retention: PromptCacheRetention,
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
            prompt_cache_retention: PromptCacheRetention::from_env(),
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

const MAX_PROMPT_CACHE_KEY_CHARS: usize = 64;

/// Normalize a provider prompt-cache key without creating prefix collisions.
///
/// OpenAI accepts at most 64 characters. Preserve already-valid identifiers so
/// existing sessions keep their cache affinity, but hash the complete UTF-8
/// value when it is longer instead of truncating a potentially shared prefix.
pub(crate) fn normalize_prompt_cache_key(key: &str) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    if key.chars().count() <= MAX_PROMPT_CACHE_KEY_CHARS {
        return Some(key.to_string());
    }

    Some(format!("{:x}", Sha256::digest(key.as_bytes())))
}

/// Resolve the normalized cache key for a call, honoring the caching toggle.
pub(crate) fn normalized_prompt_cache_key(options: &CallOptions) -> Option<String> {
    if !options.enable_caching {
        return None;
    }
    options
        .session_id
        .as_deref()
        .and_then(normalize_prompt_cache_key)
}

/// Build the current OpenAI cache-options envelope when that model supports it.
pub(crate) fn openai_prompt_cache_options(
    options: &CallOptions,
    model: &str,
    mode: OpenAiPromptCacheMode,
) -> Option<Value> {
    if normalized_prompt_cache_key(options).is_none() || !supports_prompt_cache_options(model) {
        return None;
    }

    Some(serde_json::json!({
        "mode": mode.as_str(),
        "ttl": "30m",
    }))
}

/// Resolve the deprecated extended-retention field for earlier model families.
/// GPT-5.6+ must use `prompt_cache_options.ttl = "30m"` instead.
pub(crate) fn openai_prompt_cache_retention(options: &CallOptions, model: &str) -> Option<Value> {
    if options.prompt_cache_retention != PromptCacheRetention::Extended
        || normalized_prompt_cache_key(options).is_none()
        || !supports_extended_prompt_cache_retention(model)
    {
        return None;
    }

    Some(serde_json::json!("24h"))
}

fn supports_extended_prompt_cache_retention(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    const SUPPORTED_FAMILIES: &[&str] = &[
        "gpt-5.5-pro",
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.2",
        "gpt-5.1-codex-max",
        "gpt-5.1-codex-mini",
        "gpt-5.1-codex",
        "gpt-5.1-chat-latest",
        "gpt-5.1",
        "gpt-5-codex",
        "gpt-5",
        "gpt-4.1",
    ];

    SUPPORTED_FAMILIES.iter().any(|family| {
        model == *family
            || model.strip_prefix(family).is_some_and(|suffix| {
                suffix
                    .strip_prefix('-')
                    .and_then(|suffix| suffix.chars().next())
                    .is_some_and(|character| character.is_ascii_digit())
            })
    })
}

/// Build Anthropic-style cache control for capable transports. The documented
/// one-hour TTL is sent only to Anthropic itself; compatible proxies retain
/// their existing default policy rather than receiving an assumed extension.
pub(crate) fn anthropic_prompt_cache_control(
    options: &CallOptions,
    provider: ProviderId,
) -> Option<Value> {
    let capabilities = ProviderCapabilities::for_provider(provider);
    if !options.enable_caching || !capabilities.prompt_caching {
        return None;
    }

    let mut control = serde_json::json!({"type": "ephemeral"});
    if options.prompt_cache_retention == PromptCacheRetention::Extended
        && provider == ProviderId::Anthropic
    {
        control["ttl"] = serde_json::json!("1h");
    }
    Some(control)
}

/// GPT-5.6 introduced request-wide prompt cache options and explicit
/// breakpoints. Earlier models reject those fields.
pub(crate) fn supports_prompt_cache_options(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    let Some(version) = model.strip_prefix("gpt-") else {
        return false;
    };
    let numeric = version
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect::<String>();
    let mut parts = numeric.split('.');
    let major = parts.next().and_then(|part| part.parse::<u32>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if (major, minor) >= (5, 6))
        || matches!(major, Some(major) if major > 5)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        anthropic_prompt_cache_control, normalize_prompt_cache_key, normalized_prompt_cache_key,
        openai_prompt_cache_options, openai_prompt_cache_retention, supports_prompt_cache_options,
        CallOptions, OpenAiPromptCacheMode, PromptCacheRetention,
    };
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
    fn prompt_cache_options_are_model_gated() {
        assert!(!supports_prompt_cache_options("gpt-5.5"));
        assert!(supports_prompt_cache_options("gpt-5.6"));
        assert!(supports_prompt_cache_options("openai/gpt-5.7-codex"));
        assert!(supports_prompt_cache_options("gpt-6"));
        assert!(!supports_prompt_cache_options("claude-opus-4-6"));
    }

    #[test]
    fn prompt_cache_key_preserves_short_and_boundary_values() {
        let short = "session:01JZ-example";
        assert_eq!(normalize_prompt_cache_key(short).as_deref(), Some(short));

        let boundary = "x".repeat(64);
        assert_eq!(normalize_prompt_cache_key(&boundary), Some(boundary));
        assert_eq!(normalize_prompt_cache_key(""), None);
    }

    #[test]
    fn prompt_cache_key_hashes_the_complete_long_value_deterministically() {
        let long = "session/".to_string() + &"a".repeat(80);
        let normalized = normalize_prompt_cache_key(&long).expect("long key should normalize");

        assert_eq!(normalized.len(), 64);
        assert_eq!(
            normalized,
            "ccd91d5ebd1f7899dbe3de69cacf02b3809a63ed5a3a0b16f0234fc438786e49"
        );
        assert_eq!(normalized, normalize_prompt_cache_key(&long).unwrap());

        let different_suffix = "session/".to_string() + &"a".repeat(79) + "b";
        assert_ne!(
            normalized,
            normalize_prompt_cache_key(&different_suffix).unwrap()
        );
    }

    #[test]
    fn prompt_cache_key_hashes_long_composite_session_identifiers() {
        let shared_prefix = format!("openai:gpt-5.6:{}", "project/".repeat(8));
        let first = format!("{shared_prefix}:delegated-run-a");
        let second = format!("{shared_prefix}:delegated-run-b");

        let first_key = normalize_prompt_cache_key(&first).unwrap();
        let second_key = normalize_prompt_cache_key(&second).unwrap();
        assert_eq!(first_key.len(), 64);
        assert_eq!(second_key.len(), 64);
        assert_ne!(first_key, second_key);

        let disabled = CallOptions {
            enable_caching: false,
            session_id: Some(first),
            ..Default::default()
        };
        assert!(normalized_prompt_cache_key(&disabled).is_none());
    }

    #[test]
    fn prompt_cache_retention_maps_default_and_extended_provider_values() {
        let standard = CallOptions {
            session_id: Some("session".into()),
            prompt_cache_retention: PromptCacheRetention::Standard,
            ..Default::default()
        };
        assert_eq!(
            openai_prompt_cache_options(&standard, "gpt-5.6", OpenAiPromptCacheMode::Implicit)
                .unwrap()["ttl"],
            "30m"
        );
        assert_eq!(
            anthropic_prompt_cache_control(&standard, ProviderId::Anthropic).unwrap(),
            json!({"type": "ephemeral"})
        );

        let extended = CallOptions {
            prompt_cache_retention: PromptCacheRetention::Extended,
            ..standard
        };
        assert_eq!(
            openai_prompt_cache_options(&extended, "gpt-5.6", OpenAiPromptCacheMode::Explicit)
                .unwrap()["ttl"],
            "30m"
        );
        assert_eq!(
            openai_prompt_cache_options(&extended, "gpt-5.6", OpenAiPromptCacheMode::Explicit)
                .unwrap()["mode"],
            "explicit"
        );
        assert!(openai_prompt_cache_retention(&extended, "gpt-5.6").is_none());
        assert_eq!(
            openai_prompt_cache_retention(&extended, "gpt-5.5").unwrap(),
            "24h"
        );
        assert_eq!(
            anthropic_prompt_cache_control(&extended, ProviderId::Anthropic).unwrap(),
            json!({"type": "ephemeral", "ttl": "1h"})
        );
    }

    #[test]
    fn prompt_cache_retention_is_omitted_for_unsupported_or_disabled_paths() {
        let extended = CallOptions {
            session_id: Some("session".into()),
            prompt_cache_retention: PromptCacheRetention::Extended,
            ..Default::default()
        };

        assert!(
            openai_prompt_cache_options(&extended, "gpt-5.5", OpenAiPromptCacheMode::Implicit)
                .is_none()
        );
        assert!(openai_prompt_cache_retention(&extended, "gpt-4o").is_none());
        assert!(anthropic_prompt_cache_control(&extended, ProviderId::MiniMax).is_none());
        assert_eq!(
            anthropic_prompt_cache_control(&extended, ProviderId::OpenRouter).unwrap(),
            json!({"type": "ephemeral"})
        );

        let disabled = CallOptions {
            enable_caching: false,
            ..extended
        };
        assert!(
            openai_prompt_cache_options(&disabled, "gpt-5.6", OpenAiPromptCacheMode::Implicit)
                .is_none()
        );
        assert!(anthropic_prompt_cache_control(&disabled, ProviderId::Anthropic).is_none());
    }

    #[test]
    fn prompt_cache_retention_parses_environment_style_values() {
        assert_eq!(
            PromptCacheRetention::from_config_value("long"),
            PromptCacheRetention::Extended
        );
        assert_eq!(
            PromptCacheRetention::from_config_value(" 24H "),
            PromptCacheRetention::Extended
        );
        assert_eq!(
            PromptCacheRetention::from_config_value("short"),
            PromptCacheRetention::Standard
        );
        assert_eq!(
            PromptCacheRetention::from_config_value("unexpected"),
            PromptCacheRetention::Standard
        );
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
