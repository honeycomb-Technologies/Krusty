use serde::Deserialize;

/// Response from OpenRouter models endpoint
#[derive(Debug, Deserialize)]
pub(super) struct ModelsResponse {
    pub(super) data: Vec<OpenRouterModel>,
}

/// Single model from OpenRouter API
#[derive(Debug, Deserialize)]
pub(super) struct OpenRouterModel {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) context_length: Option<usize>,
    #[serde(default)]
    pub(super) top_provider: Option<TopProvider>,
    #[serde(default)]
    pub(super) pricing: Option<Pricing>,
    #[serde(default)]
    pub(super) supported_parameters: Vec<String>,
    #[serde(default)]
    pub(super) architecture: Option<Architecture>,
    #[serde(default)]
    pub(super) reasoning: Option<ReasoningCapabilities>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReasoningCapabilities {
    #[serde(default)]
    pub(super) supported_efforts: Vec<String>,
    #[serde(default)]
    pub(super) default_effort: Option<String>,
    #[serde(default)]
    pub(super) default_enabled: Option<bool>,
    #[serde(default)]
    pub(super) mandatory: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct TopProvider {
    pub(super) max_completion_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Pricing {
    pub(super) prompt: Option<String>,
    pub(super) completion: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Architecture {
    #[serde(default)]
    pub(super) input_modalities: Vec<String>,
}
