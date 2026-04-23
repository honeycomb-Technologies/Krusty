use serde_json::Value;
use tracing::info;

use super::AnthropicParser;
use crate::ai::sse::{parse_finish_reason, SseEvent};
use crate::ai::types::{ContextEditingMetrics, FinishReason, Usage};

impl AnthropicParser {
    pub(super) fn parse_message_delta(&self, json: &Value) -> SseEvent {
        if let Some(usage) = json.get("usage") {
            let input_tokens = usage
                .get("input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;
            let output_tokens = usage
                .get("output_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;

            if input_tokens > 0 || output_tokens > 0 {
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as usize;
                let cache_creation = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as usize;
                return SseEvent::Usage(Usage {
                    prompt_tokens: input_tokens,
                    completion_tokens: output_tokens,
                    total_tokens: input_tokens + output_tokens,
                    cache_creation_input_tokens: cache_creation,
                    cache_read_input_tokens: cache_read,
                });
            }
        }

        if let Some(delta) = json.get("delta") {
            if let Some(stop_reason) = delta.get("stop_reason").and_then(|s| s.as_str()) {
                let reason = parse_finish_reason(stop_reason);
                return SseEvent::Finish {
                    reason,
                    usage: None,
                };
            }
        }

        SseEvent::Skip
    }

    pub(super) fn parse_message_start(&self, json: &Value) -> SseEvent {
        if let Some(message) = json.get("message") {
            if let Some(ctx_edit) = message.get("context_editing") {
                let metrics = ContextEditingMetrics {
                    cleared_tool_uses: ctx_edit
                        .get("cleared_tool_uses")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    cleared_thinking_turns: ctx_edit
                        .get("cleared_thinking_turns")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    cleared_input_tokens: ctx_edit
                        .get("cleared_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                };
                if metrics.cleared_input_tokens > 0 {
                    info!(
                        "Context edited: cleared {} tokens ({} tool uses, {} thinking turns)",
                        metrics.cleared_input_tokens,
                        metrics.cleared_tool_uses,
                        metrics.cleared_thinking_turns
                    );
                }
                return SseEvent::ContextEdited(metrics);
            }

            if let Some(usage) = message.get("usage") {
                let input_tokens = usage
                    .get("input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as usize;
                let cache_creation = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as usize;
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0) as usize;

                if cache_creation > 0 || cache_read > 0 {
                    info!(
                        "Cache metrics: read={}, created={}, fresh={}",
                        cache_read, cache_creation, input_tokens
                    );
                }

                let total_input = input_tokens + cache_creation + cache_read;
                return SseEvent::Usage(Usage {
                    prompt_tokens: total_input,
                    completion_tokens: 0,
                    total_tokens: total_input,
                    cache_creation_input_tokens: cache_creation,
                    cache_read_input_tokens: cache_read,
                });
            }
        }
        SseEvent::Skip
    }

    pub(super) fn parse_message_stop(&self) -> SseEvent {
        SseEvent::Finish {
            reason: FinishReason::Stop,
            usage: None,
        }
    }

    pub(super) fn parse_error_event(&self, json: &Value) -> anyhow::Result<SseEvent> {
        let error_msg = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error");
        Err(anyhow::anyhow!("API error: {}", error_msg))
    }
}
