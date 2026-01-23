//! Response normalization
//!
//! Converts responses from different API formats to a unified Anthropic-style format
//! for consistent downstream processing.

use serde_json::Value;

/// Convert OpenAI response format to Anthropic format
pub fn normalize_openai_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut stop_reason = "end_turn".to_string();

    // Get the first choice
    if let Some(choices) = response.get("choices").and_then(|c| c.as_array()) {
        if let Some(choice) = choices.first() {
            // Check finish reason
            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                stop_reason = match reason {
                    "tool_calls" => "tool_use".to_string(),
                    "stop" => "end_turn".to_string(),
                    "length" => "max_tokens".to_string(),
                    _ => reason.to_string(),
                };
            }

            if let Some(message) = choice.get("message") {
                // Extract text content
                if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
                    if !text.is_empty() {
                        content.push(serde_json::json!({
                            "type": "text",
                            "text": text
                        }));
                    }
                }

                // Extract tool calls
                if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        if let Some(function) = tc.get("function") {
                            let name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let args_str = function
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .unwrap_or("{}");
                            let input: Value =
                                serde_json::from_str(args_str).unwrap_or(Value::Null);

                            content.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input
                            }));
                        }
                    }
                }
            }
        }
    }

    serde_json::json!({
        "content": content,
        "stop_reason": stop_reason,
        "model": response.get("model").cloned().unwrap_or(Value::Null)
    })
}

/// Convert Google response format to Anthropic format
pub fn normalize_google_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut stop_reason = "end_turn".to_string();

    // Get the first candidate
    if let Some(candidates) = response.get("candidates").and_then(|c| c.as_array()) {
        if let Some(candidate) = candidates.first() {
            // Check finish reason
            if let Some(reason) = candidate.get("finishReason").and_then(|r| r.as_str()) {
                stop_reason = match reason {
                    "STOP" => "end_turn".to_string(),
                    "MAX_TOKENS" => "max_tokens".to_string(),
                    "SAFETY" => "stop_sequence".to_string(),
                    _ => reason.to_lowercase(),
                };
            }

            // Extract parts from content
            if let Some(content_obj) = candidate.get("content") {
                if let Some(parts) = content_obj.get("parts").and_then(|p| p.as_array()) {
                    for part in parts {
                        // Text content
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            content.push(serde_json::json!({
                                "type": "text",
                                "text": text
                            }));
                        }

                        // Function call
                        if let Some(fc) = part.get("functionCall") {
                            let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let args = fc.get("args").cloned().unwrap_or(Value::Null);
                            // Generate a unique ID for the tool use
                            let id = format!(
                                "toolu_{}",
                                &uuid::Uuid::new_v4().to_string().replace('-', "")[..24]
                            );

                            content.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": args
                            }));
                            stop_reason = "tool_use".to_string();
                        }
                    }
                }
            }
        }
    }

    serde_json::json!({
        "content": content,
        "stop_reason": stop_reason,
        "model": response.get("modelVersion").cloned().unwrap_or(Value::Null)
    })
}

/// Extract text content from an Anthropic-format content array
pub fn extract_text_from_content(content: Option<&Value>) -> String {
    content
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                        item.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Convert ChatGPT Codex response format to Anthropic format
///
/// Codex responses have an `output` array containing items like:
/// - `{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "..."}]}`
/// - `{"type": "function_call", "call_id": "...", "name": "...", "arguments": "..."}`
pub fn normalize_codex_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut stop_reason = "end_turn".to_string();

    // Codex uses "output" array instead of "choices"
    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match item_type {
                "message" => {
                    // Extract text from message content
                    if let Some(msg_content) = item.get("content").and_then(|c| c.as_array()) {
                        for part in msg_content {
                            let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if part_type == "output_text" {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        content.push(serde_json::json!({
                                            "type": "text",
                                            "text": text
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    // Convert function call to tool_use
                    let call_id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let args_str = item
                        .get("arguments")
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);

                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call_id,
                        "name": name,
                        "input": input
                    }));
                    stop_reason = "tool_use".to_string();
                }
                _ => {}
            }
        }
    }

    // Check status for stop reason
    if let Some(status) = response.get("status").and_then(|s| s.as_str()) {
        match status {
            "completed" => stop_reason = "end_turn".to_string(),
            "incomplete" => {
                if let Some(reason) = response
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason").and_then(|r| r.as_str()))
                {
                    if reason == "max_output_tokens" {
                        stop_reason = "max_tokens".to_string();
                    }
                }
            }
            _ => {}
        }
    }

    serde_json::json!({
        "content": content,
        "stop_reason": stop_reason,
        "model": response.get("model").cloned().unwrap_or(Value::Null)
    })
}
