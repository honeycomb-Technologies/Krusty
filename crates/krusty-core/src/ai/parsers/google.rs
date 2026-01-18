//! Google Gemini SSE parser for streaming responses

use anyhow::Result;
use serde_json::Value;

use crate::ai::sse::{SseEvent, SseParser, ToolCallAccumulator};
use crate::ai::types::FinishReason;

/// Google Gemini SSE parser
///
/// Parses the Google AI streaming response format:
/// ```json
/// {"candidates": [{"content": {"parts": [{"text": "..."}], "role": "model"}, "finishReason": "STOP"}]}
/// ```
pub struct GoogleParser {
    /// Track tool calls being accumulated
    tool_accumulators: std::sync::Mutex<std::collections::HashMap<usize, ToolCallAccumulator>>,
}

impl GoogleParser {
    pub fn new() -> Self {
        Self {
            tool_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Parse Google finish reason to our FinishReason enum
    fn parse_finish_reason(reason: &str) -> FinishReason {
        match reason {
            "STOP" => FinishReason::Stop,
            "MAX_TOKENS" => FinishReason::Length,
            "SAFETY" | "RECITATION" | "OTHER" => FinishReason::Stop,
            _ => FinishReason::Other(reason.to_string()),
        }
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
        // Google Gemini format: {"candidates": [{...}]}
        if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
            if let Some(candidate) = candidates.first() {
                // Check for finish reason
                if let Some(finish_reason) = candidate.get("finishReason").and_then(|f| f.as_str())
                {
                    // If there's content with finish, extract it first
                    if let Some(content) = candidate.get("content") {
                        if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                            for part in parts {
                                // Text content
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        // Return text delta, finish will be next chunk
                                        return Ok(SseEvent::TextDelta(text.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    // Return finish event
                    return Ok(SseEvent::Finish {
                        reason: Self::parse_finish_reason(finish_reason),
                    });
                }

                // Extract content parts
                if let Some(content) = candidate.get("content") {
                    if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                        for part in parts {
                            // Text content
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    return Ok(SseEvent::TextDelta(text.to_string()));
                                }
                            }

                            // Function call (tool use)
                            if let Some(function_call) = part.get("functionCall") {
                                let name = function_call
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let args = function_call
                                    .get("args")
                                    .map(|a| serde_json::to_string(a).unwrap_or_default())
                                    .unwrap_or_default();

                                if !name.is_empty() {
                                    // Generate a unique ID for the tool call
                                    let id = format!("google_{}", uuid::Uuid::new_v4());

                                    // Store accumulator
                                    let mut accumulators = self.tool_accumulators.lock().unwrap();
                                    let index = accumulators.len();
                                    let mut acc = ToolCallAccumulator::new(id.clone(), name.clone());
                                    acc.add_arguments(&args);
                                    accumulators.insert(index, acc);

                                    return Ok(SseEvent::ToolCallStart { id, name });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check for usage metadata
        if let Some(usage) = json.get("usageMetadata") {
            let prompt = usage
                .get("promptTokenCount")
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;
            let completion = usage
                .get("candidatesTokenCount")
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;
            if prompt > 0 || completion > 0 {
                return Ok(SseEvent::Usage(crate::ai::types::Usage {
                    prompt_tokens: prompt,
                    completion_tokens: completion,
                    total_tokens: prompt + completion,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }));
            }
        }

        Ok(SseEvent::Skip)
    }
}
