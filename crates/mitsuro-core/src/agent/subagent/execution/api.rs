use serde_json::{json, Value};
use std::time::Instant;
use tracing::info;

use crate::ai::client::{AiClient, RemoteAttemptPolicy};
use crate::ai::providers::ReasoningEffort;
use crate::ai::types::{AiTool, ModelMessage, Usage};
use crate::ai::usage::{
    parse_anthropic_usage, parse_google_usage, parse_openai_chat_usage,
    parse_openai_responses_usage,
};

use super::super::types::{SubAgentApiError, ToolCall};

/// Make a non-streaming API call for a sub-agent with retry logic.
pub(super) async fn call_subagent_api(
    client: &AiClient,
    model: &str,
    system: &str,
    messages: &[ModelMessage],
    tools: &[AiTool],
    max_tokens: usize,
    reasoning_effort: Option<ReasoningEffort>,
    session_id: &str,
    prompt_cache_key: Option<&str>,
    attempt_policy: RemoteAttemptPolicy,
) -> Result<Value, SubAgentApiError> {
    info!(
        model = model,
        msg_count = messages.len(),
        "SubAgent API call starting"
    );
    let start = Instant::now();

    let result = client
        .call_with_tools_at_reasoning_and_attempt_policy(
            model,
            system,
            messages,
            tools,
            max_tokens,
            reasoning_effort,
            Some(session_id),
            prompt_cache_key,
            attempt_policy,
        )
        .await
        .map_err(SubAgentApiError::from);

    let elapsed = start.elapsed();
    info!(
        elapsed_ms = elapsed.as_millis() as u64,
        success = result.is_ok(),
        "SubAgent API call completed"
    );
    result
}

pub(super) fn parse_response_usage(response: &Value) -> Option<Usage> {
    parse_openai_responses_usage(response)
        .or_else(|| parse_openai_chat_usage(response))
        .or_else(|| parse_anthropic_usage(response))
        .or_else(|| parse_google_usage(response))
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
