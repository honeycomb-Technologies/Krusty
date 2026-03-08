//! OpenAI API format handler
//!
//! Handles conversion to OpenAI chat/completions and responses API formats.
//! Includes message alternation validation and thinking block preservation
//! for parity with Anthropic format handling.

use serde_json::Value;
use tracing::debug;

use super::{needs_role_alternation_filler, FormatHandler, RequestOptions};
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, Content, ImageContent, ModelMessage, Role};

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

    /// Convert image content to a URL accepted by OpenAI:
    /// - pass-through remote URL
    /// - data URL for base64 payloads
    fn image_to_url(image: &ImageContent) -> Option<String> {
        if let Some(url) = &image.url {
            return Some(url.clone());
        }

        image.base64.as_ref().map(|base64| {
            let media_type = image.media_type.as_deref().unwrap_or("image/png");
            format!("data:{};base64,{}", media_type, base64)
        })
    }

    /// Build a user text content part for the current OpenAI API flavor.
    fn user_text_part(&self, text: &str) -> Value {
        if self.is_responses_format() {
            serde_json::json!({
                "type": "input_text",
                "text": text
            })
        } else {
            serde_json::json!({
                "type": "text",
                "text": text
            })
        }
    }

    /// Build a user image content part for the current OpenAI API flavor.
    fn user_image_part(&self, image: &ImageContent, detail: Option<&str>) -> Option<Value> {
        let image_url = Self::image_to_url(image)?;
        let detail = detail.filter(|d| !d.is_empty());

        if self.is_responses_format() {
            let mut part = serde_json::json!({
                "type": "input_image",
                "image_url": image_url
            });
            if let Some(detail) = detail {
                part["detail"] = serde_json::json!(detail);
            }
            Some(part)
        } else {
            let mut image_url_obj = serde_json::json!({
                "url": image_url
            });
            if let Some(detail) = detail {
                image_url_obj["detail"] = serde_json::json!(detail);
            }
            Some(serde_json::json!({
                "type": "image_url",
                "image_url": image_url_obj
            }))
        }
    }
}

impl FormatHandler for OpenAIFormat {
    /// Convert domain messages to OpenAI chat/completions format
    ///
    /// OpenAI format: role + content (string or array of content parts)
    ///
    /// CRITICAL: This function ensures proper message alternation required by many providers.
    /// Some providers (like Kimi) require user/assistant messages to strictly alternate.
    /// If there are consecutive same-role messages, we insert filler messages.
    ///
    /// ORPHANED TOOL CALLS: If a session was interrupted mid-tool-execution, there may be
    /// tool_calls without matching tool results. This function detects and handles them
    /// by adding placeholder results to prevent API errors.
    ///
    /// THINKING BLOCKS: Preserved as text content prefixed with "[Thinking]" for
    /// providers that support reasoning/thinking models via OpenAI format.
    fn convert_messages(
        &self,
        messages: &[ModelMessage],
        _provider_id: Option<ProviderId>,
    ) -> Vec<Value> {
        // First pass: collect all tool_use IDs and tool_result IDs
        let mut tool_use_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut tool_result_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for msg in messages {
            for content in &msg.content {
                match content {
                    Content::ToolUse { id, .. } => {
                        tool_use_ids.insert(id.clone());
                    }
                    Content::ToolResult { tool_use_id, .. } => {
                        tool_result_ids.insert(tool_use_id.clone());
                    }
                    _ => {}
                }
            }
        }

        // Find orphaned tool calls (tool_use without matching tool_result)
        let orphaned_ids: std::collections::HashSet<&String> =
            tool_use_ids.difference(&tool_result_ids).collect();

        if !orphaned_ids.is_empty() {
            debug!(
                "Found {} orphaned tool calls without results: {:?}",
                orphaned_ids.len(),
                orphaned_ids
            );
        }

        let mut result: Vec<Value> = Vec::new();
        let mut last_role: Option<&str> = None;

        for msg in messages.iter().filter(|m| m.role != Role::System) {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => continue,
            };

            // Check if this message contains tool results
            // Tool results can come in Role::Tool OR Role::User messages (Anthropic style)
            let has_tool_results = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolResult { .. }));

            // For messages with tool results, convert to OpenAI tool format
            // This handles both Role::Tool and Role::User with ToolResult content
            if has_tool_results {
                for content in &msg.content {
                    if let Content::ToolResult {
                        tool_use_id,
                        output,
                        ..
                    } = content
                    {
                        // Format output as string for OpenAI
                        let output_str = match output {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        result.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": output_str
                        }));
                    }
                }
                // Update last_role to track tool messages in the sequence
                // This prevents incorrect filler insertion after tool results
                last_role = Some("tool");
                continue;
            }

            // Check for consecutive same-role messages (excluding tool role)
            // Many OpenAI-compatible APIs require strict user/assistant alternation
            if let Some(filler_role) = needs_role_alternation_filler(last_role, role, &["tool"]) {
                debug!(
                    "Inserting filler {} message to maintain alternation",
                    filler_role
                );
                result.push(serde_json::json!({
                    "role": filler_role,
                    "content": "."
                }));
            }

            // For assistant messages with tool calls
            let has_tool_use = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolUse { .. }));

            if has_tool_use && role == "assistant" {
                let mut tool_calls = Vec::new();
                let mut text_content = String::new();
                let mut orphaned_tool_ids: Vec<String> = Vec::new();

                for content in &msg.content {
                    match content {
                        Content::Text { text } => text_content.push_str(text),
                        Content::ToolUse { id, name, input } => {
                            // Track if this tool call is orphaned (no result)
                            if orphaned_ids.contains(&id) {
                                orphaned_tool_ids.push(id.clone());
                            }
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": input.to_string()
                                }
                            }));
                        }
                        // Preserve thinking in tool call messages too
                        Content::Thinking { thinking, .. } => {
                            if !thinking.is_empty() {
                                if !text_content.is_empty() {
                                    text_content.push_str("\n\n");
                                }
                                text_content.push_str("[Thinking]\n");
                                text_content.push_str(thinking);
                                text_content.push_str("\n[/Thinking]\n\n");
                            }
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

                // Add placeholder results for orphaned tool calls
                // This prevents "No tool output found for function call" errors
                for orphan_id in orphaned_tool_ids {
                    debug!(
                        "Adding placeholder result for orphaned tool call: {}",
                        orphan_id
                    );
                    result.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": orphan_id,
                        "content": "[Tool execution was interrupted - session resumed]"
                    }));
                }

                last_role = Some(role);
                continue;
            }

            // Regular messages - extract text/thinking and preserve user image parts
            let mut text_parts: Vec<String> = Vec::new();
            let mut user_parts: Vec<Value> = Vec::new();
            let mut has_user_images = false;

            for content in &msg.content {
                match content {
                    Content::Text { text } => {
                        if !text.is_empty() {
                            text_parts.push(text.clone());
                            if role == "user" {
                                user_parts.push(self.user_text_part(text));
                            }
                        }
                    }
                    // Preserve thinking blocks as formatted text
                    // This maintains context for reasoning models
                    Content::Thinking { thinking, .. } => {
                        if !thinking.is_empty() {
                            let formatted = format!("[Thinking]\n{}\n[/Thinking]", thinking);
                            text_parts.push(formatted.clone());
                            if role == "user" {
                                user_parts.push(self.user_text_part(&formatted));
                            }
                        }
                    }
                    Content::Image { image, detail } if role == "user" => {
                        if let Some(part) = self.user_image_part(image, detail.as_deref()) {
                            user_parts.push(part);
                            has_user_images = true;
                        }
                    }
                    _ => {}
                }
            }

            let text = text_parts.join("\n\n");

            if role == "user" && has_user_images && !user_parts.is_empty() {
                result.push(serde_json::json!({
                    "role": role,
                    "content": user_parts
                }));
                last_role = Some(role);
            } else if !text.is_empty() {
                result.push(serde_json::json!({
                    "role": role,
                    "content": text
                }));
                last_role = Some(role);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{Content, ImageContent, ModelMessage, Role};

    #[test]
    fn convert_messages_openai_chat_preserves_user_image_parts() {
        let format = OpenAIFormat::new(ApiFormat::OpenAI);
        let messages = vec![ModelMessage {
            role: Role::User,
            content: vec![
                Content::Text {
                    text: "Describe this".to_string(),
                },
                Content::Image {
                    image: ImageContent {
                        url: None,
                        base64: Some("AAA".to_string()),
                        media_type: Some("image/jpeg".to_string()),
                    },
                    detail: Some("high".to_string()),
                },
            ],
        }];

        let converted = format.convert_messages(&messages, None);
        assert_eq!(converted.len(), 1);
        let content = converted[0]
            .get("content")
            .and_then(|c| c.as_array())
            .expect("content should be a multimodal array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Describe this");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/jpeg;base64,AAA");
        assert_eq!(content[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn convert_messages_openai_responses_uses_input_image() {
        let format = OpenAIFormat::new(ApiFormat::OpenAIResponses);
        let messages = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Image {
                image: ImageContent {
                    url: Some("https://example.com/cat.png".to_string()),
                    base64: None,
                    media_type: None,
                },
                detail: Some("low".to_string()),
            }],
        }];

        let converted = format.convert_messages(&messages, None);
        assert_eq!(converted.len(), 1);
        let content = converted[0]
            .get("content")
            .and_then(|c| c.as_array())
            .expect("content should be a multimodal array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "input_image");
        assert_eq!(content[0]["image_url"], "https://example.com/cat.png");
        assert_eq!(content[0]["detail"], "low");
    }
}
