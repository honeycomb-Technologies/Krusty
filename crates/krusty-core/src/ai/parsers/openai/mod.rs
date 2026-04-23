//! OpenAI-compatible SSE parser for chat/completions format

mod chat;
mod responses;
mod state;

use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use crate::ai::sse::{SseEvent, SseParser, ToolCallAccumulator};

/// OpenAI-compatible SSE parser for chat/completions format
pub struct OpenAIParser {
    /// Track tool calls being accumulated
    tool_accumulators: std::sync::Mutex<HashMap<String, ToolCallAccumulator>>,
    /// Preserve tool call ordering for deterministic completion
    tool_order: std::sync::Mutex<Vec<String>>,
    /// Map Responses API item ids to call ids for interleaved argument deltas
    response_item_to_call: std::sync::Mutex<HashMap<String, String>>,
}

impl OpenAIParser {
    pub fn new() -> Self {
        Self {
            tool_accumulators: std::sync::Mutex::new(HashMap::new()),
            tool_order: std::sync::Mutex::new(Vec::new()),
            response_item_to_call: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl Default for OpenAIParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SseParser for OpenAIParser {
    async fn parse_event(&self, json: &Value) -> Result<SseEvent> {
        if let Some(error) = json.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            let error_type = error
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            return Err(anyhow::anyhow!(
                "OpenAI API error ({}): {}",
                error_type,
                message
            ));
        }

        if let Some(event_type) = json.get("type").and_then(|t| t.as_str()) {
            if event_type == "error" || event_type.contains("error") {
                let message = json
                    .get("message")
                    .and_then(|m| m.as_str())
                    .or_else(|| json.get("error").and_then(|e| e.as_str()))
                    .unwrap_or("Unknown error");
                return Err(anyhow::anyhow!("OpenAI Responses API error: {}", message));
            }
            return self.parse_responses_api_event(json, event_type);
        }

        self.parse_chat_completions_event(json)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::OpenAIParser;
    use crate::ai::sse::SseEvent;

    #[test]
    fn responses_output_item_done_does_not_duplicate_arguments() {
        let parser = OpenAIParser::new();
        let args = "{\"pattern\":\"**/*prompt*\",\"path\":\".\"}";

        let start_event = json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "glob",
                "arguments": args
            }
        });
        let event = parser
            .parse_responses_api_event(&start_event, "response.output_item.added")
            .expect("start event should parse");
        assert!(matches!(event, SseEvent::ToolCallStart { .. }));

        let done_event = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "glob",
                "arguments": args
            }
        });
        let event = parser
            .parse_responses_api_event(&done_event, "response.output_item.done")
            .expect("done event should parse");
        assert!(matches!(event, SseEvent::Skip));

        let finish_event = json!({
            "type": "response.done",
            "response": {
                "status": "incomplete",
                "incomplete_details": {
                    "reason": "tool_calls"
                }
            }
        });
        let event = parser
            .parse_responses_api_event(&finish_event, "response.done")
            .expect("finish event should parse");

        let SseEvent::FinishWithToolCalls { tool_calls, .. } = event else {
            panic!("expected FinishWithToolCalls event");
        };
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].arguments["pattern"], "**/*prompt*");
        assert_eq!(tool_calls[0].arguments["path"], ".");
        assert!(tool_calls[0].arguments.get("raw").is_none());
    }

    #[test]
    fn responses_output_item_done_replaces_partial_snapshot() {
        let parser = OpenAIParser::new();

        let start_event = json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "glob",
                "arguments": "{\"pattern\":\"**/*prompt*\""
            }
        });
        parser
            .parse_responses_api_event(&start_event, "response.output_item.added")
            .expect("start event should parse");

        let done_event = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "glob",
                "arguments": "{\"pattern\":\"**/*prompt*\",\"path\":\".\"}"
            }
        });
        parser
            .parse_responses_api_event(&done_event, "response.output_item.done")
            .expect("done event should parse");

        let finish_event = json!({
            "type": "response.done",
            "response": {
                "status": "incomplete",
                "incomplete_details": {
                    "reason": "tool_calls"
                }
            }
        });
        let event = parser
            .parse_responses_api_event(&finish_event, "response.done")
            .expect("finish event should parse");

        let SseEvent::FinishWithToolCalls { tool_calls, .. } = event else {
            panic!("expected FinishWithToolCalls event");
        };
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].arguments["pattern"], "**/*prompt*");
        assert_eq!(tool_calls[0].arguments["path"], ".");
        assert!(tool_calls[0].arguments.get("raw").is_none());
    }

    #[test]
    fn responses_argument_deltas_still_accumulate() {
        let parser = OpenAIParser::new();

        let start_event = json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "glob"
            }
        });
        let event = parser
            .parse_responses_api_event(&start_event, "response.output_item.added")
            .expect("start event should parse");
        assert!(matches!(event, SseEvent::ToolCallStart { .. }));

        let delta_one = json!({
            "type": "response.function_call_arguments.delta",
            "call_id": "call_1",
            "delta": "{\"pattern\":\"**/*"
        });
        parser
            .parse_responses_api_event(&delta_one, "response.function_call_arguments.delta")
            .expect("first delta should parse");

        let delta_two = json!({
            "type": "response.function_call_arguments.delta",
            "call_id": "call_1",
            "delta": "prompt*\",\"path\":\".\"}"
        });
        parser
            .parse_responses_api_event(&delta_two, "response.function_call_arguments.delta")
            .expect("second delta should parse");

        let finish_event = json!({
            "type": "response.done",
            "response": {
                "status": "incomplete",
                "incomplete_details": {
                    "reason": "tool_calls"
                }
            }
        });
        let event = parser
            .parse_responses_api_event(&finish_event, "response.done")
            .expect("finish event should parse");

        let SseEvent::FinishWithToolCalls { tool_calls, .. } = event else {
            panic!("expected FinishWithToolCalls event");
        };
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].arguments["pattern"], "**/*prompt*");
        assert_eq!(tool_calls[0].arguments["path"], ".");
    }
}
