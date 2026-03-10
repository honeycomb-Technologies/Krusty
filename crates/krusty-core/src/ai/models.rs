//! Model metadata and registry
//!
//! Central management for AI models from all providers.
//! Supports static models (built-in) and dynamic models (fetched from APIs).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::providers::{ProviderId, ReasoningFormat};
use crate::constants;

pub type ModelsByProvider = HashMap<ProviderId, Vec<ModelMetadata>>;
pub type OrganizedModels = (Vec<ModelMetadata>, ModelsByProvider);

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

/// Infer minimal metadata for an arbitrary model ID.
///
/// This is used when a user selects a model that is not present in the cached
/// catalog but should still be usable with the provider.
pub fn infer_model_metadata(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> ModelMetadata {
    let normalized = model_id.trim().to_ascii_lowercase();
    let context_window = resolve_context_window(provider, model_id, api_format);
    let max_output = if context_window >= 1_000_000 {
        65_536
    } else if context_window >= 400_000 {
        128_000
    } else if context_window >= 200_000 {
        100_000
    } else {
        32_768
    };

    let mut metadata =
        ModelMetadata::new(model_id, model_id, provider).with_context(context_window, max_output);

    metadata.api_format = if provider == ProviderId::OpenAI
        && crate::ai::providers::ProviderConfig::openai_prefers_responses_api(model_id)
    {
        ApiFormat::OpenAIResponses
    } else {
        api_format
    };

    metadata.reasoning_format =
        if normalized.contains("claude") || normalized.starts_with("anthropic/") {
            Some(ReasoningFormat::Anthropic)
        } else if normalized.contains("deepseek") {
            Some(ReasoningFormat::DeepSeek)
        } else if crate::ai::providers::ProviderConfig::openai_prefers_responses_api(model_id) {
            Some(ReasoningFormat::OpenAI)
        } else {
            None
        };
    metadata.supports_thinking = metadata.reasoning_format.is_some();
    metadata.supports_tools = true;
    metadata.supports_vision = normalized.contains("gpt-4o")
        || normalized.contains("gpt-4.1")
        || normalized.contains("gpt-5")
        || normalized.contains("gpt-6")
        || normalized.contains("gemini")
        || normalized.contains("claude");

    metadata
}

/// Resolve best-known metadata for a model by checking built-in provider catalog first,
/// then falling back to heuristic inference.
pub fn resolve_model_metadata(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> ModelMetadata {
    if let Some(provider_config) = crate::ai::providers::get_provider(provider) {
        if let Some(model) = provider_config
            .models
            .iter()
            .find(|candidate| candidate.id.eq_ignore_ascii_case(model_id))
        {
            let mut metadata = ModelMetadata::new(&model.id, &model.display_name, provider)
                .with_context(model.context_window, model.max_output);
            metadata.supports_tools = true;
            metadata.reasoning_format = model.reasoning;
            metadata.supports_thinking = model.reasoning.is_some();
            metadata.supports_vision =
                infer_model_metadata(provider, model_id, api_format).supports_vision;
            metadata.api_format = if provider == ProviderId::OpenAI
                && crate::ai::providers::ProviderConfig::openai_prefers_responses_api(&model.id)
            {
                ApiFormat::OpenAIResponses
            } else {
                api_format
            };
            return metadata;
        }
    }

    infer_model_metadata(provider, model_id, api_format)
}

/// Resolve the best-known context window for a model.
///
/// Order of preference:
/// 1. exact built-in provider metadata
/// 2. heuristic inference from model ID / family
/// 3. global default fallback
pub fn resolve_context_window(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> usize {
    if let Some(provider_config) = crate::ai::providers::get_provider(provider) {
        if let Some(model) = provider_config
            .models
            .iter()
            .find(|model| model.id.eq_ignore_ascii_case(model_id))
        {
            return model.context_window;
        }
    }

    infer_context_window(model_id, api_format).unwrap_or(constants::ai::CONTEXT_WINDOW_TOKENS)
}

fn infer_context_window(model_id: &str, api_format: ApiFormat) -> Option<usize> {
    let normalized = model_id.trim().to_ascii_lowercase();
    let id = normalized.strip_prefix("openai/").unwrap_or(&normalized);

    if normalized.contains("claude-sonnet-4.5") || normalized.contains("gemini-2.5") {
        return Some(1_000_000);
    }

    if normalized.contains("gemini")
        || normalized.contains("llama-4-maverick")
        || normalized.contains("gemini-2.0")
    {
        return Some(1_000_000);
    }

    if normalized.contains("llama-4-scout") {
        return Some(512_000);
    }

    if normalized.contains("codex") || gpt_major_version(id).is_some_and(|major| major >= 5) {
        return Some(400_000);
    }

    if id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || normalized.contains("gpt-4.1")
        || matches!(api_format, ApiFormat::OpenAIResponses)
    {
        return Some(200_000);
    }

    if normalized.contains("claude") || normalized.contains("glm-5") {
        return Some(200_000);
    }

    if normalized.contains("qwen") || normalized.contains("deepseek") {
        return Some(128_000);
    }

    if normalized.contains("gpt-4") || normalized.contains("gpt-3.5") || normalized.contains("grok")
    {
        return Some(128_000);
    }

    None
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}

/// Central model registry
///
/// Thread-safe store for all models from all providers.
/// Supports both static (built-in) and dynamic (fetched) models.
pub struct ModelRegistry {
    /// All models indexed by provider
    models: RwLock<HashMap<ProviderId, Vec<ModelMetadata>>>,

    /// Index for O(1) model lookup by ID -> (provider, index in provider's vec)
    model_index: RwLock<HashMap<String, (ProviderId, usize)>>,

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
            model_index: RwLock::new(HashMap::new()),
            recent_ids: RwLock::new(Vec::new()),
            max_recent: 10,
        }
    }

    /// Set models for a provider (replaces existing)
    pub async fn set_models(&self, provider: ProviderId, models: Vec<ModelMetadata>) {
        let mut all_models = self.models.write().await;
        let mut index = self.model_index.write().await;

        // Remove old index entries for this provider
        index.retain(|_, (p, _)| *p != provider);

        // Add new index entries
        for (idx, model) in models.iter().enumerate() {
            index.insert(model.id.clone(), (provider, idx));
        }

        all_models.insert(provider, models);
    }

    /// Insert or replace a single model entry for its provider.
    ///
    /// This keeps custom/manual model IDs first-class in the registry so
    /// recents and context-window lookup behave consistently.
    pub async fn upsert_model(&self, metadata: ModelMetadata) {
        let provider = metadata.provider;
        let model_id = metadata.id.clone();

        let mut all_models = self.models.write().await;
        let mut index = self.model_index.write().await;
        let provider_models = all_models.entry(provider).or_default();

        if let Some((existing_provider, existing_idx)) = index.get(&model_id).copied() {
            if existing_provider == provider {
                if let Some(slot) = provider_models.get_mut(existing_idx) {
                    *slot = metadata;
                    return;
                }
            }
        }

        let insert_idx = provider_models.len();
        provider_models.push(metadata);
        index.insert(model_id, (provider, insert_idx));
    }

    /// Check if we have models for a provider
    pub async fn has_models(&self, provider: ProviderId) -> bool {
        let models = self.models.read().await;
        models
            .get(&provider)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    }

    /// Get a specific model by ID - O(1) lookup via index
    pub async fn get_model(&self, model_id: &str) -> Option<ModelMetadata> {
        let index = self.model_index.read().await;
        let (provider, idx) = index.get(model_id)?;

        let models = self.models.read().await;
        models.get(provider).and_then(|v| v.get(*idx)).cloned()
    }

    /// Get a specific model by ID (non-blocking, for use in sync contexts like rendering)
    /// Returns None if lock is contended or model not found - O(1) lookup via index
    pub fn try_get_model(&self, model_id: &str) -> Option<ModelMetadata> {
        let index = self.model_index.try_read().ok()?;
        let (provider, idx) = index.get(model_id)?;

        let models = self.models.try_read().ok()?;
        models.get(provider).and_then(|v| v.get(*idx)).cloned()
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

    /// Get models organized for display - O(n) for recent models via index
    /// Returns: (recent_models, models_by_provider)
    pub async fn get_organized_models(
        &self,
        configured_providers: &[ProviderId],
    ) -> OrganizedModels {
        let models = self.models.read().await;
        let index = self.model_index.read().await;
        let recent_ids = self.recent_ids.read().await;

        // Collect recent models using index for O(1) lookup per ID
        let recent_models: Vec<ModelMetadata> = recent_ids
            .iter()
            .filter_map(|id| {
                let (provider, idx) = index.get(id)?;
                let model = models.get(provider)?.get(*idx)?;
                // Only include if provider is configured
                if configured_providers.contains(&model.provider) {
                    Some(model.clone())
                } else {
                    None
                }
            })
            .collect();

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

    /// Get models organized for display (non-blocking) - O(n) for recent models via index
    /// Returns None if locks are contended
    pub fn try_get_organized_models(
        &self,
        configured_providers: &[ProviderId],
    ) -> Option<OrganizedModels> {
        let models = self.models.try_read().ok()?;
        let index = self.model_index.try_read().ok()?;
        let recent_ids = self.recent_ids.try_read().ok()?;

        // Collect recent models using index for O(1) lookup per ID
        let recent_models: Vec<ModelMetadata> = recent_ids
            .iter()
            .filter_map(|id| {
                let (provider, idx) = index.get(id)?;
                let model = models.get(provider)?.get(*idx)?;
                if configured_providers.contains(&model.provider) {
                    Some(model.clone())
                } else {
                    None
                }
            })
            .collect();

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

#[cfg(test)]
mod tests {
    use super::{infer_model_metadata, resolve_context_window, resolve_model_metadata, ApiFormat};
    use crate::ai::providers::{ProviderId, ReasoningFormat};

    #[test]
    fn resolves_exact_static_context_window() {
        assert_eq!(
            resolve_context_window(
                ProviderId::Anthropic,
                "claude-opus-4-6",
                ApiFormat::Anthropic
            ),
            200_000
        );
    }

    #[test]
    fn infers_future_openai_reasoning_model_window() {
        assert_eq!(
            resolve_context_window(
                ProviderId::OpenAI,
                "gpt-6.4-codex",
                ApiFormat::OpenAIResponses
            ),
            400_000
        );
    }

    #[test]
    fn infers_gemini_window_for_dynamic_model_ids() {
        assert_eq!(
            resolve_context_window(
                ProviderId::OpenRouter,
                "google/gemini-2.5-pro",
                ApiFormat::Anthropic,
            ),
            1_000_000
        );
    }

    #[test]
    fn infers_custom_openai_metadata_for_manual_model_ids() {
        let metadata =
            infer_model_metadata(ProviderId::OpenAI, "gpt-6.4", ApiFormat::OpenAIResponses);

        assert_eq!(metadata.id, "gpt-6.4");
        assert_eq!(metadata.api_format, ApiFormat::OpenAIResponses);
        assert_eq!(metadata.context_window, 400_000);
        assert!(metadata.supports_thinking);
    }

    #[test]
    fn resolves_builtin_reasoning_metadata_before_fallback() {
        let metadata =
            resolve_model_metadata(ProviderId::MiniMax, "MiniMax-M2.5", ApiFormat::Anthropic);

        assert_eq!(metadata.reasoning_format, Some(ReasoningFormat::Anthropic));
        assert!(metadata.supports_thinking);
        assert_eq!(metadata.context_window, 204_800);
    }
}
