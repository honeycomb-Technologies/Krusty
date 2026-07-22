use krusty_core::ai::models::{
    ApiFormat, ModelAuthScope, ModelCatalogSource, ModelKey, ModelMetadata,
};
use krusty_core::ai::providers::{FastMode, ProviderId, ReasoningControl, ReasoningEffort};
use serde::Serialize;

// ============================================================================
// Model Types
// ============================================================================

#[derive(Serialize)]
pub struct ModelResponse {
    /// Provider-aware executable identity. Older clients may continue using `id`
    /// only while that slug is unambiguous.
    pub key: ModelKey,
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub provider_id: ProviderId,
    pub auth_scope: Option<ModelAuthScope>,
    pub api_format: ApiFormat,
    pub catalog_source: ModelCatalogSource,
    pub catalog_revision: Option<String>,
    pub context_window: usize,
    pub max_output: usize,
    /// Legacy capability flag retained for older clients.
    pub supports_thinking: bool,
    /// How (or whether) clients may explicitly control reasoning for this model.
    pub reasoning_control: Option<ReasoningControl>,
    pub supported_reasoning_levels: Vec<ReasoningEffort>,
    pub default_reasoning_level: Option<ReasoningEffort>,
    pub reasoning_is_mandatory: bool,
    pub supports_fast_mode: bool,
    pub fast_mode: Option<FastMode>,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

impl ModelResponse {
    pub fn from_metadata(model: &ModelMetadata, provider: String) -> Self {
        // Ultra is an orchestration/delegation mode in the ChatGPT catalog, not
        // a provider reasoning-effort wire value. Keep accepting it on legacy
        // requests, but do not advertise it as a selectable thought level until
        // Krusty implements its delegation semantics.
        let supported_reasoning_levels = model
            .supported_reasoning_levels
            .iter()
            .copied()
            .filter(|level| *level != ReasoningEffort::Ultra)
            .collect::<Vec<_>>();
        let default_reasoning_level = model
            .default_reasoning_level
            .filter(|level| supported_reasoning_levels.contains(level));

        Self {
            key: model.key(),
            id: model.id.clone(),
            display_name: model.display_name.clone(),
            provider,
            provider_id: model.provider,
            auth_scope: model.auth_scope,
            api_format: model.api_format,
            catalog_source: model.catalog_source,
            catalog_revision: model.catalog_revision.clone(),
            context_window: model.context_window,
            max_output: model.max_output,
            supports_thinking: model.supports_thinking,
            reasoning_control: model.reasoning_control,
            supported_reasoning_levels,
            default_reasoning_level,
            reasoning_is_mandatory: model.reasoning_is_mandatory,
            supports_fast_mode: model.fast_mode.is_some(),
            fast_mode: model.fast_mode,
            supports_tools: model.supports_tools,
            supports_vision: model.supports_vision,
        }
    }
}

#[derive(Serialize)]
pub struct ModelsListResponse {
    pub models: Vec<ModelResponse>,
    pub default_model: Option<String>,
    pub default_model_key: Option<ModelKey>,
}

#[derive(Serialize)]
pub struct SimpleOkResponse {
    pub ok: bool,
}
