use anyhow::Result;
use serde_json::Value;
use std::time::Instant;
use tracing::{error, info};

use super::super::config::{CallOptions, CodexReasoningEffort};
use super::super::core::AiClient;
use crate::ai::format::response::{extract_text_from_content, normalize_openai_response};
use crate::ai::models::ApiFormat;
use crate::ai::transform::apply_request_body_transform;

fn build_openai_tool_request_body(
    model: &str,
    api_format: ApiFormat,
    max_tokens: usize,
    system_prompt: Option<&str>,
    messages: Vec<Value>,
    tools: Vec<Value>,
    reasoning_effort: Option<&str>,
    service_tier: Option<&str>,
) -> Value {
    let responses_format = matches!(api_format, ApiFormat::OpenAIResponses);
    let messages_key = if responses_format {
        "input"
    } else {
        "messages"
    };
    let max_tokens_key = if responses_format {
        "max_output_tokens"
    } else {
        "max_tokens"
    };

    let mut request_messages = Vec::new();
    if let Some(prompt) = system_prompt.filter(|prompt| !prompt.is_empty()) {
        request_messages.push(serde_json::json!({
            "role": "system",
            "content": prompt
        }));
    }
    for message in messages {
        request_messages.extend(convert_openai_tool_message_for_request(message, api_format));
    }

    let mut body = serde_json::json!({
        "model": model,
    });
    body[max_tokens_key] = serde_json::json!(max_tokens);
    body[messages_key] = serde_json::json!(request_messages);

    let openai_tools: Vec<Value> = tools
        .iter()
        .map(|tool| openai_tool_definition(tool, api_format))
        .collect();
    if !openai_tools.is_empty() {
        body["tools"] = serde_json::json!(openai_tools);
    }

    if let Some(effort) = reasoning_effort {
        if responses_format {
            body["reasoning"] = serde_json::json!({ "effort": effort });
        } else {
            body["reasoning_effort"] = serde_json::json!(effort);
        }
    }

    if let Some(tier) = service_tier {
        body["service_tier"] = serde_json::json!(tier);
    }

    body
}

fn openai_tool_definition(tool: &Value, api_format: ApiFormat) -> Value {
    let name = tool
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or("");
    let description = tool
        .get("description")
        .and_then(|description| description.as_str())
        .unwrap_or("");
    let parameters = tool.get("input_schema").cloned().unwrap_or(Value::Null);

    if matches!(api_format, ApiFormat::OpenAIResponses) {
        serde_json::json!({
            "type": "function",
            "name": name,
            "description": description,
            "parameters": parameters
        })
    } else {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters
            }
        })
    }
}

fn convert_openai_tool_message_for_request(message: Value, api_format: ApiFormat) -> Vec<Value> {
    if !matches!(api_format, ApiFormat::OpenAIResponses) {
        return vec![message];
    }

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

impl AiClient {
    pub(super) async fn call_with_tools_openai(
        &self,
        model: &str,
        options: &CallOptions,
        messages: Vec<Value>,
        tools: Vec<Value>,
    ) -> Result<Value> {
        let system_prompt = options.system_prompt.as_deref().unwrap_or_default();
        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);
        let thinking_enabled = options.thinking.is_some();

        // Check if we're using ChatGPT Codex API (OAuth)
        let is_chatgpt_codex = self
            .config()
            .base_url
            .as_ref()
            .map(|url| url.contains("chatgpt.com"))
            .unwrap_or(false);

        if is_chatgpt_codex {
            return self
                .call_with_tools_chatgpt_codex(model, options, messages, tools)
                .await;
        }

        info!(model = model, provider = %self.provider_id(), "Sub-agent OpenAI format API call starting");
        let start = Instant::now();

        // Convert messages from Anthropic to OpenAI format
        let mut openai_messages: Vec<Value> = vec![];

        // Add system message first
        openai_messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));

        // Convert each message
        for msg in &messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg.get("content");

            if role == "assistant" {
                // Check for tool_use in content
                if let Some(content_arr) = content.and_then(|c| c.as_array()) {
                    let has_tool_use = content_arr
                        .iter()
                        .any(|c| c.get("type").and_then(|t| t.as_str()) == Some("tool_use"));

                    if has_tool_use {
                        let mut tool_calls = vec![];
                        let mut text_content = String::new();

                        for item in content_arr {
                            match item.get("type").and_then(|t| t.as_str()) {
                                Some("text") => {
                                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                        text_content.push_str(text);
                                    }
                                }
                                Some("tool_use") => {
                                    let id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
                                    let name =
                                        item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    let input = item.get("input").cloned().unwrap_or(Value::Null);
                                    tool_calls.push(serde_json::json!({
                                        "id": id,
                                        "type": "function",
                                        "function": {
                                            "name": name,
                                            "arguments": input.to_string()
                                        }
                                    }));
                                }
                                _ => {}
                            }
                        }

                        let mut msg_obj = serde_json::json!({"role": "assistant"});
                        if !text_content.is_empty() {
                            msg_obj["content"] = serde_json::json!(text_content);
                        }
                        if !tool_calls.is_empty() {
                            msg_obj["tool_calls"] = serde_json::json!(tool_calls);
                        }
                        openai_messages.push(msg_obj);
                        continue;
                    }
                }

                // Regular assistant message
                let text = extract_text_from_content(content);
                openai_messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": text
                }));
            } else if role == "user" {
                // Check for tool_result in content
                if let Some(content_arr) = content.and_then(|c| c.as_array()) {
                    for item in content_arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            let tool_use_id = item
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            let output = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                            openai_messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": output
                            }));
                        } else if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                openai_messages.push(serde_json::json!({
                                    "role": "user",
                                    "content": text
                                }));
                            }
                        }
                    }
                    continue;
                }

                // Simple user message
                let text = extract_text_from_content(content);
                openai_messages.push(serde_json::json!({
                    "role": "user",
                    "content": text
                }));
            }
        }

        let effort = if thinking_enabled {
            Some(
                options
                    .codex_reasoning_effort
                    .unwrap_or(CodexReasoningEffort::High)
                    .normalized_for_model(model)
                    .as_str()
                    .to_string(),
            )
        } else {
            None
        };

        let body = build_openai_tool_request_body(
            model,
            self.config().api_format,
            max_tokens,
            None,
            openai_messages,
            tools,
            effort.as_deref(),
            options.service_tier_for_provider(self.provider_id()),
        );

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());
        let response = match request.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent OpenAI API request failed");
                return Err(anyhow::anyhow!("API request failed: {}", e));
            }
        };

        let status = response.status();
        info!(status = %status, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent OpenAI API response received");

        let response = self.handle_error_response(response).await?;
        let json: Value = response.json().await?;

        // Convert OpenAI response to Anthropic format for consistent parsing
        let anthropic_response = normalize_openai_response(&json);

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent OpenAI API call complete"
        );
        Ok(anthropic_response)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::build_openai_tool_request_body;
    use crate::ai::models::ApiFormat;

    #[test]
    fn responses_tool_request_uses_responses_shape() {
        let body = build_openai_tool_request_body(
            "gpt-5.5",
            ApiFormat::OpenAIResponses,
            4096,
            Some("System"),
            vec![json!({"role": "user", "content": [{"type": "text", "text": "Use a tool"}]})],
            vec![
                json!({"name": "read", "description": "Read a file", "input_schema": {"type": "object"}}),
            ],
            Some("xhigh"),
            Some("priority"),
        );

        assert!(body.get("messages").is_none());
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["reasoning"]["effort"], "xhigh");
        assert_eq!(body["service_tier"], "priority");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "read");
        assert!(body["tools"][0].get("function").is_none());
    }
}
