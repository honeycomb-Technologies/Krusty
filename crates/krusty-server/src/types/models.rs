use krusty_core::ai::models::ModelMetadata;
use krusty_core::ai::providers::{FastMode, ReasoningControl, ReasoningEffort};
use serde::Serialize;

// ============================================================================
// Model Types
// ============================================================================

#[derive(Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub display_name: String,
    pub provider: String,
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
            id: model.id.clone(),
            display_name: model.display_name.clone(),
            provider,
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
}

#[derive(Serialize)]
pub struct SimpleOkResponse {
    pub ok: bool,
}
