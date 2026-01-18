//! Provider-specific transformations and parameters
//!
//! Handles model-specific and provider-specific API parameters, message
//! transformations, and compatibility layers based on OpenCode's logic.

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
    /// Chat template args for GLM/Kimi thinking models
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_template_args: Option<Value>,
}

/// Get temperature for a model (based on OpenCode's logic)
///
/// For OpenAI-compatible models (GLM, MiniMax, Kimi), delegates to glm module
pub fn temperature_for_model(model_id: &str) -> Option<f32> {
    let id = model_id.to_lowercase();

    // OpenAI-compatible models: use glm module for specific defaults
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

    if id.contains("minimax-m2") {
        if id.contains("m2.1") {
            return Some(40);
        }
        return Some(20);
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
/// For GLM/Kimi models, delegates to glm module with the user's reasoning preference
pub fn chat_template_args_for_model(model_id: &str, thinking_enabled: bool) -> Option<Value> {
    // GLM/Kimi models: use glm module for thinking-specific handling
    if glm::is_openai_compatible_model(model_id) {
        return glm::get_chat_template_args(model_id, glm::ReasoningMode::from(thinking_enabled));
    }

    None
}

/// Build provider-specific parameters for a model
///
/// Note: For OpenCode Zen, we skip model-specific params since Zen handles
/// translation internally. Adding params like chat_template_args breaks requests.
pub fn build_provider_params(
    model_id: &str,
    provider_id: ProviderId,
    thinking_enabled: bool,
) -> ProviderSpecificParams {
    // OpenCode Zen handles model-specific params internally - don't add them
    // Adding chat_template_args etc. breaks requests to Zen's routing layer
    if provider_id == ProviderId::OpenCodeZen {
        return ProviderSpecificParams::default();
    }

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
        ProviderId::Anthropic => ProviderOptions {
            anthropic: Some(options),
            ..Default::default()
        },
        ProviderId::OpenRouter => ProviderOptions {
            openrouter: Some(options),
            ..Default::default()
        },
        ProviderId::OpenCodeZen => {
            if options
                .as_object()
                .and_then(|o| o.get("chat_template_args"))
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
        ProviderId::ZAi | ProviderId::MiniMax | ProviderId::Kimi => {
            // For OpenAI-compatible providers (GLM, MiniMax, Kimi)
            // Check if options contain reasoning_content (DeepSeek/MiniMax style)
            if options
                .as_object()
                .and_then(|o| o.get("reasoning_content"))
                .is_some()
            {
                // Wrap in openai_compatible with reasoning_content
                ProviderOptions {
                    openai_compatible: Some(options),
                    ..Default::default()
                }
            } else {
                // Standard Anthropic-compatible format
                ProviderOptions {
                    anthropic: Some(options),
                    ..Default::default()
                }
            }
        }
    }
}

/// Transform message for provider-specific requirements
pub fn transform_message_for_provider(
    message: &serde_json::Value,
    model_id: &str,
    provider_id: ProviderId,
) -> serde_json::Value {
    let id = model_id.to_lowercase();

    if id.contains("mistral") || provider_id == ProviderId::ZAi {
        return transform_mistral_message(message);
    }

    // GLM, MiniMax, DeepSeek (OpenAI-compatible)
    if id.contains("deepseek") || id.contains("glm") || id.contains("minimax") {
        return transform_glm_message(message);
    }

    message.clone()
}

/// Transform message for Mistral/GLM/MiniMax (tool call ID sanitization)
fn transform_mistral_message(message: &serde_json::Value) -> serde_json::Value {
    let mut msg = message.clone();

    if let Some(obj) = msg.as_object_mut() {
        if let Some(content) = obj.get_mut("content").and_then(|c| c.as_array_mut()) {
            for part in content.iter_mut() {
                if let Some(part_obj) = part.as_object_mut() {
                    if let Some(tool_call_id) = part_obj.get("toolCallId") {
                        if let Some(id_str) = tool_call_id.as_str() {
                            let normalized: String = id_str
                                .chars()
                                .filter(|c| c.is_alphanumeric())
                                .collect::<String>()
                                .chars()
                                .take(9)
                                .collect::<String>();

                            let padding_len = 9_usize.saturating_sub(normalized.chars().count());
                            let padding = std::iter::repeat_n('0', padding_len);

                            let final_id: String = normalized.chars().chain(padding).collect();

                            part_obj.insert("toolCallId".to_string(), Value::String(final_id));
                        }
                    }
                }
            }
        }
    }

    msg
}

/// Transform message for GLM/MiniMax/DeepSeek (move reasoning content to provider options)
fn transform_glm_message(message: &serde_json::Value) -> serde_json::Value {
    let mut msg = message.clone();

    if let Some(obj) = msg.as_object_mut() {
        if let Some(role) = obj.get("role").and_then(|r| r.as_str()) {
            if role == "assistant" {
                if let Some(content) = obj.get_mut("content").and_then(|c| c.as_array_mut()) {
                    let reasoning_text: String = content
                        .iter()
                        .filter_map(|part| {
                            part.as_object()
                                .and_then(|o| o.get("type").and_then(|t| t.as_str()))
                                .filter(|t| *t == "reasoning")
                                .and_then(|_| {
                                    part.as_object().and_then(|o| {
                                        o.get("text")
                                            .and_then(|t| t.as_str())
                                            .map(|s| s.to_string())
                                    })
                                })
                        })
                        .collect();

                    if !reasoning_text.is_empty() {
                        let filtered_content: Vec<Value> = content
                            .iter()
                            .filter(|part| {
                                part.as_object()
                                    .and_then(|o| o.get("type").and_then(|t| t.as_str()))
                                    != Some("reasoning")
                            })
                            .cloned()
                            .collect();

                        obj.insert("content".to_string(), Value::Array(filtered_content));

                        let provider_options = obj
                            .entry("providerOptions")
                            .or_insert_with(|| Value::Object(serde_json::Map::new()));

                        if let Some(opts) = provider_options.as_object_mut() {
                            opts.entry("openaiCompatible")
                                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                                .as_object_mut()
                                .unwrap()
                                .insert(
                                    "reasoning_content".to_string(),
                                    Value::String(reasoning_text),
                                );
                        }
                    }
                }
            }
        }
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temperature_for_model() {
        assert_eq!(temperature_for_model("qwen-coder"), Some(0.55));
        assert_eq!(temperature_for_model("claude-sonnet-4"), None);
        assert_eq!(temperature_for_model("gemini-3-pro"), Some(1.0));
        assert_eq!(temperature_for_model("glm-4.7"), Some(1.0));
        assert_eq!(temperature_for_model("minimax-m2.1"), Some(1.0));
        assert_eq!(temperature_for_model("kimi-k2"), Some(0.6));
        assert_eq!(temperature_for_model("kimi-k2-thinking"), Some(1.0));
    }

    #[test]
    fn test_top_p_for_model() {
        assert_eq!(top_p_for_model("qwen-coder"), Some(1.0));
        assert_eq!(top_p_for_model("minimax-m2.1"), Some(0.95));
        assert_eq!(top_p_for_model("gemini-3-pro"), Some(0.95));
        assert_eq!(top_p_for_model("claude-sonnet-4"), None);
    }

    #[test]
    fn test_top_k_for_model() {
        assert_eq!(top_k_for_model("minimax-m2.1"), Some(40));
        assert_eq!(top_k_for_model("minimax-m2"), Some(20));
        assert_eq!(top_k_for_model("gemini-3-pro"), Some(64));
        assert_eq!(top_k_for_model("claude-sonnet-4"), None);
    }

    #[test]
    fn test_supports_reasoning_effort() {
        // OpenAI-compatible models (GLM, MiniMax, DeepSeek) don't support effort levels
        assert!(!supports_reasoning_effort("deepseek-r1"));
        assert!(!supports_reasoning_effort("glm-4.7"));
        assert!(!supports_reasoning_effort("minimax-m2.1"));
        assert!(!supports_reasoning_effort("mistral-large"));
        assert!(supports_reasoning_effort("gpt-5"));
        assert!(supports_reasoning_effort("claude-sonnet-4"));
    }

    #[test]
    fn test_chat_template_args_for_model() {
        // GLM-4.6 with thinking enabled: returns chat_template_args
        let args = chat_template_args_for_model("glm-4.6", true);
        assert!(args.is_some());
        let binding = args.unwrap();
        let obj = binding.as_object().unwrap();
        assert_eq!(obj.get("enableThinking").unwrap().as_bool(), Some(true));

        // GLM-4.6 with thinking disabled: returns None
        let args = chat_template_args_for_model("glm-4.6", false);
        assert!(args.is_none());

        // Kimi K2 thinking with thinking enabled: returns chat_template_args
        let args = chat_template_args_for_model("kimi-k2-thinking", true);
        assert!(args.is_some());

        // Kimi K2 with thinking disabled: returns None
        let args = chat_template_args_for_model("kimi-k2-thinking", false);
        assert!(args.is_none());

        // MiniMax M2: doesn't use chat_template_args (even with thinking enabled)
        let args = chat_template_args_for_model("minimax-m2.1", true);
        assert!(args.is_none());

        // Non-OpenAI-compatible model
        let args = chat_template_args_for_model("claude-sonnet-4", true);
        assert!(args.is_none());
    }
}
