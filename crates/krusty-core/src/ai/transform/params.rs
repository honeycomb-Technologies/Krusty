use crate::ai::glm;
use crate::ai::providers::ProviderId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Provider-specific options that get wrapped in provider-specific objects
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOptions {
    /// Anthropic-specific options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<Value>,
    /// OpenAI/OpenAI-compatible options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai: Option<Value>,
    /// OpenAI-compatible options (used by GLM, DeepSeek, MiniMax, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai_compatible: Option<Value>,
    /// Google/Gemini options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google: Option<Value>,
    /// Bedrock options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bedrock: Option<Value>,
    /// OpenRouter options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter: Option<Value>,
    /// Generic provider-specific options (provider ID as key)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub custom: Option<Value>,
}

/// Provider-specific request parameters
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpecificParams {
    /// Temperature (model-specific defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Top P sampling (model-specific defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Top K sampling (model-specific defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    /// Chat template args for GLM thinking models
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_args: Option<Value>,
}

/// Get temperature for a model (based on OpenCode's logic)
///
/// For OpenAI-compatible models (GLM, MiniMax), delegates to glm module
pub fn temperature_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();

    if glm::is_openai_compatible_model(model_id) {
        return glm::get_default_temperature(model_id);
    }

    if id.contains("qwen") {
        return Some(0.55);
    }
    if id.contains("claude") {
        return None;
    }
    if id.contains("gemini") {
        return Some(1.0);
    }

    None
}

/// Get top P for a model (based on OpenCode's logic)
pub fn top_p_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();

    if id.contains("qwen") {
        return Some(1.0);
    }
    if id.contains("minimax-m2") {
        return Some(0.95);
    }
    if id.contains("gemini") {
        return Some(0.95);
    }

    None
}

/// Get top K for a model (based on OpenCode's logic)
pub fn top_k_for_model(model_id: &str) -> Option<i32> {
    let id = model_id.to_lowercase();

    if id.contains("minimax") {
        return None;
    }

    if id.contains("gemini") {
        return Some(64);
    }

    None
}

/// Check if a model supports reasoning effort control
///
/// OpenAI-compatible models (GLM, MiniMax, DeepSeek) don't support effort levels
pub fn supports_reasoning_effort(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    !["deepseek", "minimax", "glm", "mistral"]
        .iter()
        .any(|pat| id.contains(pat))
}

/// Get chat template args for thinking models
///
/// For GLM models, delegates to glm module with the user's reasoning preference
pub fn chat_template_args_for_model(model_id: &str, thinking_enabled: bool) -> Option<Value> {
    if glm::is_openai_compatible_model(model_id) {
        return glm::get_chat_template_args(model_id, glm::ReasoningMode::from(thinking_enabled));
    }

    None
}

/// Build provider-specific parameters for a model
pub fn build_provider_params(
    model_id: &str,
    _provider_id: ProviderId,
    thinking_enabled: bool,
) -> ProviderSpecificParams {
    ProviderSpecificParams {
        temperature: temperature_for_model(model_id),
        top_p: top_p_for_model(model_id),
        top_k: top_k_for_model(model_id),
        chat_template_args: chat_template_args_for_model(model_id, thinking_enabled),
    }
}

/// Wrap options in provider-specific structure
pub fn wrap_provider_options(options: Value, provider_id: ProviderId) -> ProviderOptions {
    match provider_id {
        ProviderId::OpenRouter => ProviderOptions {
            openrouter: Some(options),
            ..Default::default()
        },
        ProviderId::Anthropic => ProviderOptions {
            anthropic: Some(options),
            ..Default::default()
        },
        ProviderId::ZAi | ProviderId::MiniMax | ProviderId::OpenAI | ProviderId::Grok => {
            if options
                .as_object()
                .and_then(|o| o.get("reasoning_content"))
                .is_some()
            {
                ProviderOptions {
                    openai_compatible: Some(options),
                    ..Default::default()
                }
            } else {
                ProviderOptions {
                    anthropic: Some(options),
                    ..Default::default()
                }
            }
        }
    }
}
