use serde_json::Value;

use super::OpenAIParser;
use crate::ai::sse::SseEvent;
use crate::ai::types::{FinishReason, Usage};

fn parse_chat_usage(json: &Value) -> Option<Usage> {
    let usage = json.get("usage")?;
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|tokens| tokens.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|tokens| tokens.as_u64())
        .unwrap_or(0) as usize;
    let cached_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(|tokens| tokens.as_u64())
        .or_else(|| {
            usage
                .get("cache_read_input_tokens")
                .and_then(|tokens| tokens.as_u64())
        })
        .unwrap_or(0) as usize;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|tokens| tokens.as_u64())
        .unwrap_or((prompt_tokens + completion_tokens) as u64) as usize;

    (prompt_tokens > 0 || completion_tokens > 0 || cached_tokens > 0).then_some(Usage {
        prompt_tokens: prompt_tokens.saturating_sub(cached_tokens),
        completion_tokens,
        total_tokens,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cached_tokens,
    })
}

impl OpenAIParser {
    pub(super) fn parse_chat_completions_event(&self, json: &Value) -> anyhow::Result<SseEvent> {
        let usage = parse_chat_usage(json);
        let choices = json.get("choices").and_then(|c| c.as_array());

        if let Some(choices) = choices {
            if let Some(choice) = choices.first() {
                if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                    if reason == "tool_calls" {
                        let tool_calls = self.drain_tool_calls()?;

                        if tool_calls.is_empty() {
                            return Ok(SseEvent::Finish {
                                reason: FinishReason::ToolCalls,
                                usage,
                            });
                        }
                        return Ok(SseEvent::FinishWithToolCalls { tool_calls, usage });
                    }
                    let reason = match reason {
                        "stop" | "end_turn" => FinishReason::Stop,
                        "length" | "max_tokens" => FinishReason::Length,
                        "content_filter" => FinishReason::ContentFilter,
                        other => FinishReason::Other(other.to_string()),
                    };
                    return Ok(SseEvent::Finish { reason, usage });
                }

                if let Some(delta) = choice.get("delta") {
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            return Ok(SseEvent::TextDelta(content.to_string()));
                        }
                    }

                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str())
                    {
                        if !reasoning.is_empty() {
                            return Ok(SseEvent::ThinkingDelta {
                                index: 0,
                                thinking: reasoning.to_string(),
                            });
                        }
                    }

                    if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                        for tool_call in tool_calls {
                            let index = tool_call.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                as usize;
                            let key = format!("chat-index:{}", index);

                            if let Some(function) = tool_call.get("function") {
                                let id = tool_call
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let tool_id = if id.is_empty() { key.clone() } else { id };
                                let mut emitted_event: Option<SseEvent> = None;

                                if let Some(name) = function.get("name").and_then(|n| n.as_str()) {
                                    let inserted =
                                        self.register_tool_call(key.clone(), &tool_id, name, None)?;
                                    if inserted {
                                        emitted_event = Some(SseEvent::ToolCallStart {
                                            id: tool_id.clone(),
                                            name: name.to_string(),
                                        });
                                    }
                                }

                                if let Some(args) =
                                    function.get("arguments").and_then(|a| a.as_str())
                                {
                                    if let Some(id) = self.append_tool_arguments(&key, args)? {
                                        if emitted_event.is_none() {
                                            emitted_event = Some(SseEvent::ToolCallDelta {
                                                id,
                                                delta: args.to_string(),
                                            });
                                        }
                                    }
                                }

                                if let Some(event) = emitted_event {
                                    return Ok(event);
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(usage) = usage {
            return Ok(SseEvent::Usage(usage));
        }

        Ok(SseEvent::Skip)
    }
}
