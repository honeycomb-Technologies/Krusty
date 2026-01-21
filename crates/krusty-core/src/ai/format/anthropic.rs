//! Anthropic API format handler
//!
//! Handles message alternation, thinking block preservation, and tool conversion
//! for the Anthropic Messages API.

use serde_json::Value;
use tracing::{debug, info};

use super::{FormatHandler, RequestOptions};
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, Content, ModelMessage, Role};

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
    /// Convert domain messages to Anthropic format
    ///
    /// CRITICAL: This function ensures proper message alternation required by the API.
    /// The API requires user/assistant messages to strictly alternate. If there are
    /// consecutive user messages (e.g., tool_result followed by user text without
    /// assistant response between), we must insert an empty assistant message.
    ///
    /// THINKING BLOCKS: Provider-specific handling:
    /// - MiniMax: Preserve ALL thinking blocks (per their docs), no signature field needed
    /// - Anthropic: Only preserve last thinking with pending tools (signature validation)
    fn convert_messages(
        &self,
        messages: &[ModelMessage],
        provider_id: Option<ProviderId>,
    ) -> Vec<Value> {
        let mut result: Vec<Value> = Vec::new();
        let mut last_role: Option<&str> = None;

        info!("Converting {} messages for Anthropic API", messages.len());

        // MiniMax: Preserve ALL thinking blocks (per their docs)
        // Anthropic: Only preserve last thinking with pending tools (signature validation)
        let preserve_all_thinking = provider_id == Some(ProviderId::MiniMax);
        let include_signature = provider_id != Some(ProviderId::MiniMax);

        // Determine which assistant message (if any) should keep thinking blocks.
        // This is the last assistant message that has tool_use AND is followed by tool_result.
        // Only used for Anthropic (when not preserving all thinking).
        let non_system_messages: Vec<_> =
            messages.iter().filter(|m| m.role != Role::System).collect();

        let last_assistant_with_tools_idx = if preserve_all_thinking {
            None // Not needed when preserving all thinking
        } else {
            let mut idx = None;
            for (i, msg) in non_system_messages.iter().enumerate() {
                if msg.role == Role::Assistant
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::ToolUse { .. }))
                {
                    // Check if followed by tool result
                    if i + 1 < non_system_messages.len()
                        && (non_system_messages[i + 1].role == Role::Tool
                            || non_system_messages[i + 1]
                                .content
                                .iter()
                                .any(|c| matches!(c, Content::ToolResult { .. })))
                    {
                        idx = Some(i);
                    }
                }
            }
            idx
        };

        for (i, msg) in non_system_messages.iter().enumerate() {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "user", // Tool results come as user messages
                Role::System => unreachable!(),
            };

            // Check for consecutive same-role messages
            // API requires strict user/assistant alternation
            if let Some(prev_role) = last_role {
                if prev_role == role {
                    // Insert minimal message of opposite role to maintain alternation
                    let filler_role = if role == "user" { "assistant" } else { "user" };
                    debug!(
                        "Inserting filler {} message to maintain alternation",
                        filler_role
                    );
                    result.push(serde_json::json!({
                        "role": filler_role,
                        "content": [{
                            "type": "text",
                            "text": "."
                        }]
                    }));
                }
            }

            // Determine if this message should include thinking blocks
            let include_thinking =
                preserve_all_thinking || last_assistant_with_tools_idx == Some(i);

            let content: Vec<Value> = msg
                .content
                .iter()
                .filter_map(|c| convert_content(c, include_thinking, include_signature))
                .collect();

            result.push(serde_json::json!({
                "role": role,
                "content": content
            }));

            last_role = Some(role);
        }

        result
    }

    fn convert_tools(&self, tools: &[AiTool]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect()
    }

    fn build_request_body(
        &self,
        model: &str,
        messages: Vec<Value>,
        options: &RequestOptions,
    ) -> Value {
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "max_tokens": options.max_tokens,
        });

        if options.streaming {
            body["stream"] = serde_json::json!(true);
        }

        if let Some(system) = options.system_prompt {
            body["system"] = serde_json::json!(system);
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

/// Convert a single content block to Anthropic JSON format
///
/// # Arguments
/// * `content` - The content block to convert
/// * `include_thinking` - Whether to include thinking blocks
/// * `include_signature` - Whether to include signature field in thinking blocks
///   (Anthropic requires signature, MiniMax doesn't need it)
fn convert_content(
    content: &Content,
    include_thinking: bool,
    include_signature: bool,
) -> Option<Value> {
    match content {
        Content::Text { text } => Some(serde_json::json!({
            "type": "text",
            "text": text
        })),
        Content::ToolUse { id, name, input } => Some(serde_json::json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input
        })),
        Content::ToolResult {
            tool_use_id,
            output,
            is_error,
        } => Some(serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": output,
            "is_error": is_error.unwrap_or(false)
        })),
        Content::Image { image, detail: _ } => {
            if let Some(base64_data) = &image.base64 {
                Some(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": image.media_type.clone().unwrap_or_else(|| "image/png".to_string()),
                        "data": base64_data
                    }
                }))
            } else if let Some(url) = &image.url {
                Some(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": url
                    }
                }))
            } else {
                Some(serde_json::json!({
                    "type": "text",
                    "text": "[Invalid image content]"
                }))
            }
        }
        Content::Document { source } => {
            if let Some(data) = &source.data {
                Some(serde_json::json!({
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": source.media_type,
                        "data": data
                    }
                }))
            } else if let Some(url) = &source.url {
                Some(serde_json::json!({
                    "type": "document",
                    "source": {
                        "type": "url",
                        "url": url
                    }
                }))
            } else {
                Some(serde_json::json!({
                    "type": "text",
                    "text": "[Invalid document content]"
                }))
            }
        }
        // Provider-specific thinking block handling:
        // - Anthropic: Include signature (required for validation)
        // - MiniMax: No signature field (matches their API format)
        Content::Thinking {
            thinking,
            signature,
        } => {
            if include_thinking {
                if include_signature {
                    Some(serde_json::json!({
                        "type": "thinking",
                        "thinking": thinking,
                        "signature": signature
                    }))
                } else {
                    // MiniMax: No signature field needed
                    Some(serde_json::json!({
                        "type": "thinking",
                        "thinking": thinking
                    }))
                }
            } else {
                None // Strip thinking from other messages
            }
        }
        Content::RedactedThinking { data } => {
            if include_thinking {
                Some(serde_json::json!({
                    "type": "redacted_thinking",
                    "data": data
                }))
            } else {
                None // Strip redacted thinking from other messages
            }
        }
    }
}
