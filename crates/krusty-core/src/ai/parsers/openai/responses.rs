use serde_json::Value;

use super::OpenAIParser;
use crate::ai::retry::safe_provider_code;
use crate::ai::sse::SseEvent;
use crate::ai::types::FinishReason;
use crate::ai::usage::parse_openai_responses_usage;

impl OpenAIParser {
    fn reasoning_completion_key(json: &Value) -> String {
        let item_id = json
            .get("item_id")
            .and_then(Value::as_str)
            .unwrap_or("default-item");
        let output_index = json
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let summary_index = json
            .get("summary_index")
            .or_else(|| json.get("content_index"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        format!("{item_id}:{output_index}:{summary_index}")
    }

    fn responses_snapshot_text(json: &Value) -> String {
        let response = json.get("response").unwrap_or(json);
        if let Some(text) = response
            .get("output_text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            return text.to_string();
        }

        let mut text = String::new();
        let Some(output) = response.get("output").and_then(Value::as_array) else {
            return text;
        };

        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    let Some(parts) = item.get("content").and_then(Value::as_array) else {
                        continue;
                    };
                    for part in parts {
                        if matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("output_text" | "text")
                        ) {
                            if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                                text.push_str(part_text);
                            }
                        }
                    }
                }
                Some("output_text" | "text") => {
                    if let Some(item_text) = item.get("text").and_then(Value::as_str) {
                        text.push_str(item_text);
                    }
                }
                _ => {}
            }
        }

        text
    }

    pub(super) fn final_response_snapshot_delta(
        &self,
        json: &Value,
    ) -> anyhow::Result<Option<String>> {
        self.reconcile_response_text_snapshot(&Self::responses_snapshot_text(json))
    }

    fn responses_finish_reason(response: Option<&Value>) -> FinishReason {
        let Some(response) = response else {
            return FinishReason::Stop;
        };

        let Some(status) = response.get("status").and_then(|s| s.as_str()) else {
            return FinishReason::Stop;
        };

        match status {
            "completed" => FinishReason::Stop,
            "incomplete" => {
                let reason = response
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("incomplete");
                match reason {
                    "max_output_tokens" | "max_tokens" | "length" => FinishReason::Length,
                    "tool_calls" | "tool_use" => FinishReason::ToolCalls,
                    other => {
                        FinishReason::Other(format!("incomplete:{}", safe_provider_code(other)))
                    }
                }
            }
            "cancelled" => FinishReason::Other("cancelled".to_string()),
            "failed" => FinishReason::Other("failed".to_string()),
            other => FinishReason::Other(safe_provider_code(other)),
        }
    }

    /// Parse OpenAI Responses API event format.
    /// Used by GPT-5-class models on OpenAI Responses-compatible transports.
    pub(super) fn parse_responses_api_event(
        &self,
        json: &Value,
        event_type: &str,
    ) -> anyhow::Result<SseEvent> {
        let event_kind = match event_type {
            "response.output_text.delta" => "output_text_delta",
            "response.output_text.done" => "output_text_done",
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                "reasoning_delta"
            }
            "response.reasoning_summary_part.added" => "reasoning_start",
            "response.reasoning_summary_text.done"
            | "response.reasoning_text.done"
            | "response.reasoning_summary_part.done" => "reasoning_done",
            "response.done" | "response.completed" => "response_complete",
            "response.function_call_arguments.start"
            | "response.output_item.added"
            | "response.output_item.done" => "tool_call_item",
            "response.function_call_arguments.delta" => "tool_arguments_delta",
            "response.function_call_arguments.done" => "tool_arguments_done",
            "response.usage" => "usage",
            _ => "unknown",
        };
        let content_bytes = ["delta", "text", "arguments"]
            .iter()
            .find_map(|field| json.get(*field).and_then(Value::as_str))
            .map_or(0, str::len);
        tracing::debug!(
            event_kind,
            top_level_field_count = json.as_object().map_or(0, |object| object.len()),
            content_bytes,
            "Responses API event received"
        );

        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    if !delta.is_empty() {
                        self.record_response_text_delta(delta)?;
                        return Ok(SseEvent::TextDelta(delta.to_string()));
                    }
                }
            }
            "response.output_text.done" => {
                if let Some(text) = json.get("text").and_then(Value::as_str) {
                    if let Some(delta) = self.reconcile_response_text_snapshot(text)? {
                        return Ok(SseEvent::TextDelta(delta));
                    }
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    if !delta.is_empty() {
                        tracing::debug!(delta_bytes = delta.len(), "Reasoning delta received");
                        return Ok(SseEvent::ThinkingDelta {
                            index: 0,
                            thinking: delta.to_string(),
                        });
                    }
                }
            }
            "response.reasoning_summary_part.added" => {
                tracing::info!("Reasoning block started");
                return Ok(SseEvent::ThinkingStart { index: 0 });
            }
            "response.reasoning_summary_text.done"
            | "response.reasoning_text.done"
            | "response.reasoning_summary_part.done" => {
                if !self.mark_reasoning_complete(Self::reasoning_completion_key(json))? {
                    tracing::debug!("Skipping duplicate reasoning completion event");
                    return Ok(SseEvent::Skip);
                }
                tracing::info!("Reasoning block complete");
                return Ok(SseEvent::ThinkingComplete {
                    index: 0,
                    thinking: String::new(),
                    signature: String::new(),
                });
            }
            "response.done" | "response.completed" => {
                let usage = json
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .and_then(parse_openai_responses_usage);

                if let Some(usage) = &usage {
                    tracing::info!(
                        "Responses API usage: input={}, output={}, cached={}",
                        usage.prompt_tokens,
                        usage.completion_tokens,
                        usage.cache_read_input_tokens
                    );
                }

                let tool_calls = self.drain_tool_calls()?;
                if !tool_calls.is_empty() {
                    tracing::info!(
                        "Responses API completing with {} tool calls",
                        tool_calls.len()
                    );
                    return Ok(SseEvent::FinishWithToolCalls { tool_calls, usage });
                }

                return Ok(SseEvent::Finish {
                    reason: Self::responses_finish_reason(json.get("response")),
                    usage,
                });
            }
            "response.function_call_arguments.start"
            | "response.output_item.added"
            | "response.output_item.done" => {
                if let Some(item) = json.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("web_search_call") {
                        let id = item
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("web_search")
                            .to_string();

                        return if event_type == "response.output_item.done" {
                            Ok(SseEvent::ServerToolComplete {
                                id,
                                name: "web_search".to_string(),
                                input: item.get("action").cloned().unwrap_or(Value::Null),
                            })
                        } else {
                            Ok(SseEvent::ServerToolStart {
                                id,
                                name: "web_search".to_string(),
                            })
                        };
                    }

                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        let call_id = item
                            .get("call_id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let item_id = item.get("id").and_then(|i| i.as_str()).map(str::to_string);
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();

                        let key = if !call_id.is_empty() {
                            call_id
                        } else if let Some(item_id) = item_id.as_deref().filter(|id| !id.is_empty())
                        {
                            format!("item:{}", item_id)
                        } else {
                            format!("tool-{}", self.lock_tool_order()?.len())
                        };

                        let tool_id = key.clone();

                        if !name.is_empty() {
                            let inserted =
                                self.register_tool_call(key.clone(), &tool_id, &name, item_id)?;

                            if let Some(arguments) = item.get("arguments").and_then(|a| a.as_str())
                            {
                                let _ = self.apply_tool_arguments_snapshot(&key, arguments)?;
                            }

                            tracing::info!(
                                tool_id_bytes = tool_id.len(),
                                tool_name_bytes = name.len(),
                                "Responses API tool call started"
                            );
                            if inserted {
                                return Ok(SseEvent::ToolCallStart { id: tool_id, name });
                            }
                        }
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    if let Some(key) = self.resolve_responses_tool_key(json)? {
                        if let Some(id) = self.append_tool_arguments(&key, delta)? {
                            return Ok(SseEvent::ToolCallDelta {
                                id,
                                delta: delta.to_string(),
                            });
                        }
                    }
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(arguments) = json.get("arguments").and_then(|a| a.as_str()) {
                    if let Some(key) = self.resolve_responses_tool_key(json)? {
                        if let Some(id) = self.apply_tool_arguments_snapshot(&key, arguments)? {
                            tracing::debug!(
                                tool_id_bytes = id.len(),
                                argument_bytes = arguments.len(),
                                "Tool call arguments complete"
                            );
                        }
                    }
                }
            }
            "response.usage" => {
                let usage_obj = json.get("usage").unwrap_or(json);

                if let Some(usage) = parse_openai_responses_usage(usage_obj) {
                    tracing::info!(
                        "Responses API usage: input={}, output={}, cached={}",
                        usage.prompt_tokens,
                        usage.completion_tokens,
                        usage.cache_read_input_tokens
                    );
                    return Ok(SseEvent::Usage(usage));
                }
            }
            _ => {
                tracing::trace!(
                    event_type_bytes = event_type.len(),
                    "Skipping unknown Responses API event"
                );
            }
        }

        Ok(SseEvent::Skip)
    }
}
