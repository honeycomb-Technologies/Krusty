//! Google Gemini SSE parser for streaming responses

use anyhow::Result;
use serde_json::Value;

use crate::ai::sse::{SseEvent, SseParser};
use crate::ai::types::{AiToolCall, FinishReason};
use crate::ai::usage::parse_google_usage;

/// Google Gemini SSE parser
///
/// Parses the Google AI streaming response format:
/// ```json
/// {"candidates": [{"content": {"parts": [{"text": "..."}], "role": "model"}, "finishReason": "STOP"}]}
/// ```
pub struct GoogleParser;

impl GoogleParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse Google finish reason to our FinishReason enum
    fn parse_finish_reason(reason: &str) -> FinishReason {
        match reason {
            "STOP" => FinishReason::Stop,
            "MAX_TOKENS" => FinishReason::Length,
            "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => {
                FinishReason::ContentFilter
            }
            _ => FinishReason::Other(reason.to_string()),
        }
    }

    fn parse_frame(&self, json: &Value) -> Vec<SseEvent> {
        let mut events = Vec::new();
        let mut usage = parse_google_usage(json);
        let mut has_tool_calls = false;

        if let Some(candidate) = json
            .get("candidates")
            .and_then(|candidates| candidates.as_array())
            .and_then(|candidates| candidates.first())
        {
            if let Some(parts) = candidate
                .get("content")
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.as_array())
            {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|text| text.as_str()) {
                        if !text.is_empty() {
                            events.push(SseEvent::TextDelta(text.to_string()));
                        }
                    }

                    if let Some(function_call) = part.get("functionCall") {
                        let name = function_call
                            .get("name")
                            .and_then(|name| name.as_str())
                            .unwrap_or("");
                        if name.is_empty() {
                            continue;
                        }

                        has_tool_calls = true;
                        let id = function_call
                            .get("id")
                            .and_then(|id| id.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("google_{}", uuid::Uuid::new_v4()));
                        let arguments = function_call
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        events.push(SseEvent::ToolCallStart {
                            id: id.clone(),
                            name: name.to_string(),
                        });
                        events.push(SseEvent::ToolCallComplete(AiToolCall {
                            id,
                            name: name.to_string(),
                            arguments,
                        }));
                    }
                }
            }

            if let Some(reason) = candidate
                .get("finishReason")
                .and_then(|reason| reason.as_str())
            {
                let mut reason = Self::parse_finish_reason(reason);
                if has_tool_calls && reason == FinishReason::Stop {
                    reason = FinishReason::ToolCalls;
                }
                events.push(SseEvent::Finish {
                    reason,
                    usage: usage.take(),
                });
            }
        }

        if let Some(usage) = usage {
            events.push(SseEvent::Usage(usage));
        }
        if events.is_empty() {
            events.push(SseEvent::Skip);
        }
        events
    }
}

impl Default for GoogleParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SseParser for GoogleParser {
    async fn parse_event(&self, json: &Value) -> Result<SseEvent> {
        Ok(self
            .parse_frame(json)
            .into_iter()
            .next()
            .unwrap_or(SseEvent::Skip))
    }

    async fn parse_events(&self, json: &Value) -> Result<Vec<SseEvent>> {
        Ok(self.parse_frame(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::Usage;
    use serde_json::json;

    #[tokio::test]
    async fn cached_prompt_tokens_are_not_counted_twice() {
        let event = GoogleParser::new()
            .parse_event(&json!({
                "usageMetadata": {
                    "promptTokenCount": 1_000,
                    "cachedContentTokenCount": 700,
                    "candidatesTokenCount": 50,
                    "totalTokenCount": 1_070
                }
            }))
            .await
            .expect("usage event should parse");

        let SseEvent::Usage(usage) = event else {
            panic!("expected usage event");
        };
        assert_eq!(
            usage,
            Usage {
                prompt_tokens: 300,
                completion_tokens: 50,
                reasoning_tokens: 0,
                total_tokens: 1_070,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 700,
            }
        );
        assert_eq!(usage.input_tokens(), 1_000);
    }

    #[tokio::test]
    async fn final_frame_preserves_text_usage_and_finish() {
        let events = GoogleParser::new()
            .parse_events(&json!({
                "candidates": [{
                    "content": {"parts": [{"text": "done"}]},
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 100,
                    "candidatesTokenCount": 4,
                    "totalTokenCount": 104
                }
            }))
            .await
            .expect("final frame should parse");

        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], SseEvent::TextDelta(text) if text == "done"));
        assert!(matches!(
            &events[1],
            SseEvent::Finish {
                reason: FinishReason::Stop,
                usage: Some(Usage {
                    total_tokens: 104,
                    ..
                })
            }
        ));
    }

    #[tokio::test]
    async fn thought_tokens_are_preserved_in_completion_and_total_usage() {
        let event = GoogleParser::new()
            .parse_event(&json!({
                "usageMetadata": {
                    "promptTokenCount": 1_000,
                    "candidatesTokenCount": 50,
                    "thoughtsTokenCount": 500,
                    "totalTokenCount": 1_550
                }
            }))
            .await
            .expect("usage event should parse");

        let SseEvent::Usage(usage) = event else {
            panic!("expected usage event");
        };
        assert_eq!(usage.prompt_tokens, 1_000);
        assert_eq!(usage.completion_tokens, 550);
        assert_eq!(usage.reasoning_tokens, 500);
        assert_eq!(usage.total_tokens, 1_550);
        assert_eq!(usage.logical_total_tokens(), 1_550);
    }

    #[tokio::test]
    async fn function_call_is_completed_before_tool_finish() {
        let events = GoogleParser::new()
            .parse_events(&json!({
                "candidates": [{
                    "content": {"parts": [{
                        "functionCall": {
                            "name": "read",
                            "args": {"path": "README.md"}
                        }
                    }]},
                    "finishReason": "STOP"
                }]
            }))
            .await
            .expect("tool frame should parse");

        assert_eq!(events.len(), 3);
        let SseEvent::ToolCallStart { id, name } = &events[0] else {
            panic!("expected tool start");
        };
        assert_eq!(name, "read");
        assert!(matches!(
            &events[1],
            SseEvent::ToolCallComplete(call)
                if call.id == *id
                    && call.name == "read"
                    && call.arguments == json!({"path": "README.md"})
        ));
        assert!(matches!(
            &events[2],
            SseEvent::Finish {
                reason: FinishReason::ToolCalls,
                usage: None
            }
        ));
    }

    #[test]
    fn safety_finishes_are_not_treated_as_success() {
        assert_eq!(
            GoogleParser::parse_finish_reason("SAFETY"),
            FinishReason::ContentFilter
        );
        assert!(matches!(
            GoogleParser::parse_finish_reason("OTHER"),
            FinishReason::Other(reason) if reason == "OTHER"
        ));
    }
}
