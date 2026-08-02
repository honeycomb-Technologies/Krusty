use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct ModelsResponse {
    pub(super) data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiModel {
    pub(super) id: String,
}

/// Rich model list returned by the ChatGPT Codex backend for the authenticated
/// account. This is intentionally separate from OpenAI's public `/v1/models`
/// response: the Codex endpoint carries entitlement-specific capabilities.
#[derive(Debug, Deserialize)]
pub(super) struct ChatGptModelsResponse {
    pub(super) models: Vec<ChatGptModel>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatGptModel {
    pub(super) slug: String,
    pub(super) display_name: String,
    pub(super) visibility: String,
    #[serde(default)]
    pub(super) context_window: Option<usize>,
    #[serde(default)]
    pub(super) max_context_window: Option<usize>,
    #[serde(default, alias = "max_output", alias = "output_token_limit")]
    pub(super) max_output_tokens: Option<usize>,
    #[serde(default)]
    pub(super) supported_reasoning_levels: Vec<ChatGptReasoningPreset>,
    #[serde(default)]
    pub(super) default_reasoning_level: Option<String>,
    #[serde(default, alias = "reasoning_mandatory", alias = "mandatory_reasoning")]
    pub(super) reasoning_is_mandatory: Option<bool>,
    #[serde(default)]
    pub(super) service_tiers: Vec<ChatGptServiceTier>,
    #[serde(default)]
    pub(super) input_modalities: Vec<String>,
    #[serde(default)]
    pub(super) supports_tools: Option<bool>,
    #[serde(default)]
    pub(super) supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub(super) shell_type: Option<String>,
    #[serde(default)]
    pub(super) experimental_supported_tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatGptReasoningPreset {
    pub(super) effort: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatGptServiceTier {
    pub(super) id: String,
}
