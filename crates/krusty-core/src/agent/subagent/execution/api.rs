use serde_json::{json, Value};
use std::time::Instant;
use tracing::info;

use crate::ai::client::AiClient;
use crate::ai::retry::{with_retry, RetryConfig};
use crate::ai::types::{AiTool, Content, ModelMessage, Role};

use super::super::types::{SubAgentApiError, ToolCall};

/// Make a non-streaming API call for a sub-agent with retry logic.
pub(super) async fn call_subagent_api(
    client: &AiClient,
    model: &str,
    system: &str,
    messages: &[ModelMessage],
    tools: &[AiTool],
    max_tokens: usize,
    thinking_enabled: bool,
) -> Result<Value, SubAgentApiError> {
    info!(
        model = model,
        msg_count = messages.len(),
        "SubAgent API call starting"
    );
    let start = Instant::now();

    let messages_json: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "user",
                Role::Tool => "user",
            };

            let content: Vec<Value> = m
                .content
                .iter()
                .map(|c| match c {
                    Content::Text { text } => json!({"type": "text", "text": text}),
                    Content::ToolUse { id, name, input } => {
                        json!({"type": "tool_use", "id": id, "name": name, "input": input})
                    }
                    Content::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                    } => {
                        let content_str = match output {
                            Value::String(s) => Value::String(s.clone()),
                            other => Value::String(other.to_string()),
                        };
                        json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content_str,
                            "is_error": is_error.unwrap_or(false)
                        })
                    }
                    _ => json!({"type": "text", "text": "[unsupported content]"}),
                })
                .collect();

            json!({"role": role, "content": content})
        })
        .collect();

    let mut sorted_tools: Vec<_> = tools.iter().collect();
    sorted_tools.sort_by(|a, b| a.name.cmp(&b.name));
    let tools_json: Vec<Value> = sorted_tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })
        })
        .collect();

    let config = RetryConfig::aggressive();

    let result = with_retry(&config, || async {
        client
            .call_with_tools(
                model,
                system,
                messages_json.clone(),
                tools_json.clone(),
                max_tokens,
                thinking_enabled,
            )
            .await
            .map_err(SubAgentApiError::from)
    })
    .await;

    let elapsed = start.elapsed();
    info!(
        elapsed_ms = elapsed.as_millis() as u64,
        success = result.is_ok(),
        "SubAgent API call completed"
    );
    result
}

/// Parse API response to extract text, tool calls, and stop reason.
pub(super) fn parse_response(response: &Value) -> (Vec<String>, Vec<ToolCall>, String) {
    let mut texts = vec![];
    let mut tool_calls = vec![];

    let stop_reason = response
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
        for block in content {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match block_type {
                "text" => {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        texts.push(text.to_string());
                    }
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(json!({}));

                    tool_calls.push(ToolCall { id, name, input });
                }
                _ => {}
            }
        }
    }

    (texts, tool_calls, stop_reason)
}
