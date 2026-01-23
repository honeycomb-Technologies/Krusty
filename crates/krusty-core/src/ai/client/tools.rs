//! Tool-calling API methods
//!
//! Non-streaming calls with tool support, used by sub-agents.

use anyhow::Result;
use serde_json::Value;
use std::time::Instant;
use tracing::{error, info};

use super::core::AiClient;
use crate::ai::format::response::{
    extract_text_from_content, normalize_codex_response, normalize_google_response,
    normalize_openai_response,
};

impl AiClient {
    /// Call the API with tools (non-streaming, for sub-agents)
    ///
    /// Used by sub-agents that need tool execution but don't need streaming.
    /// Routes to appropriate format handler based on API format.
    pub async fn call_with_tools(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        max_tokens: usize,
    ) -> Result<Value> {
        // Route to appropriate format handler based on API format
        if self.config().uses_openai_format() {
            return self
                .call_with_tools_openai(model, system_prompt, messages, tools, max_tokens)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_with_tools_google(model, system_prompt, messages, tools, max_tokens)
                .await;
        }

        // Anthropic format (default)
        self.call_with_tools_anthropic(model, system_prompt, messages, tools, max_tokens)
            .await
    }

    /// Call with tools using Anthropic format
    async fn call_with_tools_anthropic(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        max_tokens: usize,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
            "system": system_prompt,
            "tools": tools
        });

        // Add thinking beta for providers that support it
        let beta_headers = vec!["interleaved-thinking-2025-05-14"];
        let request = self.build_request_with_beta(&self.config().api_url(), &beta_headers);

        info!(model = model, provider = %self.provider_id(), "Sub-agent API call starting");
        let start = Instant::now();

        let response = match request.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent API request failed");
                return Err(anyhow::anyhow!("API request failed: {}", e));
            }
        };

        let status = response.status();
        info!(status = %status, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent API response received");

        let response = self.handle_error_response(response).await?;
        let json: Value = response.json().await?;

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent API call complete"
        );
        Ok(json)
    }

    /// Call with tools using OpenAI format (non-streaming)
    ///
    /// Converts Anthropic-format messages/tools to OpenAI format and returns
    /// a normalized Anthropic-format response for consistent parsing.
    ///
    /// Handles both standard OpenAI API and ChatGPT Codex API (which has different format).
    async fn call_with_tools_openai(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        max_tokens: usize,
    ) -> Result<Value> {
        // Check if we're using ChatGPT Codex API (OAuth)
        let is_chatgpt_codex = self
            .config()
            .base_url
            .as_ref()
            .map(|url| url.contains("chatgpt.com"))
            .unwrap_or(false);

        if is_chatgpt_codex {
            return self
                .call_with_tools_chatgpt_codex(model, system_prompt, messages, tools)
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
        for msg in messages {
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

        // Convert tools from Anthropic to OpenAI format
        let openai_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                        "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "parameters": t.get("input_schema").cloned().unwrap_or(Value::Null)
                    }
                })
            })
            .collect();

        // Build request body
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": openai_messages,
        });

        if !openai_tools.is_empty() {
            body["tools"] = serde_json::json!(openai_tools);
        }

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

    /// Call with tools using ChatGPT Codex format (non-streaming)
    ///
    /// ChatGPT Codex API has a completely different format than standard OpenAI:
    /// - Uses "instructions" field instead of system message
    /// - Uses "input" instead of "messages"
    /// - Messages wrapped in {"type": "message", ...}
    /// - No max_tokens parameter
    /// - Requires store=false
    async fn call_with_tools_chatgpt_codex(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
    ) -> Result<Value> {
        info!(model = model, provider = %self.provider_id(), "Sub-agent ChatGPT Codex API call starting");
        let start = Instant::now();

        // Convert messages from Anthropic to Codex format
        let mut codex_input: Vec<Value> = vec![];

        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg.get("content");

            if role == "assistant" {
                // Check for tool_use in content
                if let Some(content_arr) = content.and_then(|c| c.as_array()) {
                    let has_tool_use = content_arr
                        .iter()
                        .any(|c| c.get("type").and_then(|t| t.as_str()) == Some("tool_use"));

                    if has_tool_use {
                        // Add text content first if any
                        let text_content: String = content_arr
                            .iter()
                            .filter_map(|item| {
                                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    item.get("text")
                                        .and_then(|t| t.as_str())
                                        .map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        if !text_content.is_empty() {
                            codex_input.push(serde_json::json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{
                                    "type": "output_text",
                                    "text": text_content
                                }]
                            }));
                        }

                        // Add each tool call
                        for item in content_arr {
                            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                let id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
                                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                let input = item.get("input").cloned().unwrap_or(Value::Null);
                                codex_input.push(serde_json::json!({
                                    "type": "function_call",
                                    "call_id": id,
                                    "name": name,
                                    "arguments": input.to_string()
                                }));
                            }
                        }
                        continue;
                    }
                }

                // Regular assistant message
                let text = extract_text_from_content(content);
                if !text.is_empty() {
                    codex_input.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": text
                        }]
                    }));
                }
            } else if role == "user" {
                // Check for tool_result in content
                if let Some(content_arr) = content.and_then(|c| c.as_array()) {
                    let has_tool_result = content_arr
                        .iter()
                        .any(|c| c.get("type").and_then(|t| t.as_str()) == Some("tool_result"));

                    if has_tool_result {
                        for item in content_arr {
                            if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                                let tool_use_id = item
                                    .get("tool_use_id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("");
                                let output =
                                    item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                codex_input.push(serde_json::json!({
                                    "type": "function_call_output",
                                    "call_id": tool_use_id,
                                    "output": output
                                }));
                            }
                        }
                        continue;
                    }
                }

                // Simple user message
                let text = extract_text_from_content(content);
                if !text.is_empty() {
                    codex_input.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": [{
                            "type": "input_text",
                            "text": text
                        }]
                    }));
                }
            }
        }

        // Convert tools from Anthropic to Codex format (flat structure)
        let codex_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "name": t.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                    "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "parameters": t.get("input_schema").cloned().unwrap_or(Value::Null)
                })
            })
            .collect();

        // Generate cache key
        let cache_key = uuid::Uuid::new_v4().to_string();

        // Build Codex request body
        let mut body = serde_json::json!({
            "model": model,
            "instructions": system_prompt,
            "input": codex_input,
            "tools": codex_tools,
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "reasoning": {
                "summary": "auto"
            },
            "store": false,
            "stream": false,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": cache_key
        });

        // Remove tools if empty
        if codex_tools.is_empty() {
            body.as_object_mut().unwrap().remove("tools");
            body.as_object_mut().unwrap().remove("tool_choice");
        }

        let request = self.build_request(&self.config().api_url());
        let response = match request.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent Codex API request failed");
                return Err(anyhow::anyhow!("API request failed: {}", e));
            }
        };

        let status = response.status();
        info!(status = %status, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent Codex API response received");

        let response = self.handle_error_response(response).await?;
        let json: Value = response.json().await?;

        // Convert Codex response to Anthropic format for consistent parsing
        let anthropic_response = normalize_codex_response(&json);

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent Codex API call complete"
        );
        Ok(anthropic_response)
    }

    /// Call with tools using Google format (non-streaming)
    ///
    /// Converts Anthropic-format messages/tools to Google format and returns
    /// a normalized Anthropic-format response for consistent parsing.
    async fn call_with_tools_google(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        max_tokens: usize,
    ) -> Result<Value> {
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
