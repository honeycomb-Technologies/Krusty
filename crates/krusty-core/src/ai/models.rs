//! Model metadata and registry
//!
//! Central management for AI models from all providers.
//! Supports static models (built-in) and dynamic models (fetched from APIs).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::providers::{ProviderId, ReasoningFormat};

/// API format for model requests
///
/// OpenCode Zen routes different models to different endpoints based on format.
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
    /// API format for this model (used by OpenCode Zen for routing)
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
            Some(p) if p < 0.5 => "¢",   // Cheap
            Some(p) if p < 3.0 => "$",   // Medium
            Some(p) if p < 10.0 => "$$", // Expensive
            Some(_) => "$$$",            // Very expensive
            None => "",                  // Unknown
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

/// Central model registry
///
/// Thread-safe store for all models from all providers.
/// Supports both static (built-in) and dynamic (fetched) models.
pub struct ModelRegistry {
    /// All models indexed by provider
    models: RwLock<HashMap<ProviderId, Vec<ModelMetadata>>>,

    /// Recently used model IDs (most recent first)
    recent_ids: RwLock<Vec<String>>,

    /// Maximum recent models to track
    max_recent: usize,
}

impl ModelRegistry {
    /// Create new empty registry
    pub fn new() -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
            recent_ids: RwLock::new(Vec::new()),
            max_recent: 10,
        }
    }

    /// Set models for a provider (replaces existing)
    pub async fn set_models(&self, provider: ProviderId, models: Vec<ModelMetadata>) {
        let mut all_models = self.models.write().await;
        all_models.insert(provider, models);
    }

    /// Check if we have models for a provider
    pub async fn has_models(&self, provider: ProviderId) -> bool {
        let models = self.models.read().await;
        models
            .get(&provider)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    }

    /// Get a specific model by ID (searches all providers)
    pub async fn get_model(&self, model_id: &str) -> Option<ModelMetadata> {
        let models = self.models.read().await;
        models
            .values()
            .flat_map(|v| v.iter())
            .find(|m| m.id == model_id)
            .cloned()
    }

    /// Get a specific model by ID (non-blocking, for use in sync contexts like rendering)
    /// Returns None if lock is contended or model not found
    pub fn try_get_model(&self, model_id: &str) -> Option<ModelMetadata> {
        let models = self.models.try_read().ok()?;
        for provider_models in models.values() {
            if let Some(model) = provider_models.iter().find(|m| m.id == model_id) {
                return Some(model.clone());
            }
        }
        None
    }

    /// Record a model as recently used
    pub async fn mark_recent(&self, model_id: &str) {
        let mut recent = self.recent_ids.write().await;

        // Remove if already exists (will re-add at front)
        recent.retain(|id| id != model_id);

        // Add at front
        recent.insert(0, model_id.to_string());

        // Trim to max
        recent.truncate(self.max_recent);
    }

    /// Set recent model IDs (for loading from preferences)
    pub async fn set_recent_ids(&self, ids: Vec<String>) {
        let mut recent = self.recent_ids.write().await;
        *recent = ids;
        recent.truncate(self.max_recent);
    }

    /// Get models organized for display
    /// Returns: (recent_models, models_by_provider)
    pub async fn get_organized_models(
        &self,
        configured_providers: &[ProviderId],
    ) -> (Vec<ModelMetadata>, HashMap<ProviderId, Vec<ModelMetadata>>) {
        let models = self.models.read().await;
        let recent_ids = self.recent_ids.read().await;

        // Collect recent models
        let mut recent_models = Vec::new();
        for id in recent_ids.iter() {
            for provider_models in models.values() {
                if let Some(model) = provider_models.iter().find(|m| &m.id == id) {
                    // Only include if provider is configured
                    if configured_providers.contains(&model.provider) {
                        recent_models.push(model.clone());
                    }
                    break;
                }
            }
        }

        // Collect models by provider (only configured)
        let mut by_provider = HashMap::new();
        for provider in configured_providers {
            if let Some(provider_models) = models.get(provider) {
                if !provider_models.is_empty() {
                    by_provider.insert(*provider, provider_models.clone());
                }
            }
        }

        (recent_models, by_provider)
    }

    /// Get models organized for display (non-blocking)
    /// Returns None if locks are contended
    pub fn try_get_organized_models(
        &self,
        configured_providers: &[ProviderId],
    ) -> Option<(Vec<ModelMetadata>, HashMap<ProviderId, Vec<ModelMetadata>>)> {
        let models = self.models.try_read().ok()?;
        let recent_ids = self.recent_ids.try_read().ok()?;

        let mut recent_models = Vec::new();
        for id in recent_ids.iter() {
            for provider_models in models.values() {
                if let Some(model) = provider_models.iter().find(|m| &m.id == id) {
                    if configured_providers.contains(&model.provider) {
                        recent_models.push(model.clone());
                    }
                    break;
                }
            }
        }

        let mut by_provider = HashMap::new();
        for provider in configured_providers {
            if let Some(provider_models) = models.get(provider) {
                if !provider_models.is_empty() {
                    by_provider.insert(*provider, provider_models.clone());
                }
            }
        }

        Some((recent_models, by_provider))
    }

    /// Check if provider has models (non-blocking)
    pub fn try_has_models(&self, provider: ProviderId) -> Option<bool> {
        let models = self.models.try_read().ok()?;
        Some(
            models
                .get(&provider)
                .map(|m| !m.is_empty())
                .unwrap_or(false),
        )
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared model registry type
pub type SharedModelRegistry = Arc<ModelRegistry>;

/// Create a new shared model registry
pub fn create_model_registry() -> SharedModelRegistry {
    Arc::new(ModelRegistry::new())
}
