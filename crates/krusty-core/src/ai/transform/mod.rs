//! Provider-specific transformations and parameters
//!
//! Handles model-specific and provider-specific API parameters, message
//! transformations, and compatibility layers based on OpenCode's logic.

mod messages;
mod params;
mod sanitize;

pub use messages::{apply_request_body_transform, transform_message_for_provider};
pub use params::{
    build_provider_params, chat_template_args_for_model, supports_reasoning_effort,
    temperature_for_model, top_k_for_model, top_p_for_model, wrap_provider_options,
    ProviderOptions, ProviderSpecificParams,
};
pub use sanitize::apply_stream_part_transform;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::ProviderId;
    use crate::ai::streaming::StreamPart;
    use crate::ai::types::AiToolCall;
    use serde_json::{json, Value};

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
        assert_eq!(top_k_for_model("minimax-m2.5"), None);
        assert_eq!(top_k_for_model("minimax-m2"), None);
        assert_eq!(top_k_for_model("gemini-3-pro"), Some(64));
        assert_eq!(top_k_for_model("claude-sonnet-4"), None);
    }

    #[test]
    fn test_supports_reasoning_effort() {
        assert!(!supports_reasoning_effort("deepseek-r1"));
        assert!(!supports_reasoning_effort("GLM-5"));
        assert!(!supports_reasoning_effort("minimax-m2.5"));
        assert!(!supports_reasoning_effort("mistral-large"));
        assert!(supports_reasoning_effort("gpt-5"));
        assert!(supports_reasoning_effort("claude-sonnet-4"));
    }

    #[test]
    fn test_chat_template_args_for_model() {
        let args = chat_template_args_for_model("GLM-5", true);
        assert!(args.is_some());
        let binding = args.unwrap();
        let obj = binding.as_object().unwrap();
        assert_eq!(obj.get("enableThinking").unwrap().as_bool(), Some(true));

        let args = chat_template_args_for_model("GLM-5", false);
        assert!(args.is_none());

        let args = chat_template_args_for_model("minimax-m2.5", true);
        assert!(args.is_none());

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
