//! Provider-specific transformations and parameters
//!
//! Handles model-specific and provider-specific API parameters, message
//! transformations, and compatibility layers based on OpenCode's logic.

use crate::ai::glm;
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::streaming::StreamPart;
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

    // MiniMax ignores top_k according to their API docs
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
    // GLM models: use glm module for thinking-specific handling
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
        ProviderId::ZAi | ProviderId::MiniMax | ProviderId::OpenAI => {
            // For OpenAI-compatible providers (GLM, MiniMax, OpenAI)
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
    _provider_id: ProviderId,
) -> serde_json::Value {
    let id = model_id.to_lowercase();

    // Mistral requires tool call ID sanitization
    if id.contains("mistral") {
        return transform_mistral_message(message);
    }

    // GLM, MiniMax, DeepSeek (OpenAI-compatible)
    if id.contains("deepseek") || id.contains("glm") || id.contains("minimax") {
        return transform_glm_message(message);
    }

    message.clone()
}

/// Apply a final request-body transform immediately before dispatch.
///
/// This keeps provider quirks out of the transport call-sites and gives us one
/// seam to patch request bodies as model families drift.
pub fn apply_request_body_transform(
    mut body: Value,
    provider_id: ProviderId,
    api_format: ApiFormat,
    model_id: &str,
) -> Value {
    if let Some(messages) = body
        .get_mut("messages")
        .and_then(|value| value.as_array_mut())
    {
        for message in messages.iter_mut() {
            *message = transform_message_for_provider(message, model_id, provider_id);
            if requires_tool_call_id_sanitization(model_id) {
                sanitize_tool_call_ids_in_value(message);
            }
        }
    }

    if matches!(api_format, ApiFormat::OpenAIResponses)
        && body
            .get("tools")
            .and_then(|value| value.as_array())
            .is_some()
        && body.get("parallel_tool_calls").is_none()
    {
        body["parallel_tool_calls"] = Value::Bool(true);
    }

    body
}

/// Apply a final stream-part transform after parser mapping.
///
/// This is currently used for provider families that need sanitized tool IDs so
/// tool call start/delta/complete events stay aligned.
pub fn apply_stream_part_transform(
    part: StreamPart,
    _provider_id: ProviderId,
    _api_format: ApiFormat,
    model_id: &str,
) -> StreamPart {
    if !requires_tool_call_id_sanitization(model_id) {
        return part;
    }

    match part {
        StreamPart::ToolCallStart { id, name } => StreamPart::ToolCallStart {
            id: sanitize_tool_call_id(&id),
            name,
        },
        StreamPart::ToolCallDelta { id, delta } => StreamPart::ToolCallDelta {
            id: sanitize_tool_call_id(&id),
            delta,
        },
        StreamPart::ToolCallComplete { mut tool_call } => {
            tool_call.id = sanitize_tool_call_id(&tool_call.id);
            StreamPart::ToolCallComplete { tool_call }
        }
        other => other,
    }
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
                            part_obj.insert(
                                "toolCallId".to_string(),
                                Value::String(sanitize_tool_call_id(id_str)),
                            );
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
                            if let Some(compat) = opts
                                .entry("openaiCompatible")
                                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                                .as_object_mut()
                            {
                                compat.insert(
                                    "reasoning_content".to_string(),
                                    Value::String(reasoning_text),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    msg
}

fn requires_tool_call_id_sanitization(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    id.contains("mistral")
        || id.contains("deepseek")
        || id.contains("glm")
        || id.contains("minimax")
}

fn sanitize_tool_call_id(id: &str) -> String {
    let normalized: String = id
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .chars()
        .take(9)
        .collect();

    let padding_len = 9_usize.saturating_sub(normalized.chars().count());
    let padding = std::iter::repeat_n('0', padding_len);
    normalized.chars().chain(padding).collect()
}

fn sanitize_tool_call_ids_in_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                sanitize_tool_call_ids_in_value(item);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if matches!(
                    key.as_str(),
                    "toolCallId" | "tool_call_id" | "tool_use_id" | "call_id"
                ) {
                    if let Some(id) = child.as_str() {
                        *child = Value::String(sanitize_tool_call_id(id));
                        continue;
                    }
                }

                sanitize_tool_call_ids_in_value(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::streaming::StreamPart;
    use crate::ai::types::AiToolCall;
    use serde_json::json;

    #[test]
    fn test_temperature_for_model() {
        assert_eq!(temperature_for_model("qwen-coder"), Some(0.55));
        assert_eq!(temperature_for_model("claude-sonnet-4"), None);
        assert_eq!(temperature_for_model("gemini-3-pro"), Some(1.0));
        assert_eq!(temperature_for_model("GLM-5"), Some(1.0));
        assert_eq!(temperature_for_model("minimax-m2.5"), Some(1.0));
    }

    #[test]
    fn test_top_p_for_model() {
        assert_eq!(top_p_for_model("qwen-coder"), Some(1.0));
        assert_eq!(top_p_for_model("minimax-m2.5"), Some(0.95));
        assert_eq!(top_p_for_model("gemini-3-pro"), Some(0.95));
        assert_eq!(top_p_for_model("claude-sonnet-4"), None);
    }

    #[test]
    fn test_top_k_for_model() {
        // MiniMax ignores top_k per their API docs
        assert_eq!(top_k_for_model("minimax-m2.5"), None);
        assert_eq!(top_k_for_model("minimax-m2"), None);
        assert_eq!(top_k_for_model("gemini-3-pro"), Some(64));
        assert_eq!(top_k_for_model("claude-sonnet-4"), None);
    }

    #[test]
    fn test_supports_reasoning_effort() {
        // OpenAI-compatible models (GLM, MiniMax, DeepSeek) don't support effort levels
        assert!(!supports_reasoning_effort("deepseek-r1"));
        assert!(!supports_reasoning_effort("GLM-5"));
        assert!(!supports_reasoning_effort("minimax-m2.5"));
        assert!(!supports_reasoning_effort("mistral-large"));
        assert!(supports_reasoning_effort("gpt-5"));
        assert!(supports_reasoning_effort("claude-sonnet-4"));
    }

    #[test]
    fn test_chat_template_args_for_model() {
        // GLM-5 with thinking enabled: returns chat_template_args
        let args = chat_template_args_for_model("GLM-5", true);
        assert!(args.is_some());
        let binding = args.unwrap();
        let obj = binding.as_object().unwrap();
        assert_eq!(obj.get("enableThinking").unwrap().as_bool(), Some(true));

        // GLM-5 with thinking disabled: returns None
        let args = chat_template_args_for_model("GLM-5", false);
        assert!(args.is_none());

        // MiniMax M2.5: doesn't use chat_template_args (even with thinking enabled)
        let args = chat_template_args_for_model("minimax-m2.5", true);
        assert!(args.is_none());

        // Non-OpenAI-compatible model
        let args = chat_template_args_for_model("claude-sonnet-4", true);
        assert!(args.is_none());
    }

    #[test]
    fn request_transform_sets_parallel_tool_calls_for_responses() {
        let body = json!({
            "model": "gpt-5.3-codex",
            "tools": [{"name": "read"}]
        });

        let transformed = apply_request_body_transform(
            body,
            ProviderId::OpenAI,
            ApiFormat::OpenAIResponses,
            "gpt-5.3-codex",
        );

        assert_eq!(transformed["parallel_tool_calls"], Value::Bool(true));
    }

    #[test]
    fn request_transform_sanitizes_message_tool_ids() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "toolCallId": "call_1:weird"
                }]
            }]
        });

        let transformed = apply_request_body_transform(
            body,
            ProviderId::MiniMax,
            ApiFormat::Anthropic,
            "minimax-m2.5",
        );

        assert_eq!(
            transformed["messages"][0]["content"][0]["toolCallId"],
            Value::String("call1weir".to_string())
        );
    }

    #[test]
    fn stream_part_transform_sanitizes_tool_ids() {
        let part = StreamPart::ToolCallComplete {
            tool_call: AiToolCall {
                id: "call_1:weird".to_string(),
                name: "read".to_string(),
                arguments: json!({}),
            },
        };

        let transformed = apply_stream_part_transform(
            part,
            ProviderId::MiniMax,
            ApiFormat::Anthropic,
            "minimax-m2.5",
        );

        match transformed {
            StreamPart::ToolCallComplete { tool_call } => {
                assert_eq!(tool_call.id, "call1weir");
            }
            other => panic!("unexpected part: {other:?}"),
        }
    }
}
