//! OpenAI API format handler
//!
//! Handles conversion to OpenAI chat/completions and responses API formats.

use serde_json::Value;

use super::{FormatHandler, RequestOptions};
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, Content, ModelMessage, Role};

/// OpenAI format handler
pub struct OpenAIFormat {
    api_format: ApiFormat,
    endpoint: String,
}

impl OpenAIFormat {
    pub fn new(format: ApiFormat) -> Self {
        let endpoint = match format {
            ApiFormat::OpenAIResponses => "/v1/responses".to_string(),
            _ => "/v1/chat/completions".to_string(),
        };
        Self {
            api_format: format,
            endpoint,
        }
    }

    fn is_responses_format(&self) -> bool {
        matches!(self.api_format, ApiFormat::OpenAIResponses)
    }
}

impl FormatHandler for OpenAIFormat {
    /// Convert domain messages to OpenAI chat/completions format
    ///
    /// OpenAI format is simpler: role + content (string or array of content parts)
    /// Note: provider_id is unused for OpenAI format (no thinking block handling needed)
    fn convert_messages(
        &self,
        messages: &[ModelMessage],
        _provider_id: Option<ProviderId>,
    ) -> Vec<Value> {
        let mut result: Vec<Value> = Vec::new();

        for msg in messages.iter().filter(|m| m.role != Role::System) {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => continue,
            };

            // For tool results, use special format
            if msg.role == Role::Tool {
                for content in &msg.content {
                    if let Content::ToolResult {
                        tool_use_id,
                        output,
                        ..
                    } = content
                    {
                        result.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": output
                        }));
                    }
                }
                continue;
            }

            // For assistant messages with tool calls
            let has_tool_use = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolUse { .. }));

            if has_tool_use && role == "assistant" {
                let mut tool_calls = Vec::new();
                let mut text_content = String::new();

                for content in &msg.content {
                    match content {
                        Content::Text { text } => text_content.push_str(text),
                        Content::ToolUse { id, name, input } => {
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": input.to_string()
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut msg_obj = serde_json::json!({
                    "role": "assistant",
                    "tool_calls": tool_calls
                });
                if !text_content.is_empty() {
                    msg_obj["content"] = serde_json::json!(text_content);
                }
                result.push(msg_obj);
                continue;
            }

            // Regular messages - extract text content
            let text: String = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            if !text.is_empty() {
                result.push(serde_json::json!({
                    "role": role,
                    "content": text
                }));
            }
        }

        result
    }

    fn convert_tools(&self, tools: &[AiTool]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                if self.is_responses_format() {
                    // Responses API: flat structure with name at top level
                    serde_json::json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema
                    })
                } else {
                    // Chat Completions: nested under "function"
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema
                        }
                    })
                }
            })
            .collect()
    }

    fn build_request_body(
        &self,
        model: &str,
        messages: Vec<Value>,
        options: &RequestOptions,
    ) -> Value {
        // Responses API uses "input", Chat Completions uses "messages"
        let (messages_key, max_tokens_key) = if self.is_responses_format() {
            ("input", "max_output_tokens")
        } else {
            ("messages", "max_tokens")
        };

        let mut body = serde_json::json!({
            "model": model,
        });

        body[messages_key] = serde_json::json!(messages);
        body[max_tokens_key] = serde_json::json!(options.max_tokens);

        if options.streaming {
            body["stream"] = serde_json::json!(true);
        }

        // Add system message at the start if present
        if let Some(system) = options.system_prompt {
            if let Some(msgs) = body.get_mut(messages_key).and_then(|m| m.as_array_mut()) {
                msgs.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": system
                    }),
                );
            }
        }

        if let Some(temp) = options.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if let Some(tools) = options.tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(self.convert_tools(tools));
            }
        }

        body
    }

    fn endpoint_path(&self, _model: &str) -> &str {
        &self.endpoint
    }
}
