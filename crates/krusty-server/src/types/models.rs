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
    pub supports_thinking: bool,
    pub supports_tools: bool,
    pub supports_vision: bool,
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
