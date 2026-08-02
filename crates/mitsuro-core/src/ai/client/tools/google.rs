use anyhow::Result;
use serde_json::Value;
use std::time::Instant;
use tracing::{error, info};

use super::super::config::CallOptions;
use super::super::core::AiClient;
use crate::ai::format::response::normalize_google_response;
use crate::ai::transform::apply_request_body_transform;

impl AiClient {
    pub(super) async fn call_with_tools_google(
        &self,
        model: &str,
        options: &CallOptions,
        messages: Vec<Value>,
        tools: Vec<Value>,
    ) -> Result<Value> {
        let system_prompt = options.system_prompt.as_deref().unwrap_or_default();
        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);
        info!(model = model, provider = %self.provider_id(), "Sub-agent Google format API call starting");
        let start = Instant::now();

        // Convert messages from Anthropic to Google contents format
        let mut contents: Vec<Value> = vec![];

        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg.get("content");

            let google_role = match role {
                "assistant" => "model",
                _ => "user",
            };

            let mut parts: Vec<Value> = vec![];

            if let Some(content_arr) = content.and_then(|c| c.as_array()) {
                for item in content_arr {
                    match item.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                parts.push(serde_json::json!({"text": text}));
                            }
                        }
                        Some("tool_use") => {
                            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let input = item.get("input").cloned().unwrap_or(Value::Null);
                            parts.push(serde_json::json!({
                                "functionCall": {
                                    "name": name,
                                    "args": input
                                }
                            }));
                        }
                        Some("tool_result") => {
                            let tool_use_id = item
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            let output = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                            parts.push(serde_json::json!({
                                "functionResponse": {
                                    "name": tool_use_id,
                                    "response": {
                                        "content": output
                                    }
                                }
                            }));
                        }
                        _ => {}
                    }
                }
            }

            if !parts.is_empty() {
                contents.push(serde_json::json!({
                    "role": google_role,
                    "parts": parts
                }));
            }
        }

        // Convert tools to Google function declarations format
        let google_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                    "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "parameters": t.get("input_schema").cloned().unwrap_or(Value::Null)
                })
            })
            .collect();

        // Build request body
        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": max_tokens,
            }
        });

        // Add system instruction
        body["systemInstruction"] = serde_json::json!({
            "parts": [{"text": system_prompt}]
        });

        // Add tools if present
        if !google_tools.is_empty() {
            body["tools"] = serde_json::json!([{
                "functionDeclarations": google_tools
            }]);
        }

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());
        let response = match request.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent Google API request failed");
                return Err(anyhow::anyhow!("API request failed: {}", e));
            }
        };

        let status = response.status();
        info!(status = %status, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent Google API response received");

        let response = self.handle_error_response(response).await?;
        let json: Value = response.json().await?;

        // Convert Google response to Anthropic format for consistent parsing
        let anthropic_response = normalize_google_response(&json);

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent Google API call complete"
        );
        Ok(anthropic_response)
    }
}
