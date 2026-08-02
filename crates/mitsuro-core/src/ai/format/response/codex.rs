use serde_json::Value;

/// Convert ChatGPT Codex response format to Anthropic format
///
/// Codex responses have an `output` array containing items like:
/// - `{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "..."}]}`
/// - `{"type": "function_call", "call_id": "...", "name": "...", "arguments": "..."}`
pub fn normalize_codex_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut stop_reason = "end_turn";

    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match item_type {
                "message" => {
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
                    stop_reason = "tool_use";
                }
                _ => {}
            }
        }
    }

    if let Some(status) = response.get("status").and_then(|s| s.as_str()) {
        match status {
            "completed" => stop_reason = "end_turn",
            "incomplete" => {
                if let Some(reason) = response
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason").and_then(|r| r.as_str()))
                {
                    if reason == "max_output_tokens" {
                        stop_reason = "max_tokens";
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
