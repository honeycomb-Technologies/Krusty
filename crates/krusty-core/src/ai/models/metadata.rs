use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use super::super::providers::{ProviderId, ReasoningFormat};

/// Metadata describing a cached dynamic model catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DynamicModelCacheMetadata {
    pub fetched_at: u64,
    pub ttl_seconds: u64,
    pub model_count: usize,
    pub fingerprint: u64,
}

/// API format for model requests
///
/// Different model families route to different provider endpoints based on format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    /// Anthropic Messages API (/v1/messages)
    #[default]
    Anthropic,
    /// OpenAI Chat Completions API (/v1/chat/completions)
    OpenAI,
    /// OpenAI Responses API (/v1/responses) - GPT-5 models
    OpenAIResponses,
    /// Google AI API (/v1/models/{model})
    Google,
}

/// Rich model metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    /// Unique model ID (e.g., "claude-opus-4-5-20251101")
    pub id: String,
    /// Human-readable name (e.g., "Claude Opus 4.5")
    pub display_name: String,
    /// Which provider offers this model
    pub provider: ProviderId,
    /// Maximum context window in tokens
    pub context_window: usize,
    /// Maximum output tokens
    pub max_output: usize,

    // Capabilities
    /// Supports extended thinking/reasoning (legacy boolean)
    pub supports_thinking: bool,
    /// Reasoning/thinking format (None = not supported, Some = supported with specific format)
    pub reasoning_format: Option<ReasoningFormat>,
    /// Supports function/tool calling
    pub supports_tools: bool,
    /// Supports image input (vision)
    pub supports_vision: bool,

    // Pricing (per million tokens, None if unknown)
    /// Input/prompt price per million tokens
    pub input_price: Option<f64>,
    /// Output/completion price per million tokens
    pub output_price: Option<f64>,

    // Provider-specific metadata
    /// Sub-provider for OpenRouter models (e.g., "anthropic", "openai")
    #[serde(default)]
    pub sub_provider: Option<String>,
    /// Whether this is a free model (OpenRouter :free suffix)
    #[serde(default)]
    pub is_free: bool,
    /// API format for this model, used for provider routing.
    #[serde(default)]
    pub api_format: ApiFormat,
}

impl ModelMetadata {
    /// Create basic model metadata
    pub fn new(id: &str, display_name: &str, provider: ProviderId) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            provider,
            context_window: 128_000,
            max_output: 4096,
            supports_thinking: false,
            reasoning_format: None,
            supports_tools: true,
            supports_vision: false,
            input_price: None,
            output_price: None,
            sub_provider: None,
            is_free: false,
            api_format: ApiFormat::default(),
        }
    }

    /// Builder: set context window
    pub fn with_context(mut self, context: usize, max_output: usize) -> Self {
        self.context_window = context;
        self.max_output = max_output;
        self
    }

    /// Builder: enable thinking support with specified format
    pub fn with_thinking(mut self, format: ReasoningFormat) -> Self {
        self.supports_thinking = true;
        self.reasoning_format = Some(format);
        self
    }

    /// Get pricing tier indicator for UI
    pub fn pricing_tier(&self) -> &'static str {
        match self.input_price {
            Some(p) if p < 0.5 => "¢",
            Some(p) if p < 3.0 => "$",
            Some(p) if p < 10.0 => "$$",
            Some(_) => "$$$",
            None => "",
        }
    }

    /// Format context window for display (e.g., "200K", "1M")
    pub fn context_display(&self) -> String {
        if self.context_window >= 1_000_000 {
            format!("{}M", self.context_window / 1_000_000)
        } else if self.context_window >= 1_000 {
            format!("{}K", self.context_window / 1_000)
        } else {
            format!("{}", self.context_window)
        }
    }
}

/// Provider-specific TTL for dynamic model catalog refresh.
pub fn dynamic_model_cache_ttl(provider: ProviderId) -> u64 {
    match provider {
        ProviderId::OpenAI => 6 * 60 * 60,
        ProviderId::OpenRouter => 12 * 60 * 60,
        _ => 24 * 60 * 60,
    }
}

/// Stable fingerprint for a model catalog, used to validate cached metadata.
pub fn model_catalog_fingerprint(models: &[ModelMetadata]) -> u64 {
    let mut items = models
        .iter()
        .map(|model| {
            format!(
                "{}|{}|{}|{}|{:?}|{:?}",
                model.id,
                model.display_name,
                model.context_window,
                model.max_output,
                model.provider,
                model.api_format
            )
        })
        .collect::<Vec<_>>();
    items.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for item in items {
        item.hash(&mut hasher);
    }
    hasher.finish()
}
