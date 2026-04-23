use serde_json::Value;

/// Convert OpenAI response format to Anthropic format
pub fn normalize_openai_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut stop_reason = "end_turn";

    if let Some(choices) = response.get("choices").and_then(|c| c.as_array()) {
        if let Some(choice) = choices.first() {
            if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                stop_reason = match reason {
                    "tool_calls" => "tool_use",
                    "stop" => "end_turn",
                    "length" => "max_tokens",
                    _ => reason,
                };
            }

            if let Some(message) = choice.get("message") {
                if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
                    if !text.is_empty() {
                        content.push(serde_json::json!({
                            "type": "text",
                            "text": text
                        }));
                    }
                }

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
