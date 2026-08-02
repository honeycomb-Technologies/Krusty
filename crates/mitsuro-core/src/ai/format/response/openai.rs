use serde_json::Value;

/// Convert OpenAI response format to Anthropic format
pub fn normalize_openai_response(response: &Value) -> Value {
    if response.get("choices").is_some() {
        return normalize_chat_completions_response(response);
    }

    if response.get("output").is_some() {
        return normalize_responses_api_response(response);
    }

    serde_json::json!({
        "content": [],
        "stop_reason": "end_turn",
        "model": response.get("model").cloned().unwrap_or(Value::Null)
    })
}

fn normalize_chat_completions_response(response: &Value) -> Value {
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
        "model": response.get("model").cloned().unwrap_or(Value::Null),
        "usage": normalized_usage(response)
    })
}

fn normalize_responses_api_response(response: &Value) -> Value {
    let mut content: Vec<Value> = vec![];
    let mut has_tool_calls = false;

    if let Some(output) = response.get("output").and_then(|value| value.as_array()) {
        for item in output {
            match item.get("type").and_then(|value| value.as_str()) {
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(|value| value.as_array()) {
                        for part in parts {
                            let Some(part_type) = part.get("type").and_then(|value| value.as_str())
                            else {
                                continue;
                            };
                            if matches!(part_type, "output_text" | "text") {
                                if let Some(text) =
                                    part.get("text").and_then(|value| value.as_str())
                                {
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
                Some("output_text") | Some("text") => {
                    if let Some(text) = item.get("text").and_then(|value| value.as_str()) {
                        if !text.is_empty() {
                            content.push(serde_json::json!({
                                "type": "text",
                                "text": text
                            }));
                        }
                    }
                }
                Some("function_call") => {
                    has_tool_calls = true;
                    let id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let name = item
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let args_str = item
                        .get("arguments")
                        .and_then(|value| value.as_str())
                        .unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);

                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input
                    }));
                }
                _ => {}
            }
        }
    }

    let stop_reason = responses_stop_reason(response, has_tool_calls);

    serde_json::json!({
        "content": content,
        "stop_reason": stop_reason,
        "model": response.get("model").cloned().unwrap_or(Value::Null),
        "usage": normalized_usage(response)
    })
}

fn normalized_usage(response: &Value) -> Value {
    response
        .get("usage")
        .or_else(|| {
            response
                .get("response")
                .and_then(|value| value.get("usage"))
        })
        .cloned()
        .unwrap_or(Value::Null)
}

fn responses_stop_reason(response: &Value, has_tool_calls: bool) -> &'static str {
    match response.get("status").and_then(|value| value.as_str()) {
        Some("incomplete") => match response
            .get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(|reason| reason.as_str())
        {
            Some("max_output_tokens" | "max_tokens" | "length") => "max_tokens",
            Some("tool_calls" | "tool_use") => "tool_use",
            _ if has_tool_calls => "tool_use",
            _ => "end_turn",
        },
        _ if has_tool_calls => "tool_use",
        _ => "end_turn",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::normalize_openai_response;

    #[test]
    fn normalizes_responses_api_text_and_function_calls() {
        let response = json!({
            "model": "gpt-5.5",
            "status": "completed",
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 50,
                "input_tokens_details": {"cached_tokens": 700}
            },
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "Need to inspect files."}
                    ]
                },
                {
                    "type": "function_call",
                    "call_id": "call_read",
                    "name": "read",
                    "arguments": "{\"path\":\"Cargo.toml\"}"
                }
            ]
        });

        let normalized = normalize_openai_response(&response);
        assert_eq!(normalized["stop_reason"], "tool_use");
        assert_eq!(normalized["content"][0]["type"], "text");
        assert_eq!(normalized["content"][0]["text"], "Need to inspect files.");
        assert_eq!(normalized["content"][1]["type"], "tool_use");
        assert_eq!(normalized["content"][1]["id"], "call_read");
        assert_eq!(normalized["content"][1]["name"], "read");
        assert_eq!(normalized["content"][1]["input"]["path"], "Cargo.toml");
        assert_eq!(normalized["usage"]["input_tokens"], 1000);
        assert_eq!(normalized["usage"]["output_tokens"], 50);
        assert_eq!(
            normalized["usage"]["input_tokens_details"]["cached_tokens"],
            700
        );
    }
}
