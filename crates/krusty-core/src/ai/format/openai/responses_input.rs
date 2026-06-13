//! OpenAI Responses API input shaping for tool-bearing conversations.

use serde_json::Value;

use crate::ai::models::ApiFormat;

/// Convert chat-shaped messages into Responses API `input` items.
pub fn convert_messages_for_responses_api(messages: Vec<Value>) -> Vec<Value> {
    let mut converted = Vec::new();
    for message in messages {
        converted.extend(convert_openai_tool_message_for_request(message));
    }
    converted
}

/// Convert one chat-shaped message into zero or more Responses API input items.
pub fn convert_openai_tool_message_for_request(message: Value) -> Vec<Value> {
    let role = message.get("role").and_then(|role| role.as_str());
    if role == Some("tool") {
        return vec![serde_json::json!({
            "type": "function_call_output",
            "call_id": message
                .get("tool_call_id")
                .and_then(|call_id| call_id.as_str())
                .unwrap_or(""),
            "output": message
                .get("content")
                .and_then(|content| content.as_str())
                .unwrap_or("")
        })];
    }

    if role == Some("assistant") {
        if let Some(tool_calls) = message.get("tool_calls").and_then(|calls| calls.as_array()) {
            let mut converted = Vec::new();
            if let Some(text) = message.get("content").and_then(|content| content.as_str()) {
                if !text.is_empty() {
                    converted.push(serde_json::json!({
                        "role": "assistant",
                        "content": text
                    }));
                }
            }
            for tool_call in tool_calls {
                if let Some(function) = tool_call.get("function") {
                    converted.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tool_call
                            .get("id")
                            .and_then(|id| id.as_str())
                            .unwrap_or(""),
                        "name": function
                            .get("name")
                            .and_then(|name| name.as_str())
                            .unwrap_or(""),
                        "arguments": function
                            .get("arguments")
                            .and_then(|arguments| arguments.as_str())
                            .unwrap_or("{}")
                    }));
                }
            }
            return converted;
        }
    }

    vec![message]
}

fn extract_message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }
    let parts = content.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            let part_type = part.get("type").and_then(|value| value.as_str())?;
            if matches!(part_type, "text" | "input_text" | "output_text") {
                part.get("text").and_then(|value| value.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn merge_instructions(body: &mut Value, system_parts: &[String]) {
    if system_parts.is_empty() {
        return;
    }

    let mut merged = Vec::new();
    if let Some(existing) = body.get("instructions").and_then(|value| value.as_str()) {
        if !existing.is_empty() {
            merged.push(existing.to_string());
        }
    }
    merged.extend(system_parts.iter().cloned());
    body["instructions"] = Value::String(merged.join("\n\n"));
}

/// Apply Responses API input conversion when the request body uses `input`.
pub fn transform_request_input_for_api_format(
    body: &mut Value,
    api_format: ApiFormat,
    messages_key: &str,
) {
    if !matches!(api_format, ApiFormat::OpenAIResponses) {
        return;
    }
    let Some(messages) = body.get(messages_key).and_then(|value| value.as_array()) else {
        return;
    };
    let converted = convert_messages_for_responses_api(messages.clone());

    let mut input_items = Vec::new();
    let mut system_parts = Vec::new();
    for item in converted {
        if item.get("role").and_then(|role| role.as_str()) == Some("system") {
            if let Some(text) = extract_message_text(&item) {
                system_parts.push(text);
            }
            continue;
        }
        input_items.push(item);
    }

    merge_instructions(body, &system_parts);
    body[messages_key] = Value::Array(input_items);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::ApiFormat;
    use serde_json::json;

    #[test]
    fn converts_tool_messages_to_function_call_output() {
        let converted = convert_messages_for_responses_api(vec![json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "content": "done"
        })]);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call_output");
        assert_eq!(converted[0]["call_id"], "call_123");
        assert_eq!(converted[0]["output"], "done");
    }

    #[test]
    fn extracts_system_messages_into_instructions() {
        let mut body = json!({
            "model": "grok-build",
            "input": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "hello" }
            ]
        });

        transform_request_input_for_api_format(&mut body, ApiFormat::OpenAIResponses, "input");

        assert_eq!(body["instructions"], "You are helpful.");
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert_eq!(body["input"][0]["role"], "user");
    }

    #[test]
    fn converts_assistant_tool_calls_to_function_call_items() {
        let converted = convert_messages_for_responses_api(vec![json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": "call_abc",
                "type": "function",
                "function": {
                    "name": "search_compaction_segments",
                    "arguments": "{}"
                }
            }]
        })]);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function_call");
        assert_eq!(converted[0]["name"], "search_compaction_segments");
    }
}
