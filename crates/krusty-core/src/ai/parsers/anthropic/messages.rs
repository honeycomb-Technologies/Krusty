use serde_json::Value;
use tracing::info;

use super::AnthropicParser;
use crate::ai::retry::safe_provider_event_error;
use crate::ai::sse::{parse_finish_reason, SseEvent};
use crate::ai::types::{ContextEditingMetrics, FinishReason};
use crate::ai::usage::parse_anthropic_usage;

impl AnthropicParser {
    pub(super) fn parse_message_delta(&self, json: &Value) -> SseEvent {
        let usage = json.get("usage").and_then(parse_anthropic_usage);

        if let Some(delta) = json.get("delta") {
            if let Some(stop_reason) = delta.get("stop_reason").and_then(|s| s.as_str()) {
                let reason = parse_finish_reason(stop_reason);
                return SseEvent::Finish { reason, usage };
            }
        }

        usage.map(SseEvent::Usage).unwrap_or(SseEvent::Skip)
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

            if let Some(usage) = message.get("usage").and_then(parse_anthropic_usage) {
                if usage.cache_creation_input_tokens > 0 || usage.cache_read_input_tokens > 0 {
                    info!(
                        "Cache metrics: read={}, created={}, fresh={}",
                        usage.cache_read_input_tokens,
                        usage.cache_creation_input_tokens,
                        usage.prompt_tokens
                    );
                }
                return SseEvent::Usage(usage);
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
        let error = json.get("error");
        let message = error
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .or_else(|| error.and_then(Value::as_str));
        let category = error
            .and_then(|error| error.get("type"))
            .and_then(Value::as_str);
        let code = error
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str);
        Err(anyhow::Error::msg(safe_provider_event_error(
            "Anthropic API error",
            code,
            category,
            message,
        )))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn provider_error_event_never_reflects_message_type_or_code() {
        const MESSAGE_SENTINEL: &str = "ANTHROPIC_MESSAGE_SENTINEL_4dc7";
        const TYPE_SENTINEL: &str = "ANTHROPIC_TYPE_SENTINEL_e71b";
        const CODE_SENTINEL: &str = "ANTHROPIC_CODE_SENTINEL_20a9";
        let result = AnthropicParser::new().parse_error_event(&json!({
            "type": "error",
            "error": {
                "message": MESSAGE_SENTINEL,
                "type": TYPE_SENTINEL,
                "code": CODE_SENTINEL
            }
        }));
        let error = match result {
            Ok(_) => panic!("provider error event should fail"),
            Err(error) => error.to_string(),
        };
        for sentinel in [MESSAGE_SENTINEL, TYPE_SENTINEL, CODE_SENTINEL] {
            assert!(!error.contains(sentinel));
        }
        assert!(error.contains("message_fingerprint=sha256:"));
        assert!(error.contains("category_fingerprint=sha256:"));
        assert!(error.contains("code_fingerprint=sha256:"));
    }

    #[test]
    fn message_delta_preserves_usage_and_max_tokens_finish() {
        let event = AnthropicParser::new().parse_message_delta(&json!({
            "type": "message_delta",
            "delta": {"stop_reason": "max_tokens"},
            "usage": {"output_tokens": 321}
        }));

        match event {
            SseEvent::Finish {
                reason: FinishReason::Length,
                usage: Some(usage),
            } => assert_eq!(usage.completion_tokens, 321),
            _ => panic!("unexpected Anthropic message-delta event"),
        }
    }

    #[test]
    fn message_start_keeps_cache_buckets_separate() {
        let event = AnthropicParser::new().parse_message_start(&json!({
            "type": "message_start",
            "message": {
                "usage": {
                    "input_tokens": 100,
                    "cache_creation_input_tokens": 200,
                    "cache_read_input_tokens": 700
                }
            }
        }));

        match event {
            SseEvent::Usage(usage) => {
                assert_eq!(usage.prompt_tokens, 100);
                assert_eq!(usage.cache_creation_input_tokens, 200);
                assert_eq!(usage.cache_read_input_tokens, 700);
                assert_eq!(usage.input_tokens(), 1_000);
                assert_eq!(usage.total_tokens, 1_000);
            }
            _ => panic!("unexpected Anthropic message-start event"),
        }
    }
}
