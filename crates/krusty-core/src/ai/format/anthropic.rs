//! Anthropic API format handler
//!
//! Handles message alternation, thinking block preservation, and tool conversion
//! for the Anthropic Messages API.

use serde_json::Value;
use tracing::{debug, info};

use super::{needs_role_alternation_filler, FormatHandler, RequestOptions};
use crate::ai::providers::{ProviderCapabilities, ProviderId};
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
        let strip_images = provider_id
            .map(|pid| !ProviderCapabilities::for_provider(pid).supports_vision)
            .unwrap_or(false);

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
            if let Some(filler_role) = needs_role_alternation_filler(last_role, role, &[]) {
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

            // Determine if this message should include thinking blocks
            let include_thinking =
                preserve_all_thinking || last_assistant_with_tools_idx == Some(i);

            let content: Vec<Value> = msg
                .content
                .iter()
                .filter_map(|c| {
                    convert_content(c, include_thinking, include_signature, strip_images)
                })
                .collect();

            result.push(serde_json::json!({
                "role": role,
                "content": content
            }));

            last_role = Some(role);
        }

        // Safety net: strip orphaned tool_result blocks whose tool_use_id
        // doesn't appear in the immediately preceding assistant message.
        // This prevents API error 2013 from corrupted conversations.
        sanitize_tool_results(&mut result);

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
    strip_images: bool,
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
            if strip_images {
                return Some(serde_json::json!({
                    "type": "text",
                    "text": "[Image attached - use MCP vision tools to analyze]"
                }));
            }
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
            if strip_images {
                return Some(serde_json::json!({
                    "type": "text",
                    "text": "[Document attached - use MCP vision tools to analyze]"
                }));
            }
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

/// Repair tool_use / tool_result pairing in the message array.
///
/// The Anthropic API requires:
/// 1. Every tool_result must reference a tool_use_id from the preceding assistant message.
/// 2. Every tool_use in an assistant message must have a corresponding tool_result in
///    the immediately following user message.
///
/// Corrupted conversations (e.g., from interrupted sessions where AskUser was
/// batched with other tools) can violate both rules, causing error 2013.
///
/// This function:
/// - Strips orphaned tool_results (no matching tool_use)
/// - Injects stub tool_results for missing tool_uses
fn sanitize_tool_results(messages: &mut Vec<Value>) {
    use std::collections::HashSet;

    let mut i = 0;

    while i < messages.len() {
        let role = messages[i]["role"].as_str().unwrap_or("");

        if role == "assistant" {
            // Collect tool_use IDs from this assistant message (preserve order
            // for deterministic stub insertion while keeping O(1) lookup).
            let mut tool_use_ids: Vec<String> = Vec::new();
            let mut tool_use_lookup: HashSet<String> = HashSet::new();
            if let Some(content) = messages[i]["content"].as_array() {
                for block in content {
                    if block["type"].as_str() == Some("tool_use") {
                        if let Some(id) = block["id"].as_str() {
                            let id = id.to_string();
                            if tool_use_lookup.insert(id.clone()) {
                                tool_use_ids.push(id);
                            }
                        }
                    }
                }
            }

            if !tool_use_ids.is_empty() {
                // Check the next message for matching tool_results
                let next_is_user =
                    i + 1 < messages.len() && messages[i + 1]["role"].as_str() == Some("user");

                if next_is_user {
                    let user_msg = &mut messages[i + 1];
                    let content = user_msg["content"].as_array().cloned().unwrap_or_default();

                    // Strip orphaned tool_results (no matching tool_use)
                    let mut filtered: Vec<Value> =
                        Vec::with_capacity(content.len() + tool_use_ids.len());
                    let mut result_ids: HashSet<String> =
                        HashSet::with_capacity(tool_use_ids.len());
                    for block in content {
                        if block["type"].as_str() == Some("tool_result") {
                            let id = block["tool_use_id"].as_str().unwrap_or("");
                            if tool_use_lookup.contains(id) {
                                result_ids.insert(id.to_string());
                                filtered.push(block);
                            } else {
                                debug!("Stripping orphaned tool_result for tool_use_id={}", id);
                            }
                        } else {
                            filtered.push(block);
                        }
                    }

                    // Inject stub results for missing tool_uses
                    for id in &tool_use_ids {
                        if !result_ids.contains(id) {
                            debug!("Injecting stub tool_result for missing tool_use_id={}", id);
                            filtered.push(stub_tool_result(id));
                        }
                    }

                    user_msg["content"] = Value::Array(filtered);
                } else {
                    // No user message follows — inject one with stub results for all tool_uses
                    debug!(
                        "Injecting user message with {} stub tool_results (no user message followed assistant with tool_use)",
                        tool_use_ids.len()
                    );
                    let stubs: Vec<Value> =
                        tool_use_ids.iter().map(|id| stub_tool_result(id)).collect();
                    messages.insert(
                        i + 1,
                        serde_json::json!({
                            "role": "user",
                            "content": stubs
                        }),
                    );
                }
            }
        }

        i += 1;
    }
}

fn stub_tool_result(tool_use_id: &str) -> Value {
    serde_json::json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": "Tool execution was interrupted",
        "is_error": true
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::sanitize_tool_results;

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

        sanitize_tool_results(&mut messages);

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

        sanitize_tool_results(&mut messages);

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
