//! Tool-calling API methods
//!
//! Non-streaming calls with tool support, used by sub-agents.

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info};
use url::Url;

use super::core::AiClient;
use crate::ai::format::response::{
    extract_text_from_content, normalize_google_response, normalize_openai_response,
};

const OPENAI_WS_API_VERSION: &str = "responses_websockets=2026-02-06";

fn collect_text_content_with_separator(content_arr: &[Value], separator: &str) -> String {
    let mut text_content = String::new();

    for item in content_arr {
        if item.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        let Some(text) = item.get("text").and_then(|t| t.as_str()) else {
            continue;
        };

        if !text_content.is_empty() {
            text_content.push_str(separator);
        }
        text_content.push_str(text);
    }

    text_content
}

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
        thinking_enabled: bool,
    ) -> Result<Value> {
        // Route to appropriate format handler based on API format
        if self.config().uses_openai_format() {
            return self
                .call_with_tools_openai(
                    model,
                    system_prompt,
                    messages,
                    tools,
                    max_tokens,
                    thinking_enabled,
                )
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_with_tools_google(model, system_prompt, messages, tools, max_tokens)
                .await;
        }

        // Anthropic format (default)
        self.call_with_tools_anthropic(
            model,
            system_prompt,
            messages,
            tools,
            max_tokens,
            thinking_enabled,
        )
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
        thinking_enabled: bool,
    ) -> Result<Value> {
        // Sort tools deterministically to maintain stable cache prefix.
        // Tool order is part of the cached prefix; non-deterministic order breaks caching.
        let mut sorted_tools = tools;
        sorted_tools.sort_by(|a, b| {
            let name_a = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let name_b = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
            name_a.cmp(name_b)
        });

        // Only apply cache_control for providers that support prompt caching.
        // MiniMax, Z.ai, etc. use Anthropic format but don't support caching —
        // sending cache_control or array-format system prompts may cause errors.
        let capabilities =
            crate::ai::providers::ProviderCapabilities::for_provider(self.provider_id());
        let enable_caching = capabilities.prompt_caching;

        let system_value: Value = if enable_caching {
            serde_json::json!([{
                "type": "text",
                "text": system_prompt,
                "cache_control": {"type": "ephemeral"}
            }])
        } else {
            Value::String(system_prompt.to_string())
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
            "system": system_value,
            "tools": sorted_tools
        });

        // Enable auto-caching at the request level. The API automatically places
        // the cache breakpoint on the last cacheable block, replacing the need for
        // manual breakpoints on the last tool and last message.
        if enable_caching {
            body["cache_control"] = serde_json::json!({"type": "ephemeral"});
        }

        // Add thinking configuration when enabled
        // MiniMax: Simple thinking without budget_tokens (their API doesn't support it)
        // Z.ai/others: No thinking support for sub-agents
        if thinking_enabled {
            let provider = self.provider_id();
            if provider == crate::ai::providers::ProviderId::MiniMax {
                // MiniMax uses Anthropic-compatible thinking but without budget_tokens
                body["thinking"] = serde_json::json!({
                    "type": "enabled"
                });
            }
        }

        let request = self.build_request(&self.config().api_url());

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
        thinking_enabled: bool,
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
                .call_with_tools_chatgpt_codex(
                    model,
                    system_prompt,
                    messages,
                    tools,
                    thinking_enabled,
                )
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

        // Add reasoning effort when thinking is enabled (high = maximum for OpenAI API)
        if thinking_enabled {
            body["reasoning_effort"] = serde_json::json!("high");
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

    /// Call with tools using ChatGPT Codex format (streaming required)
    ///
    /// ChatGPT Codex API has a completely different format than standard OpenAI:
    /// - Uses "instructions" field instead of system message
    /// - Uses "input" instead of "messages"
    /// - Messages wrapped in {"type": "message", ...}
    /// - No max_tokens parameter
    /// - Requires store=false
    /// - REQUIRES stream=true (even for "non-streaming" calls)
    ///
    /// We collect the streaming response and return the final result.
    async fn call_with_tools_chatgpt_codex(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        thinking_enabled: bool,
    ) -> Result<Value> {
        info!(model = model, provider = %self.provider_id(), "Sub-agent ChatGPT Codex API call starting (streaming)");
        let start = Instant::now();

        // Convert messages from Anthropic to Codex format
        let mut codex_input: Vec<Value> = vec![];

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
                        // Add text content first if any
                        let text_content = collect_text_content_with_separator(content_arr, "\n");

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

        let mut body = serde_json::json!({
            "model": model,
            "instructions": system_prompt,
            "input": codex_input,
            "tools": codex_tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
            "text": {
                "verbosity": "medium"
            }
        });

        if thinking_enabled {
            body["reasoning"] = serde_json::json!({
                "effort": "medium",
                "summary": "auto"
            });
            body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
        }

        if codex_tools.is_empty() {
            if let Some(obj) = body.as_object_mut() {
                obj.remove("tools");
                obj.remove("tool_choice");
            }
        }

        let ws_url = Self::resolve_codex_ws_url_for_tools(&self.config().api_url())?;
        let request = self.build_websocket_request(
            ws_url.as_str(),
            &[
                ("OpenAI-Beta", OPENAI_WS_API_VERSION),
                ("originator", "krusty"),
            ],
        )?;

        info!("Connecting sub-agent Codex websocket: {}", ws_url);
        let (mut ws_stream, _) = connect_async(request).await.map_err(|e| {
            anyhow::anyhow!(
                "Sub-agent Codex websocket connection failed (websocket-only mode): {}",
                e
            )
        })?;

        let create_payload = Self::codex_ws_create_payload(body);
        ws_stream
            .send(Message::Text(create_payload.to_string()))
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Sub-agent Codex websocket request send failed (websocket-only mode): {}",
                    e
                )
            })?;

        let collected_response = self
            .collect_codex_websocket_response(&mut ws_stream, model)
            .await?;

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent Codex API call complete"
        );
        Ok(collected_response)
    }

    /// Collect a websocket Codex response into a single Anthropic-format response.
    async fn collect_codex_websocket_response<S>(
        &self,
        stream: &mut S,
        model: &str,
    ) -> Result<Value>
    where
        S: futures::Stream<
                Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
            > + Unpin,
    {
        let mut text_content = String::new();
        let mut tool_order: Vec<String> = Vec::new();
        let mut pending_tools: HashMap<String, (String, String)> = HashMap::new();
        let mut item_to_call_id: HashMap<String, String> = HashMap::new();
        let mut saw_completion = false;
        let mut finish_reason = "end_turn";

        while let Some(msg) = stream.next().await {
            let payload = match msg? {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Close(frame) => {
                    if !saw_completion {
                        let (code, reason) = frame
                            .as_ref()
                            .map(|f| (f.code.to_string(), f.reason.to_string()))
                            .unwrap_or_else(|| {
                                ("no close code".to_string(), "no close reason".to_string())
                            });
                        return Err(anyhow::anyhow!(
                            "Sub-agent Codex websocket closed before completion (websocket-only mode): code={}, reason={}",
                            code, reason
                        ));
                    }
                    break;
                }
            };

            let Ok(json) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };

            let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match event_type {
                "error" | "response.failed" => {
                    let message = Self::codex_ws_error_message(&json)
                        .unwrap_or_else(|| "unknown websocket error".to_string());
                    return Err(anyhow::anyhow!(
                        "Sub-agent Codex websocket API error: {}",
                        message
                    ));
                }
                "response.output_text.delta" => {
                    if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                        text_content.push_str(delta);
                    }
                }
                "response.output_item.added" | "response.output_item.done" => {
                    if let Some(item) = json.get("item") {
                        if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
                            continue;
                        }
                        let call_id = item
                            .get("call_id")
                            .and_then(|i| i.as_str())
                            .or_else(|| item.get("id").and_then(|i| i.as_str()))
                            .unwrap_or("")
                            .to_string();
                        if call_id.is_empty() {
                            continue;
                        }
                        if let Some(item_id) = item
                            .get("id")
                            .and_then(|i| i.as_str())
                            .filter(|id| !id.is_empty())
                        {
                            item_to_call_id.insert(item_id.to_string(), call_id.clone());
                        }

                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !pending_tools.contains_key(&call_id) {
                            tool_order.push(call_id.clone());
                            pending_tools.insert(call_id.clone(), (name.clone(), String::new()));
                            debug!("Sub-agent Codex tool call start: {} ({})", name, call_id);
                        }

                        if let Some(arguments) = item.get("arguments").and_then(|a| a.as_str()) {
                            if let Some((_, args_buf)) = pending_tools.get_mut(&call_id) {
                                if args_buf.is_empty() {
                                    args_buf.push_str(arguments);
                                }
                            }
                        }
                    }
                }
                "response.function_call_arguments.delta" => {
                    if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                        if let Some(call_id) = Self::resolve_codex_tool_call_id(
                            &json,
                            &item_to_call_id,
                            &pending_tools,
                        ) {
                            if let Some((_, args_buf)) = pending_tools.get_mut(&call_id) {
                                args_buf.push_str(delta);
                            }
                        }
                    }
                }
                "response.function_call_arguments.done" => {
                    if let Some(call_id) =
                        Self::resolve_codex_tool_call_id(&json, &item_to_call_id, &pending_tools)
                    {
                        if let Some(arguments) = json.get("arguments").and_then(|a| a.as_str()) {
                            if let Some((_, args_buf)) = pending_tools.get_mut(&call_id) {
                                if args_buf.is_empty() {
                                    args_buf.push_str(arguments);
                                }
                            }
                        }
                    }
                }
                "response.usage" => {
                    let usage_obj = json.get("usage").unwrap_or(&json);
                    let input_tokens = usage_obj
                        .get("input_tokens")
                        .or_else(|| usage_obj.get("input"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    let output_tokens = usage_obj
                        .get("output_tokens")
                        .or_else(|| usage_obj.get("output"))
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0);
                    if input_tokens > 0 || output_tokens > 0 {
                        debug!(
                            "Sub-agent Codex usage: input={}, output={}",
                            input_tokens, output_tokens
                        );
                    }
                }
                "response.done" | "response.completed" => {
                    saw_completion = true;
                    if let Some(response) = json.get("response") {
                        if response.get("status").and_then(|s| s.as_str()) == Some("incomplete") {
                            let reason = response
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|r| r.as_str())
                                .unwrap_or("incomplete");
                            if matches!(reason, "max_output_tokens" | "max_tokens" | "length") {
                                finish_reason = "max_tokens";
                            }
                        }
                    }
                    break;
                }
                _ => {}
            }
        }

        let mut content: Vec<Value> = vec![];
        if !text_content.is_empty() {
            content.push(serde_json::json!({
                "type": "text",
                "text": text_content
            }));
        }

        let mut has_tool_calls = false;
        for call_id in tool_order {
            if let Some((name, args_json)) = pending_tools.remove(&call_id) {
                let input = if args_json.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str::<Value>(&args_json)
                        .unwrap_or_else(|_| serde_json::json!({ "raw": args_json }))
                };
                content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
                }));
                has_tool_calls = true;
            }
        }
        for (call_id, (name, args_json)) in pending_tools {
            let input = if args_json.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str::<Value>(&args_json)
                    .unwrap_or_else(|_| serde_json::json!({ "raw": args_json }))
            };
            content.push(serde_json::json!({
                "type": "tool_use",
                "id": call_id,
                "name": name,
                "input": input
            }));
            has_tool_calls = true;
        }

        if has_tool_calls {
            finish_reason = "tool_use";
        }

        if !saw_completion {
            return Err(anyhow::anyhow!(
                "Sub-agent Codex websocket ended before response completion (websocket-only mode)"
            ));
        }

        Ok(serde_json::json!({
            "content": content,
            "stop_reason": finish_reason,
            "model": model
        }))
    }

    fn resolve_codex_tool_call_id(
        json: &Value,
        item_to_call_id: &HashMap<String, String>,
        pending_tools: &HashMap<String, (String, String)>,
    ) -> Option<String> {
        if let Some(call_id) = json
            .get("call_id")
            .and_then(|i| i.as_str())
            .filter(|id| !id.is_empty())
        {
            return Some(call_id.to_string());
        }

        if let Some(item_id) = json
            .get("item_id")
            .and_then(|i| i.as_str())
            .filter(|id| !id.is_empty())
        {
            if let Some(call_id) = item_to_call_id.get(item_id) {
                return Some(call_id.clone());
            }
        }

        if pending_tools.len() == 1 {
            return pending_tools.keys().next().cloned();
        }

        None
    }

    fn resolve_codex_ws_url_for_tools(api_url: &str) -> Result<Url> {
        let mut url = Url::parse(api_url)
            .map_err(|e| anyhow::anyhow!("Invalid Codex API URL '{}': {}", api_url, e))?;

        url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
            .map_err(|_| anyhow::anyhow!("Failed to set websocket scheme for '{}'", api_url))?;

        Ok(url)
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
