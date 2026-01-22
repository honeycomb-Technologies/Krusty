//! AI provider configuration
//!
//! Defines provider types, configurations, and built-in provider registry
//! for Anthropic-compatible API endpoints.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use crate::ai::models::ApiFormat;
use crate::auth::OpenAIAuthType;

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
    Anthropic,
    OpenRouter,
    OpenCodeZen,
    ZAi,
    MiniMax,
    Kimi,
    OpenAI,
}

impl ProviderId {
    /// Get all available provider IDs
    /// Order: Anthropic first (default), then smallest to largest, OpenRouter last
    pub fn all() -> &'static [ProviderId] {
        &[
            ProviderId::Anthropic,   // Default provider, always first
            ProviderId::OpenAI,      // OpenAI direct (OAuth or API key)
            ProviderId::MiniMax,     // 3 models
            ProviderId::Kimi,        // 2 models
            ProviderId::ZAi,         // 2 models
            ProviderId::OpenCodeZen, // 11 models
            ProviderId::OpenRouter,  // 100+ dynamic models, always last
        ]
    }

    /// Get the storage key for this provider (used in credentials.json)
    pub fn storage_key(&self) -> &'static str {
        match self {
            ProviderId::Anthropic => "anthropic",
            ProviderId::OpenRouter => "openrouter",
            ProviderId::OpenCodeZen => "opencode_zen",
            ProviderId::ZAi => "z_ai",
            ProviderId::MiniMax => "minimax",
            ProviderId::Kimi => "kimi",
            ProviderId::OpenAI => "openai",
        }
    }

    /// Check if this provider supports OAuth authentication
    pub fn supports_oauth(&self) -> bool {
        matches!(self, ProviderId::OpenAI)
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
            _ => vec![AuthMethod::ApiKey],
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderId::Anthropic => write!(f, "Anthropic"),
            ProviderId::OpenRouter => write!(f, "OpenRouter"),
            ProviderId::OpenCodeZen => write!(f, "OpenCode Zen"),
            ProviderId::ZAi => write!(f, "Z.ai"),
            ProviderId::MiniMax => write!(f, "MiniMax"),
            ProviderId::Kimi => write!(f, "Kimi"),
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
    /// Anthropic Claude: `thinking.budget_tokens` (we use max: 32000)
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
    /// Custom headers to send with requests (e.g., User-Agent for Kimi)
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
                ProviderId::OpenRouter => "anthropic/claude-sonnet-4",
                ProviderId::OpenCodeZen => "claude-sonnet-4-5",
                _ => "claude-opus-4-5-20251101", // Ultimate fallback
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
}

// ============================================================================
// Model Mapping System
// ============================================================================

/// Canonical model families that exist across providers
/// Maps to provider-specific IDs for seamless switching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
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
            // Claude Opus 4.5
            (
                ModelFamily::ClaudeOpus4_5,
                ProviderId::Anthropic,
                "claude-opus-4-5-20251101",
            ),
            (
                ModelFamily::ClaudeOpus4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4.5",
            ),
            (
                ModelFamily::ClaudeOpus4_5,
                ProviderId::OpenCodeZen,
                "claude-opus-4-5",
            ),
            // Claude Sonnet 4.5
            (
                ModelFamily::ClaudeSonnet4_5,
                ProviderId::Anthropic,
                "claude-sonnet-4-5-20250929",
            ),
            (
                ModelFamily::ClaudeSonnet4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-4.5",
            ),
            (
                ModelFamily::ClaudeSonnet4_5,
                ProviderId::OpenCodeZen,
                "claude-sonnet-4-5",
            ),
            // Claude Sonnet 4
            (
                ModelFamily::ClaudeSonnet4,
                ProviderId::Anthropic,
                "claude-sonnet-4-20250514",
            ),
            (
                ModelFamily::ClaudeSonnet4,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-4",
            ),
            (
                ModelFamily::ClaudeSonnet4,
                ProviderId::OpenCodeZen,
                "claude-sonnet-4",
            ),
            // Claude Haiku 4.5
            (
                ModelFamily::ClaudeHaiku4_5,
                ProviderId::Anthropic,
                "claude-haiku-4-5-20251001",
            ),
            (
                ModelFamily::ClaudeHaiku4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-haiku-4.5",
            ),
            (
                ModelFamily::ClaudeHaiku4_5,
                ProviderId::OpenCodeZen,
                "claude-haiku-4-5",
            ),
            // Claude Opus 4
            (
                ModelFamily::ClaudeOpus4,
                ProviderId::Anthropic,
                "claude-opus-4-20250514",
            ),
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
            .unwrap_or_else(|| "claude-opus-4-5-20251101".to_string())
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
}

impl ProviderCapabilities {
    /// Get capabilities for a provider
    pub fn for_provider(provider: ProviderId) -> Self {
        match provider {
            ProviderId::Anthropic => Self {
                web_search: true,
                web_fetch: true,
                context_management: true,
                prompt_caching: true,
                web_plugins: false,
            },
            ProviderId::OpenRouter => Self {
                web_search: false, // Not via server tools
                web_fetch: false,
                context_management: false,
                prompt_caching: false,
                web_plugins: true, // Uses plugins array
            },
            ProviderId::OpenCodeZen => Self {
                web_search: true, // Supports Anthropic's web_search_20250305
                web_fetch: true,  // Supports Anthropic's web_fetch tool
                context_management: false,
                prompt_caching: false, // Unclear if supported
                web_plugins: false,
            },
            // OpenAI: supports tools but not server-executed web search
            ProviderId::OpenAI => Self {
                web_search: false,
                web_fetch: false,
                context_management: false,
                prompt_caching: false,
                web_plugins: false,
            },
            // Other providers: minimal capabilities
            ProviderId::ZAi | ProviderId::MiniMax | ProviderId::Kimi => Self::default(),
        }
    }
}

/// Lazily initialized built-in provider configurations
static BUILTIN_PROVIDERS: LazyLock<Vec<ProviderConfig>> = LazyLock::new(|| {
    vec![
        // Anthropic - the default provider
        ProviderConfig {
            id: ProviderId::Anthropic,
            name: "Anthropic".to_string(),
            description: "Claude models (Opus, Sonnet, Haiku)".to_string(),
            base_url: "https://api.anthropic.com/v1/messages".to_string(),
            auth_header: AuthHeader::XApiKey,
            models: vec![
                ModelInfo::new(
                    "claude-opus-4-5-20251101",
                    "Claude Opus 4.5",
                    200_000,
                    16_384,
                )
                .with_anthropic_thinking(),
                ModelInfo::new(
                    "claude-sonnet-4-5-20250929",
                    "Claude Sonnet 4.5",
                    1_000_000, // Sonnet 4.5 has 1M context
                    16_384,
                )
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
                ModelInfo::new("openai/gpt-5.2", "GPT-5.2", 400_000, 128_000),
                ModelInfo::new(
                    "openai/gpt-5.2-instant",
                    "GPT-5.2 Instant",
                    400_000,
                    128_000,
                ),
                ModelInfo::new(
                    "openai/gpt-5.2-thinking",
                    "GPT-5.2 Thinking",
                    400_000,
                    128_000,
                ),
                ModelInfo::new("openai/gpt-5.2-pro", "GPT-5.2 Pro", 400_000, 128_000),
                ModelInfo::new("openai/gpt-5.2-codex", "GPT-5.2 Codex", 400_000, 128_000),
                ModelInfo::new("openai/o3", "OpenAI o3", 200_000, 100_000),
                ModelInfo::new("openai/o4-mini", "OpenAI o4-mini", 200_000, 100_000),
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
        // OpenCode Zen - curated models for coding agents (Anthropic-compatible)
        ProviderConfig {
            id: ProviderId::OpenCodeZen,
            name: "OpenCode Zen".to_string(),
            description: "Curated coding models (Claude, GPT-5, Gemini, Qwen)".to_string(),
            base_url: "https://opencode.ai/zen/v1/messages".to_string(),
            auth_header: AuthHeader::XApiKey, // Uses x-api-key, not Bearer
            models: vec![
                // Claude models
                ModelInfo::new("claude-opus-4-5", "Claude Opus 4.5", 200_000, 16_384)
                    .with_anthropic_thinking(),
                ModelInfo::new("claude-sonnet-4-5", "Claude Sonnet 4.5", 1_000_000, 16_384)
                    .with_anthropic_thinking(),
                ModelInfo::new("claude-sonnet-4", "Claude Sonnet 4", 200_000, 8_192),
                ModelInfo::new("claude-haiku-4-5", "Claude Haiku 4.5", 200_000, 16_384),
                // GPT models
                ModelInfo::new("gpt-5.2", "GPT-5.2", 400_000, 128_000),
                ModelInfo::new("gpt-5.2-instant", "GPT-5.2 Instant", 400_000, 128_000),
                ModelInfo::new("gpt-5.2-thinking", "GPT-5.2 Thinking", 400_000, 128_000),
                ModelInfo::new("gpt-5.2-codex", "GPT-5.2 Codex", 400_000, 128_000),
                // Gemini models
                ModelInfo::new("gemini-2.5-pro", "Gemini 2.5 Pro", 1_000_000, 65_536),
                ModelInfo::new("gemini-2.5-flash", "Gemini 2.5 Flash", 1_000_000, 65_536),
                // Qwen models
                ModelInfo::new("qwen-coder-plus", "Qwen Coder Plus", 128_000, 8_192),
                ModelInfo::new("qwen-max", "Qwen Max", 128_000, 8_192),
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
            description: "GLM Coding Plan (GLM-4.7, ~3x Claude usage)".to_string(),
            base_url: "https://api.z.ai/api/anthropic/v1/messages".to_string(),
            auth_header: AuthHeader::XApiKey,
            models: vec![
                ModelInfo::new("GLM-4.7", "GLM 4.7", 128_000, 16_384),
                ModelInfo::new("GLM-4.5-Air", "GLM 4.5 Air", 128_000, 16_384),
            ],
            supports_tools: true,
            dynamic_models: false,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
        // MiniMax - M2 models (Anthropic-compatible API)
        ProviderConfig {
            id: ProviderId::MiniMax,
            name: "MiniMax".to_string(),
            description: "M2 models (fast, interleaved thinking)".to_string(),
            base_url: "https://api.minimax.io/anthropic/v1/messages".to_string(),
            auth_header: AuthHeader::XApiKey,
            models: vec![
                // Lightning: Fastest (100 tps)
                ModelInfo::new(
                    "MiniMax-M2.1-lightning",
                    "MiniMax M2.1 Lightning",
                    200_000,
                    64_000,
                )
                .with_anthropic_thinking(),
                // Standard M2.1: Balanced (60 tps)
                ModelInfo::new("MiniMax-M2.1", "MiniMax M2.1", 200_000, 64_000)
                    .with_anthropic_thinking(),
                // M2: Advanced reasoning
                ModelInfo::new("MiniMax-M2", "MiniMax M2", 200_000, 64_000)
                    .with_anthropic_thinking(),
            ],
            supports_tools: true,
            dynamic_models: false,
            pricing_hint: None,
            custom_headers: HashMap::new(),
        },
        // Kimi Code - Coding agent API (OpenAI-compatible format)
        // API: api.kimi.com/coding/v1 (requires KimiCLI User-Agent)
        // Auth: Bearer token
        // Note: sk-kimi-* keys are for api.kimi.com, not api.moonshot.ai
        ProviderConfig {
            id: ProviderId::Kimi,
            name: "Kimi".to_string(),
            description: "Kimi Code (262K context, coding agent)".to_string(),
            base_url: "https://api.kimi.com/coding/v1/chat/completions".to_string(),
            auth_header: AuthHeader::Bearer,
            models: vec![
                // kimi-for-coding: 262K context, supports reasoning
                ModelInfo::new("kimi-for-coding", "Kimi For Coding", 262_144, 16_384)
                    .with_anthropic_thinking(), // Uses reasoning mode
            ],
            supports_tools: true,
            dynamic_models: false,
            pricing_hint: None,
            custom_headers: HashMap::from([
                // Required: Kimi Code API checks User-Agent for coding agent access
                ("User-Agent".to_string(), "KimiCLI/1.0".to_string()),
            ]),
        },
        // OpenAI - Direct access with OAuth or API key (OpenAI-compatible format)
        // Supports OAuth browser flow, device code flow, and API key authentication
        ProviderConfig {
            id: ProviderId::OpenAI,
            name: "OpenAI".to_string(),
            description: "GPT-5.2, o3, Codex (OAuth or API key)".to_string(),
            base_url: "https://api.openai.com/v1/chat/completions".to_string(),
            auth_header: AuthHeader::Bearer,
            models: vec![
                // GPT-5.2 family
                ModelInfo::new("gpt-5.2", "GPT-5.2", 400_000, 128_000),
                ModelInfo::new("gpt-5.2-codex", "GPT-5.2 Codex", 400_000, 128_000),
                ModelInfo::new("gpt-5.2-instant", "GPT-5.2 Instant", 400_000, 128_000),
                ModelInfo::new("gpt-5.2-thinking", "GPT-5.2 Thinking", 400_000, 128_000),
                // o3/o4 reasoning models
                ModelInfo::new("o3", "OpenAI o3", 200_000, 100_000),
                ModelInfo::new("o4-mini", "OpenAI o4-mini", 200_000, 100_000),
                // GPT-4 family (still widely used)
                ModelInfo::new("gpt-4o", "GPT-4o", 128_000, 16_384),
                ModelInfo::new("gpt-4o-mini", "GPT-4o Mini", 128_000, 16_384),
            ],
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
        assert_eq!(ProviderId::Anthropic.to_string(), "Anthropic");
        assert_eq!(ProviderId::OpenRouter.to_string(), "OpenRouter");
        assert_eq!(ProviderId::ZAi.to_string(), "Z.ai");
        assert_eq!(ProviderId::OpenAI.to_string(), "OpenAI");
    }

    #[test]
    fn test_storage_keys() {
        assert_eq!(ProviderId::Anthropic.storage_key(), "anthropic");
        assert_eq!(ProviderId::ZAi.storage_key(), "z_ai");
        assert_eq!(ProviderId::OpenAI.storage_key(), "openai");
    }

    #[test]
    fn test_builtin_providers() {
        let providers = builtin_providers();
        assert_eq!(providers.len(), 7);
        assert!(providers.iter().any(|p| p.id == ProviderId::Anthropic));
        assert!(providers.iter().any(|p| p.id == ProviderId::OpenRouter));
        assert!(providers.iter().any(|p| p.id == ProviderId::OpenCodeZen));
        assert!(providers.iter().any(|p| p.id == ProviderId::OpenAI));
    }

    #[test]
    fn test_get_provider() {
        let anthropic = get_provider(ProviderId::Anthropic).unwrap();
        assert_eq!(anthropic.name, "Anthropic");
        assert!(!anthropic.models.is_empty());
    }

    #[test]
    fn test_anthropic_config() {
        let provider = get_provider(ProviderId::Anthropic).unwrap();
        assert_eq!(provider.base_url, "https://api.anthropic.com/v1/messages");
        assert_eq!(provider.auth_header, AuthHeader::XApiKey);
        assert_eq!(provider.default_model(), "claude-opus-4-5-20251101");
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
        let anthropic = get_provider(ProviderId::Anthropic).unwrap();
        // Valid Anthropic model
        assert!(anthropic.has_model("claude-opus-4-5-20251101"));
        // Invalid - OpenRouter format
        assert!(!anthropic.has_model("anthropic/claude-opus-4.5"));

        // OpenRouter allows any model (dynamic)
        let openrouter = get_provider(ProviderId::OpenRouter).unwrap();
        assert!(openrouter.has_model("anthropic/claude-opus-4.5"));
        assert!(openrouter.has_model("openai/gpt-4"));
    }

    #[test]
    fn test_model_family_detection() {
        // Anthropic format
        assert_eq!(
            get_model_family("claude-opus-4-5-20251101"),
            Some(ModelFamily::ClaudeOpus4_5)
        );
        assert_eq!(
            get_model_family("claude-sonnet-4-5-20250929"),
            Some(ModelFamily::ClaudeSonnet4_5)
        );

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
    fn test_model_translation_anthropic_to_openrouter() {
        let translated = translate_model_id(
            "claude-opus-4-5-20251101",
            ProviderId::Anthropic,
            ProviderId::OpenRouter,
        );
        assert_eq!(translated, Some("anthropic/claude-opus-4.5".to_string()));

        let translated = translate_model_id(
            "claude-sonnet-4-5-20250929",
            ProviderId::Anthropic,
            ProviderId::OpenRouter,
        );
        assert_eq!(translated, Some("anthropic/claude-sonnet-4.5".to_string()));
    }

    #[test]
    fn test_model_translation_openrouter_to_anthropic() {
        let translated = translate_model_id(
            "anthropic/claude-opus-4.5",
            ProviderId::OpenRouter,
            ProviderId::Anthropic,
        );
        assert_eq!(translated, Some("claude-opus-4-5-20251101".to_string()));

        let translated = translate_model_id(
            "anthropic/claude-haiku-4.5",
            ProviderId::OpenRouter,
            ProviderId::Anthropic,
        );
        assert_eq!(translated, Some("claude-haiku-4-5-20251001".to_string()));
    }

    #[test]
    fn test_model_translation_same_provider() {
        // Same provider should return the same ID
        let translated = translate_model_id(
            "claude-opus-4-5-20251101",
            ProviderId::Anthropic,
            ProviderId::Anthropic,
        );
        assert_eq!(translated, Some("claude-opus-4-5-20251101".to_string()));
    }

    #[test]
    fn test_model_translation_unknown_model() {
        // Unknown model should return None
        let translated = translate_model_id("gpt-4", ProviderId::OpenRouter, ProviderId::Anthropic);
        assert_eq!(translated, None);
    }

    #[test]
    fn test_translate_model_or_default() {
        // Known model: translate
        let result = translate_model_or_default(
            "claude-opus-4-5-20251101",
            ProviderId::Anthropic,
            ProviderId::OpenRouter,
        );
        assert_eq!(result, "anthropic/claude-opus-4.5");

        // Unknown model: fallback to provider default
        let result = translate_model_or_default("glm-4.7", ProviderId::ZAi, ProviderId::Anthropic);
        assert_eq!(result, "claude-opus-4-5-20251101");
    }

    #[test]
    fn test_provider_capabilities() {
        let anthropic = ProviderCapabilities::for_provider(ProviderId::Anthropic);
        assert!(anthropic.web_search);
        assert!(anthropic.web_fetch);
        assert!(!anthropic.web_plugins);

        let openrouter = ProviderCapabilities::for_provider(ProviderId::OpenRouter);
        assert!(!openrouter.web_search);
        assert!(!openrouter.web_fetch);
        assert!(openrouter.web_plugins);

        let zai = ProviderCapabilities::for_provider(ProviderId::ZAi);
        assert!(!zai.web_search);
        assert!(!zai.web_plugins);

        let openai = ProviderCapabilities::for_provider(ProviderId::OpenAI);
        assert!(!openai.web_search);
        assert!(!openai.web_plugins);
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

        // Anthropic doesn't support OAuth
        assert!(!ProviderId::Anthropic.supports_oauth());
        let anthropic_methods = ProviderId::Anthropic.auth_methods();
        assert_eq!(anthropic_methods, vec![AuthMethod::ApiKey]);
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
