use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// ChatGPT backend API for OAuth users (Responses API).
/// This endpoint is required for tokens obtained via ChatGPT OAuth flow.
/// Note: ChatGPT's Codex API does NOT use /v1/ prefix unlike the standard OpenAI API.
pub const CHATGPT_RESPONSES_API: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Standard OpenAI API for API key users (Chat Completions).
/// This endpoint is used when authenticating with an API key.
pub const OPENAI_CHAT_API: &str = "https://api.openai.com/v1/chat/completions";
/// Standard OpenAI Responses API for API key users.
pub const OPENAI_RESPONSES_API: &str = "https://api.openai.com/v1/responses";

/// Unique identifier for each supported provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    #[default]
    MiniMax,
    OpenRouter,
    ZAi,
    Anthropic,
    OpenAI,
    Grok,
}

impl ProviderId {
    /// Get all available provider IDs.
    /// Order: MiniMax first (default), then smallest to largest, OpenRouter last.
    pub fn all() -> &'static [ProviderId] {
        &[
            ProviderId::MiniMax,
            ProviderId::Anthropic,
            ProviderId::OpenAI,
            ProviderId::Grok,
            ProviderId::ZAi,
            ProviderId::OpenRouter,
        ]
    }

    /// Get the storage key for this provider (used in credentials.json).
    pub fn storage_key(&self) -> &'static str {
        match self {
            ProviderId::MiniMax => "minimax",
            ProviderId::OpenRouter => "openrouter",
            ProviderId::ZAi => "z_ai",
            ProviderId::Anthropic => "anthropic",
            ProviderId::OpenAI => "openai",
            ProviderId::Grok => "grok",
        }
    }

    /// Check if this provider supports OAuth authentication.
    pub fn supports_oauth(&self) -> bool {
        matches!(
            self,
            ProviderId::OpenAI | ProviderId::Anthropic | ProviderId::Grok
        )
    }

    /// Get the authentication methods supported by this provider.
    pub fn auth_methods(&self) -> Vec<crate::auth::AuthMethod> {
        use crate::auth::AuthMethod;
        match self {
            ProviderId::OpenAI => vec![
                AuthMethod::OAuthBrowser,
                AuthMethod::OAuthDevice,
                AuthMethod::ApiKey,
            ],
            ProviderId::Anthropic => vec![AuthMethod::OAuthBrowser, AuthMethod::ApiKey],
            ProviderId::Grok => vec![AuthMethod::OAuthBrowser],
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
            ProviderId::Grok => write!(f, "Grok"),
        }
    }
}

/// How to send the API key in requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AuthHeader {
    /// Use `x-api-key: <key>` header (Anthropic style).
    #[default]
    XApiKey,
    /// Use `Authorization: Bearer <key>` header (OpenAI style).
    Bearer,
}

/// Different reasoning/thinking formats used by various providers.
/// When enabled, the request layer maps the active UI effort to provider wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningFormat {
    /// Anthropic Claude: `thinking.budget_tokens`.
    Anthropic,
    /// OpenAI o1/o3/GPT-5 and Grok Build: OpenAI-style `reasoning` / `reasoning_effort`.
    OpenAI,
    /// DeepSeek R1: `reasoning.enabled: true`.
    DeepSeek,
}

/// Information about a model offered by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model ID to send in API requests.
    pub id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Context window size in tokens.
    pub context_window: usize,
    /// Maximum output tokens.
    pub max_output: usize,
    /// Reasoning/thinking support (None = not supported).
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

    pub fn with_reasoning(mut self, reasoning: ReasoningFormat) -> Self {
        self.reasoning = Some(reasoning);
        self
    }

    /// Add Anthropic-style extended thinking support.
    pub fn with_anthropic_thinking(self) -> Self {
        self.with_reasoning(ReasoningFormat::Anthropic)
    }
}

/// Configuration for an AI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique identifier.
    pub id: ProviderId,
    /// Display name.
    pub name: String,
    /// Short description for UI.
    pub description: String,
    /// API base URL (without trailing slash).
    pub base_url: String,
    /// How to send authentication.
    pub auth_header: AuthHeader,
    /// Available models (empty for dynamic providers like OpenRouter).
    pub models: Vec<ModelInfo>,
    /// Whether this provider supports tool calling.
    pub supports_tools: bool,
    /// Whether models can have dynamic list (fetched from API).
    pub dynamic_models: bool,
    /// Pricing hint to show in UI.
    pub pricing_hint: Option<String>,
    /// Custom headers to send with requests.
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
}

impl ProviderConfig {
    /// Get the default model ID for this provider.
    /// Returns the first model in the list, or a hardcoded fallback for dynamic providers.
    pub fn default_model(&self) -> &str {
        if let Some(first) = self.models.first() {
            &first.id
        } else {
            match self.id {
                ProviderId::OpenRouter => "openai/gpt-5-codex",
                ProviderId::OpenAI => "gpt-5.5",
                ProviderId::Grok => "grok-build",
                _ => "MiniMax-M2.5",
            }
        }
    }

    /// Check if a model ID is valid for this provider.
    pub fn has_model(&self, model_id: &str) -> bool {
        if self.dynamic_models {
            return true;
        }
        self.models.iter().any(|m| m.id == model_id)
    }
}
