//! AI provider configuration
//!
//! Defines provider types, configurations, and built-in provider registry
//! for Anthropic-compatible API endpoints.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use crate::ai::models::ApiFormat;
use crate::auth::{AnthropicAuthType, OpenAIAuthType};

/// ChatGPT backend API for OAuth users (Responses API)
/// This endpoint is required for tokens obtained via ChatGPT OAuth flow.
/// Note: ChatGPT's Codex API does NOT use /v1/ prefix unlike the standard OpenAI API.
pub const CHATGPT_RESPONSES_API: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Standard OpenAI API for API key users (Chat Completions)
/// This endpoint is used when authenticating with an API key.
pub const OPENAI_CHAT_API: &str = "https://api.openai.com/v1/chat/completions";

/// Unique identifier for each supported provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    #[default]
    MiniMax,
    OpenRouter,
    ZAi,
    Anthropic,
    OpenAI,
}

impl ProviderId {
    /// Get all available provider IDs
    /// Order: MiniMax first (default), then smallest to largest, OpenRouter last
    pub fn all() -> &'static [ProviderId] {
        &[
            ProviderId::MiniMax,    // Default provider, always first
            ProviderId::Anthropic,  // Anthropic direct (OAuth or API key)
            ProviderId::OpenAI,     // OpenAI direct (OAuth or API key)
            ProviderId::ZAi,        // GLM-5
            ProviderId::OpenRouter, // 100+ dynamic models, always last
        ]
    }

    /// Get the storage key for this provider (used in credentials.json)
    pub fn storage_key(&self) -> &'static str {
        match self {
            ProviderId::MiniMax => "minimax",
            ProviderId::OpenRouter => "openrouter",
            ProviderId::ZAi => "z_ai",
            ProviderId::Anthropic => "anthropic",
            ProviderId::OpenAI => "openai",
        }
    }

    /// Check if this provider supports OAuth authentication
    pub fn supports_oauth(&self) -> bool {
        matches!(self, ProviderId::OpenAI | ProviderId::Anthropic)
    }

    /// Get the authentication methods supported by this provider
    pub fn auth_methods(&self) -> Vec<crate::auth::AuthMethod> {
        use crate::auth::AuthMethod;
        match self {
            ProviderId::OpenAI => vec![
                AuthMethod::OAuthBrowser,
                AuthMethod::OAuthDevice,
                AuthMethod::ApiKey,
            ],
            ProviderId::Anthropic => vec![AuthMethod::OAuthBrowser, AuthMethod::ApiKey],
            _ => vec![AuthMethod::ApiKey],
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderId::MiniMax => write!(f, "MiniMax"),
            ProviderId::OpenRouter => write!(f, "OpenRouter"),
            ProviderId::ZAi => write!(f, "Z.ai"),
            ProviderId::Anthropic => write!(f, "Anthropic"),
            ProviderId::OpenAI => write!(f, "OpenAI"),
        }
    }
}

/// How to send the API key in requests
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthHeader {
    /// Use `x-api-key: <key>` header (Anthropic style)
    #[default]
    XApiKey,
    /// Use `Authorization: Bearer <key>` header (OpenAI style)
    Bearer,
}

// ============================================================================
// Universal Reasoning Support
// ============================================================================

/// Different reasoning/thinking formats used by various providers
/// When enabled, we always use MAX effort - no in-between settings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningFormat {
    /// Anthropic Claude: `thinking.budget_tokens` (see `DEFAULT_THINKING_BUDGET`)
    Anthropic,
    /// OpenAI o1/o3/GPT-5: `reasoning_effort: "high"`
    OpenAI,
    /// DeepSeek R1: `reasoning.enabled: true`
    DeepSeek,
}

/// Information about a model offered by a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model ID to send in API requests
    pub id: String,
    /// Human-readable display name
    pub display_name: String,
    /// Context window size in tokens
    pub context_window: usize,
    /// Maximum output tokens
    pub max_output: usize,
    /// Reasoning/thinking support (None = not supported)
    pub reasoning: Option<ReasoningFormat>,
}

impl ModelInfo {
    pub fn new(id: &str, display_name: &str, context_window: usize, max_output: usize) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            context_window,
            max_output,
            reasoning: None,
        }
    }

    /// Add Anthropic-style extended thinking support
    pub fn with_anthropic_thinking(mut self) -> Self {
        self.reasoning = Some(ReasoningFormat::Anthropic);
        self
    }
}

/// Configuration for an AI provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique identifier
    pub id: ProviderId,
    /// Display name
    pub name: String,
    /// Short description for UI
    pub description: String,
    /// API base URL (without trailing slash)
    pub base_url: String,
    /// How to send authentication
    pub auth_header: AuthHeader,
    /// Available models (empty for dynamic providers like OpenRouter)
    pub models: Vec<ModelInfo>,
    /// Whether this provider supports tool calling
    pub supports_tools: bool,
    /// Whether models can have dynamic list (fetched from API)
    pub dynamic_models: bool,
    /// Pricing hint to show in UI (e.g., "~1% of Claude")
    pub pricing_hint: Option<String>,
    /// Custom headers to send with requests
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
}

impl ProviderConfig {
    /// Get the default model ID for this provider
    /// Returns the first model in the list, or a hardcoded fallback for dynamic providers
    pub fn default_model(&self) -> &str {
        if let Some(first) = self.models.first() {
            &first.id
        } else {
            // Dynamic providers need a fallback
            match self.id {
                ProviderId::OpenRouter => "openai/gpt-5.3-codex",
                _ => "MiniMax-M2.5", // Ultimate fallback
            }
        }
    }

    /// Check if a model ID is valid for this provider
    pub fn has_model(&self, model_id: &str) -> bool {
        // For dynamic providers, we can't validate statically
        if self.dynamic_models {
            return true;
        }
        self.models.iter().any(|m| m.id == model_id)
    }

    /// Get the API base URL for OpenAI based on auth type
    ///
    /// - ChatGPT OAuth tokens require the Responses API at chatgpt.com
    /// - API keys use the standard Chat Completions API at api.openai.com
    pub fn openai_url_for_auth(auth_type: OpenAIAuthType) -> &'static str {
        match auth_type {
            OpenAIAuthType::ChatGptOAuth => CHATGPT_RESPONSES_API,
            OpenAIAuthType::ApiKey | OpenAIAuthType::None => OPENAI_CHAT_API,
        }
    }

    /// Get the API format for OpenAI based on auth type
    ///
    /// - ChatGPT OAuth requires OpenAI Responses format
    /// - API keys use standard OpenAI chat/completions format
    pub fn openai_format_for_auth(auth_type: OpenAIAuthType) -> ApiFormat {
        match auth_type {
            OpenAIAuthType::ChatGptOAuth => ApiFormat::OpenAIResponses,
            OpenAIAuthType::ApiKey | OpenAIAuthType::None => ApiFormat::OpenAI,
        }
    }

    /// Get the auth header for Anthropic based on auth type
    ///
    /// - OAuth tokens use Bearer authorization
    /// - API keys use x-api-key header
    pub fn anthropic_auth_header_for_auth(auth_type: AnthropicAuthType) -> AuthHeader {
        match auth_type {
            AnthropicAuthType::OAuth => AuthHeader::Bearer,
            AnthropicAuthType::ApiKey | AnthropicAuthType::None => AuthHeader::XApiKey,
        }
    }
}

// ============================================================================
// Model Mapping System
// ============================================================================

/// Canonical model families that exist across providers
/// Maps to provider-specific IDs for seamless switching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    ClaudeOpus4_6,
    ClaudeOpus4_5,
    ClaudeSonnet4_5,
    ClaudeSonnet4,
    ClaudeHaiku4_5,
    ClaudeOpus4,
}

/// Model ID mapping entry: (canonical_family, provider, provider_specific_id)
static MODEL_MAPPINGS: LazyLock<Vec<(ModelFamily, ProviderId, &'static str)>> =
    LazyLock::new(|| {
        vec![
            // Claude Opus 4.6
            (
                ModelFamily::ClaudeOpus4_6,
                ProviderId::Anthropic,
                "claude-opus-4-6",
            ),
            (
                ModelFamily::ClaudeOpus4_6,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4.6",
            ),
            // Claude Opus 4.5
            (
                ModelFamily::ClaudeOpus4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4.5",
            ),
            // Claude Sonnet 4.5
            (
                ModelFamily::ClaudeSonnet4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-4.5",
            ),
            // Claude Sonnet 4
            (
                ModelFamily::ClaudeSonnet4,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-4",
            ),
            // Claude Haiku 4.5
            (
                ModelFamily::ClaudeHaiku4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-haiku-4.5",
            ),
            // Claude Opus 4
            (
                ModelFamily::ClaudeOpus4,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4",
            ),
        ]
    });

/// Find the canonical model family for a provider-specific model ID
pub fn get_model_family(model_id: &str) -> Option<ModelFamily> {
    MODEL_MAPPINGS
        .iter()
        .find(|(_, _, id)| *id == model_id)
        .map(|(family, _, _)| *family)
}

/// Translate a model ID from one provider to another
/// Returns None if no mapping exists (model is provider-specific)
pub fn translate_model_id(model_id: &str, from: ProviderId, to: ProviderId) -> Option<String> {
    // Same provider, no translation needed
    if from == to {
        return Some(model_id.to_string());
    }

    // Find the canonical family for this model
    let family = get_model_family(model_id)?;

    // Find the target provider's ID for this family
    MODEL_MAPPINGS
        .iter()
        .find(|(f, p, _)| *f == family && *p == to)
        .map(|(_, _, id)| id.to_string())
}

/// Get the equivalent model ID for a target provider, or the provider's default
pub fn translate_model_or_default(model_id: &str, from: ProviderId, to: ProviderId) -> String {
    translate_model_id(model_id, from, to).unwrap_or_else(|| {
        get_provider(to)
            .map(|p| p.default_model().to_string())
            .unwrap_or_else(|| "MiniMax-M2.5".to_string())
    })
}

// ============================================================================
// Provider Capabilities
// ============================================================================

/// Features supported by a provider (used for feature negotiation)
#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    /// Server-executed web search (Anthropic: web_search_20250305)
    pub web_search: bool,
    /// Server-executed web fetch (Anthropic: web_fetch_20250910)
    pub web_fetch: bool,
    /// Context management / auto-clearing
    pub context_management: bool,
    /// Prompt caching support
    pub prompt_caching: bool,
    /// Web search via plugins array (OpenRouter style)
    pub web_plugins: bool,
    /// Native image/document content block support
    pub supports_vision: bool,
}

impl ProviderCapabilities {
    /// Get capabilities for a provider
    pub fn for_provider(provider: ProviderId) -> Self {
        match provider {
            // OpenRouter passes through Anthropic's cache_control to Claude models.
            // For non-Claude models, caching is automatic and the extra fields are ignored.
            ProviderId::OpenRouter => Self {
                web_search: false, // Not via server tools
                web_fetch: false,
                context_management: false,
                prompt_caching: true,
                web_plugins: true,     // Uses plugins array
                supports_vision: true, // Passes through to underlying model
            },
            // Anthropic: native prompt caching
            ProviderId::Anthropic => Self {
                web_search: false,
                web_fetch: false,
                context_management: false,
                prompt_caching: true,
                web_plugins: false,
                supports_vision: true,
            },
            // OpenAI: supports tools but not server-executed web search
            ProviderId::OpenAI => Self {
                web_search: false,
                web_fetch: false,
                context_management: false,
                prompt_caching: false,
                web_plugins: false,
                supports_vision: true,
            },
            // Other providers: minimal capabilities (no vision)
            ProviderId::ZAi | ProviderId::MiniMax => Self::default(),
        }
    }
}

/// Lazily initialized built-in provider configurations
static BUILTIN_PROVIDERS: LazyLock<Vec<ProviderConfig>> = LazyLock::new(|| {
    vec![
        // OpenRouter - access to 100+ models (Anthropic-compatible "skin")
        ProviderConfig {
            id: ProviderId::OpenRouter,
            name: "OpenRouter".to_string(),
            description: "100+ models (GPT, Gemini, Llama, Claude)".to_string(),
            base_url: "https://openrouter.ai/api/v1/messages".to_string(),
            auth_header: AuthHeader::Bearer,
            models: vec![
                // Claude models
                ModelInfo::new(
                    "anthropic/claude-opus-4.5",
                    "Claude Opus 4.5",
                    200_000,
                    16_384,
                )
                .with_anthropic_thinking(),
                ModelInfo::new(
                    "anthropic/claude-sonnet-4.5",
                    "Claude Sonnet 4.5",
                    1_000_000,
                    16_384,
                )
                .with_anthropic_thinking(),
                ModelInfo::new(
                    "anthropic/claude-sonnet-4",
                    "Claude Sonnet 4",
                    200_000,
                    8_192,
                ),
                ModelInfo::new(
                    "anthropic/claude-haiku-4.5",
                    "Claude Haiku 4.5",
                    200_000,
                    16_384,
                ),
                ModelInfo::new("anthropic/claude-opus-4", "Claude Opus 4", 200_000, 16_384),
                // OpenAI models
                ModelInfo::new("openai/gpt-5.3-codex", "GPT-5.3 Codex", 400_000, 128_000),
                // Google models
                ModelInfo::new(
                    "google/gemini-2.5-pro-preview",
                    "Gemini 2.5 Pro",
                    1_000_000,
                    65_536,
                ),
                ModelInfo::new(
                    "google/gemini-2.5-flash-preview",
                    "Gemini 2.5 Flash",
                    1_000_000,
                    65_536,
                ),
                ModelInfo::new(
                    "google/gemini-2.0-flash-001",
                    "Gemini 2.0 Flash",
                    1_000_000,
                    8_192,
                ),
                // DeepSeek models
                ModelInfo::new("deepseek/deepseek-r1", "DeepSeek R1", 64_000, 8_192),
                ModelInfo::new(
                    "deepseek/deepseek-chat-v3-0324",
                    "DeepSeek V3",
                    64_000,
                    8_192,
                ),
                // Meta Llama models
                ModelInfo::new(
                    "meta-llama/llama-4-maverick",
                    "Llama 4 Maverick",
                    1_000_000,
                    256_000,
                ),
                ModelInfo::new(
                    "meta-llama/llama-4-scout",
                    "Llama 4 Scout",
                    512_000,
                    128_000,
                ),
                // Qwen models
                ModelInfo::new("qwen/qwen3-235b-a22b", "Qwen 3 235B", 128_000, 8_192),
                ModelInfo::new("qwen/qwq-32b", "QwQ 32B", 128_000, 16_384),
            ],
            supports_tools: true,
            dynamic_models: true,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
        // Z.ai - GLM Coding Plan (Anthropic-compatible endpoint)
        ProviderConfig {
            id: ProviderId::ZAi,
            name: "Z.ai".to_string(),
            description: "GLM Coding Plan (GLM-5)".to_string(),
            base_url: "https://api.z.ai/api/anthropic/v1/messages".to_string(),
            auth_header: AuthHeader::XApiKey,
            models: vec![ModelInfo::new("GLM-5", "GLM 5", 200_000, 131_072)],
            supports_tools: true,
            dynamic_models: false,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
        // MiniMax - M2.5 (Anthropic-compatible API)
        ProviderConfig {
            id: ProviderId::MiniMax,
            name: "MiniMax".to_string(),
            description: "M2.5 (fast, interleaved thinking)".to_string(),
            base_url: "https://api.minimax.io/anthropic/v1/messages".to_string(),
            auth_header: AuthHeader::XApiKey,
            models: vec![
                ModelInfo::new("MiniMax-M2.5", "MiniMax M2.5", 204_800, 131_072)
                    .with_anthropic_thinking(),
            ],
            supports_tools: true,
            dynamic_models: false,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
        // Anthropic - Direct access with OAuth or API key (native Anthropic format)
        ProviderConfig {
            id: ProviderId::Anthropic,
            name: "Anthropic".to_string(),
            description: "Claude Opus 4.6 + Haiku (OAuth or API key)".to_string(),
            base_url: "https://api.anthropic.com/v1/messages".to_string(),
            auth_header: AuthHeader::Bearer, // OAuth uses Bearer; API key path overrides to XApiKey
            models: vec![
                ModelInfo::new("claude-opus-4-6", "Claude Opus 4.6", 200_000, 128_000)
                    .with_anthropic_thinking(),
                ModelInfo::new(
                    "claude-haiku-4-5-20251001",
                    "Claude Haiku 4.5",
                    200_000,
                    16_384,
                ),
            ],
            supports_tools: true,
            dynamic_models: false,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
        // OpenAI - Direct access with OAuth or API key (OpenAI-compatible format)
        // Supports OAuth browser flow, device code flow, and API key authentication
        ProviderConfig {
            id: ProviderId::OpenAI,
            name: "OpenAI".to_string(),
            description: "GPT-5.3 Codex (OAuth or API key)".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            auth_header: AuthHeader::Bearer,
            models: vec![ModelInfo::new(
                "gpt-5.3-codex",
                "GPT-5.3 Codex",
                400_000,
                128_000,
            )],
            supports_tools: true,
            dynamic_models: true,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
    ]
});

/// Get all built-in provider configurations (cached, no allocation)
pub fn builtin_providers() -> &'static [ProviderConfig] {
    &BUILTIN_PROVIDERS
}

/// Get a specific provider configuration by ID
pub fn get_provider(id: ProviderId) -> Option<&'static ProviderConfig> {
    BUILTIN_PROVIDERS.iter().find(|p| p.id == id)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_id_display() {
        assert_eq!(ProviderId::MiniMax.to_string(), "MiniMax");
        assert_eq!(ProviderId::OpenRouter.to_string(), "OpenRouter");
        assert_eq!(ProviderId::ZAi.to_string(), "Z.ai");
        assert_eq!(ProviderId::Anthropic.to_string(), "Anthropic");
        assert_eq!(ProviderId::OpenAI.to_string(), "OpenAI");
    }

    #[test]
    fn test_storage_keys() {
        assert_eq!(ProviderId::MiniMax.storage_key(), "minimax");
        assert_eq!(ProviderId::ZAi.storage_key(), "z_ai");
        assert_eq!(ProviderId::Anthropic.storage_key(), "anthropic");
        assert_eq!(ProviderId::OpenAI.storage_key(), "openai");
    }

    #[test]
    fn test_builtin_providers() {
        let providers = builtin_providers();
        assert_eq!(providers.len(), 5);
        assert!(providers.iter().any(|p| p.id == ProviderId::MiniMax));
        assert!(providers.iter().any(|p| p.id == ProviderId::OpenRouter));
        assert!(providers.iter().any(|p| p.id == ProviderId::Anthropic));
        assert!(providers.iter().any(|p| p.id == ProviderId::OpenAI));
        assert!(providers.iter().any(|p| p.id == ProviderId::ZAi));
    }

    #[test]
    fn test_get_provider() {
        let minimax = get_provider(ProviderId::MiniMax).unwrap();
        assert_eq!(minimax.name, "MiniMax");
        assert!(!minimax.models.is_empty());
    }

    #[test]
    fn test_minimax_config() {
        let provider = get_provider(ProviderId::MiniMax).unwrap();
        assert_eq!(
            provider.base_url,
            "https://api.minimax.io/anthropic/v1/messages"
        );
        assert_eq!(provider.auth_header, AuthHeader::XApiKey);
        assert_eq!(provider.default_model(), "MiniMax-M2.5");
    }

    #[test]
    fn test_openrouter_config() {
        let provider = get_provider(ProviderId::OpenRouter).unwrap();
        // OpenRouter uses Anthropic-compatible API at /api/v1/messages
        assert_eq!(provider.base_url, "https://openrouter.ai/api/v1/messages");
        assert_eq!(provider.auth_header, AuthHeader::Bearer);
        assert!(provider.dynamic_models);
    }

    #[test]
    fn test_model_validation() {
        let minimax = get_provider(ProviderId::MiniMax).unwrap();
        // Valid MiniMax model
        assert!(minimax.has_model("MiniMax-M2.5"));
        // Invalid model
        assert!(!minimax.has_model("anthropic/claude-opus-4.5"));

        // OpenRouter allows any model (dynamic)
        let openrouter = get_provider(ProviderId::OpenRouter).unwrap();
        assert!(openrouter.has_model("anthropic/claude-opus-4.5"));
        assert!(openrouter.has_model("openai/gpt-4"));
    }

    #[test]
    fn test_model_family_detection() {
        // OpenRouter format
        assert_eq!(
            get_model_family("anthropic/claude-opus-4.5"),
            Some(ModelFamily::ClaudeOpus4_5)
        );
        assert_eq!(
            get_model_family("anthropic/claude-sonnet-4"),
            Some(ModelFamily::ClaudeSonnet4)
        );

        // Unknown model
        assert_eq!(get_model_family("gpt-4"), None);
    }

    #[test]
    fn test_model_translation_same_provider() {
        // Same provider should return the same ID
        let translated = translate_model_id(
            "anthropic/claude-opus-4.5",
            ProviderId::OpenRouter,
            ProviderId::OpenRouter,
        );
        assert_eq!(translated, Some("anthropic/claude-opus-4.5".to_string()));
    }

    #[test]
    fn test_model_translation_unknown_model() {
        // Unknown model should return None
        let translated = translate_model_id("gpt-4", ProviderId::OpenRouter, ProviderId::MiniMax);
        assert_eq!(translated, None);
    }

    #[test]
    fn test_translate_model_or_default() {
        // Unknown model: fallback to provider default
        let result = translate_model_or_default("GLM-5", ProviderId::ZAi, ProviderId::MiniMax);
        assert_eq!(result, "MiniMax-M2.5");
    }

    #[test]
    fn test_provider_capabilities() {
        let openrouter = ProviderCapabilities::for_provider(ProviderId::OpenRouter);
        assert!(!openrouter.web_search);
        assert!(!openrouter.web_fetch);
        assert!(openrouter.web_plugins);
        assert!(openrouter.supports_vision);

        let zai = ProviderCapabilities::for_provider(ProviderId::ZAi);
        assert!(!zai.web_search);
        assert!(!zai.web_plugins);
        assert!(!zai.supports_vision);

        let anthropic = ProviderCapabilities::for_provider(ProviderId::Anthropic);
        assert!(!anthropic.web_search);
        assert!(anthropic.prompt_caching);
        assert!(!anthropic.web_plugins);
        assert!(anthropic.supports_vision);

        let openai = ProviderCapabilities::for_provider(ProviderId::OpenAI);
        assert!(!openai.web_search);
        assert!(!openai.web_plugins);
        assert!(openai.supports_vision);

        let minimax = ProviderCapabilities::for_provider(ProviderId::MiniMax);
        assert!(!minimax.web_search);
        assert!(!minimax.web_plugins);
        assert!(!minimax.supports_vision);
    }

    #[test]
    fn test_oauth_support() {
        use crate::auth::AuthMethod;

        // OpenAI supports OAuth
        assert!(ProviderId::OpenAI.supports_oauth());
        let openai_methods = ProviderId::OpenAI.auth_methods();
        assert!(openai_methods.contains(&AuthMethod::OAuthBrowser));
        assert!(openai_methods.contains(&AuthMethod::OAuthDevice));
        assert!(openai_methods.contains(&AuthMethod::ApiKey));

        // Anthropic supports OAuth (browser + API key, no device code)
        assert!(ProviderId::Anthropic.supports_oauth());
        let anthropic_methods = ProviderId::Anthropic.auth_methods();
        assert!(anthropic_methods.contains(&AuthMethod::OAuthBrowser));
        assert!(!anthropic_methods.contains(&AuthMethod::OAuthDevice));
        assert!(anthropic_methods.contains(&AuthMethod::ApiKey));

        // MiniMax doesn't support OAuth
        assert!(!ProviderId::MiniMax.supports_oauth());
        let minimax_methods = ProviderId::MiniMax.auth_methods();
        assert_eq!(minimax_methods, vec![AuthMethod::ApiKey]);
    }

    #[test]
    fn test_openai_config() {
        let provider = get_provider(ProviderId::OpenAI).unwrap();
        assert_eq!(provider.name, "OpenAI");
        assert_eq!(
            provider.base_url,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(provider.auth_header, AuthHeader::Bearer);
        assert!(provider.supports_tools);
        assert!(provider.dynamic_models);
        assert!(!provider.models.is_empty());
    }
}
