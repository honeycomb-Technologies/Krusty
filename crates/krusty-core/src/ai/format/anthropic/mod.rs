//! Anthropic API format handler
//!
//! Handles message alternation, thinking block preservation, and tool conversion
//! for the Anthropic Messages API.

mod messages;
mod request;

use serde_json::Value;

use super::{FormatHandler, RequestOptions};
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, ModelMessage};

/// Anthropic format handler
pub struct AnthropicFormat {
    endpoint: String,
}

impl AnthropicFormat {
    pub fn new() -> Self {
        Self {
            endpoint: "/v1/messages".to_string(),
        }
    }
}

impl Default for AnthropicFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl FormatHandler for AnthropicFormat {
    fn convert_messages(
        &self,
        messages: &[ModelMessage],
        provider_id: Option<ProviderId>,
    ) -> Vec<Value> {
        self.convert_messages_impl(messages, provider_id)
    }

    fn convert_tools(&self, tools: &[AiTool]) -> Vec<Value> {
        self.convert_tools_impl(tools)
    }

    fn build_request_body(
        &self,
        model: &str,
        messages: Vec<Value>,
        options: &RequestOptions,
    ) -> Value {
        self.build_request_body_impl(model, messages, options)
    }

    fn endpoint_path(&self, _model: &str) -> &str {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::AnthropicFormat;

    #[test]
    fn sanitize_removes_orphans_and_injects_missing_results() {
        let mut messages = vec![
            json!({
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "tool-a", "name": "read", "input": {}},
                    {"type": "tool_use", "id": "tool-b", "name": "grep", "input": {}}
                ]
            }),
            json!({
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "tool-a", "content": "ok", "is_error": false},
                    {"type": "tool_result", "tool_use_id": "orphan", "content": "bad", "is_error": false}
                ]
            }),
        ];

        AnthropicFormat::sanitize_tool_results(&mut messages);

        let content = messages[1]["content"]
            .as_array()
            .expect("expected user content array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["tool_use_id"].as_str(), Some("tool-a"));
        assert_eq!(content[1]["tool_use_id"].as_str(), Some("tool-b"));
        assert_eq!(content[1]["is_error"].as_bool(), Some(true));
    }

    #[test]
    fn sanitize_inserts_user_message_when_missing_after_tool_use() {
        let mut messages = vec![json!({
            "role": "assistant",
            "content": [
                {"type": "tool_use", "id": "tool-x", "name": "bash", "input": {}}
            ]
        })];

        AnthropicFormat::sanitize_tool_results(&mut messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"].as_str(), Some("user"));
        let content = messages[1]["content"]
            .as_array()
            .expect("expected inserted user content array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["tool_use_id"].as_str(), Some("tool-x"));
        assert_eq!(content[0]["is_error"].as_bool(), Some(true));
    }
}
