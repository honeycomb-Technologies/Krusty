use serde_json::Value;

/// Convert Google response format to Anthropic format
pub fn normalize_google_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut stop_reason: Option<&str> = Some("end_turn");
    let mut stop_reason_owned: Option<String> = None;

    if let Some(candidates) = response.get("candidates").and_then(|c| c.as_array()) {
        if let Some(candidate) = candidates.first() {
            if let Some(reason) = candidate.get("finishReason").and_then(|r| r.as_str()) {
                stop_reason = match reason {
                    "STOP" => Some("end_turn"),
                    "MAX_TOKENS" => Some("max_tokens"),
                    "SAFETY" => Some("stop_sequence"),
                    _ => {
                        stop_reason_owned = Some(reason.to_lowercase());
                        None
                    }
                };
            }

            if let Some(content_obj) = candidate.get("content") {
                if let Some(parts) = content_obj.get("parts").and_then(|p| p.as_array()) {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            content.push(serde_json::json!({
                                "type": "text",
                                "text": text
                            }));
                        }

                        if let Some(fc) = part.get("functionCall") {
                            let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let args = fc.get("args").cloned().unwrap_or(Value::Null);
                            let uuid = uuid::Uuid::new_v4().simple().to_string();
                            let id = format!("toolu_{}", &uuid[..24]);

                            content.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": args
                            }));
                            stop_reason = Some("tool_use");
                            stop_reason_owned = None;
                        }
                    }
                }
            }
        }
    }

    serde_json::json!({
        "content": content,
        "stop_reason": stop_reason.unwrap_or_else(|| stop_reason_owned.as_deref().unwrap_or("end_turn")),
        "model": response.get("modelVersion").cloned().unwrap_or(Value::Null)
    })
}
