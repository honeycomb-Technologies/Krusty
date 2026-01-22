//! Streaming API calls
//!
//! Handles SSE streaming responses from different providers.

use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::config::CallOptions;
use super::core::{AiClient, KRUSTY_SYSTEM_PROMPT};
use crate::ai::format::anthropic::AnthropicFormat;
use crate::ai::format::google::GoogleFormat;
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::format::FormatHandler;
use crate::ai::parsers::{AnthropicParser, GoogleParser, OpenAIParser};
use crate::ai::providers::{ProviderCapabilities, ReasoningFormat};
use crate::ai::reasoning::ReasoningConfig;
use crate::ai::sse::{create_streaming_channels, spawn_buffer_processor, SseStreamProcessor};
use crate::ai::streaming::StreamPart;
use crate::ai::transform::build_provider_params;
use crate::ai::types::{Content, ModelMessage, Role};

impl AiClient {
    /// Call the API with streaming response
    pub async fn call_streaming(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let call_start = Instant::now();
        info!("=== API CALL START ===");
        info!(
            "Model: {}, Messages: {}, Tools: {}, Thinking: {}, Format: {:?}",
            self.config().model,
            messages.len(),
            options.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            options.thinking.is_some(),
            self.config().api_format
        );

        // Route to appropriate format handler based on API format
        if self.config().uses_openai_format() {
            return self
                .call_streaming_openai(messages, options, call_start)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_streaming_google(messages, options, call_start)
                .await;
        }

        // Anthropic format (default)
        self.call_streaming_anthropic(messages, options, call_start)
            .await
    }

    /// Streaming call using Anthropic format
    async fn call_streaming_anthropic(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let format_handler = AnthropicFormat::new();
        let anthropic_messages =
            format_handler.convert_messages(&messages, Some(self.provider_id()));

        // Extract any system messages from conversation (e.g., pinch context)
        let injected_context: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .filter_map(|m| {
                m.content.iter().find_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        // Build system prompt
        let mut system = if let Some(custom) = &options.system_prompt {
            custom.clone()
        } else {
            KRUSTY_SYSTEM_PROMPT.to_string()
        };

        // Handle injected context
        if !injected_context.is_empty() {
            system.push_str("\n\n---\n\n");
            system.push_str(&injected_context);
            info!(
                "Injected {} chars of context into system prompt",
                injected_context.len()
            );
        }

        // Determine max_tokens based on reasoning format
        let fallback_tokens = options.max_tokens.unwrap_or(self.config().max_tokens) as u32;
        let legacy_thinking = options.thinking.is_some();
        let max_tokens = ReasoningConfig::max_tokens_for_format(
            options.reasoning_format,
            fallback_tokens,
            legacy_thinking,
        );

        // Build request body
        let mut body = serde_json::json!({
            "model": self.config().model,
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
            "stream": true,
        });

        // Add system prompt with cache control
        if !system.is_empty() {
            if options.enable_caching {
                body["system"] = serde_json::json!([{
                    "type": "text",
                    "text": system,
                    "cache_control": {"type": "ephemeral"}
                }]);
                debug!("Cache breakpoint added to system prompt");
            } else {
                body["system"] = Value::String(system);
            }
        }

        // Temperature incompatible with reasoning - only add if reasoning is off
        let reasoning_enabled = options.reasoning_format.is_some() || options.thinking.is_some();
        if !reasoning_enabled {
            if let Some(temp) = options.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
        }

        // Build tools array
        let mut all_tools: Vec<Value> = Vec::new();

        if let Some(tools) = &options.tools {
            for tool in tools {
                all_tools.push(serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                }));
            }
        }

        // Add server-executed tools based on provider capabilities
        let capabilities = ProviderCapabilities::for_provider(self.provider_id());
        self.add_server_tools(&mut all_tools, &mut body, options, &capabilities);

        // Add all tools to body with cache breakpoint on last one
        if !all_tools.is_empty() {
            if options.enable_caching {
                if let Some(last) = all_tools.last_mut() {
                    last["cache_control"] = serde_json::json!({"type": "ephemeral"});
                }
                debug!("Cache breakpoint added to last tool");
            }
            body["tools"] = Value::Array(all_tools);
        }

        // Add reasoning/thinking config
        self.add_reasoning_config(&mut body, options, reasoning_enabled);

        // Add context management
        self.add_context_management(&mut body, options);

        // Add provider-specific parameters
        self.add_provider_params(&mut body, reasoning_enabled);

        debug!("Calling {} API with streaming", self.provider_id());

        // Build beta headers
        let beta_headers = self.build_beta_headers(options);
        let request = self.build_request_with_beta(&self.config().api_url(), &beta_headers);

        // Send request
        info!("Sending API request...");
        let response = request.json(&body).send().await?;
        let request_duration = call_start.elapsed();

        let status = response.status();
        info!("API response: {} in {:?}", status, request_duration);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("API error response: {} - {}", status, error_text);
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        // Set up streaming channels
        let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
        spawn_buffer_processor(buffer_rx, tx.clone());

        let mut processor = SseStreamProcessor::new(tx, buffer_tx);
        let parser = AnthropicParser::new();

        // Spawn task to process the stream
        info!("Starting stream processing task");
        let stream = response.bytes_stream();
        tokio::spawn(async move {
            tokio::pin!(stream);
            let mut chunk_count = 0;
            while let Some(chunk) = stream.next().await {
                chunk_count += 1;
                match chunk {
                    Ok(bytes) => {
                        if let Err(e) = processor.process_chunk(bytes, &parser).await {
                            warn!("Error processing chunk #{}: {}", chunk_count, e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Stream read error at chunk #{}: {}", chunk_count, e);
                        break;
                    }
                }
            }
            info!("Stream ended after {} chunks", chunk_count);
            processor.finish().await;
        });

        Ok(rx)
    }

    /// Streaming call using OpenAI format
    async fn call_streaming_openai(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        // Check if we're using ChatGPT Codex API (OAuth) vs standard OpenAI API
        let is_chatgpt_codex = self
            .config()
            .base_url
            .as_ref()
            .map(|url| url.contains("chatgpt.com"))
            .unwrap_or(false);

        if is_chatgpt_codex {
            info!(
                "Using ChatGPT Codex format for {} (OAuth)",
                self.config().model
            );
        } else {
            info!(
                "Using OpenAI chat/completions format for {}",
                self.config().model
            );
        }

        let format_handler = OpenAIFormat::new(self.config().api_format);

        // Extract system prompt
        let system: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .filter_map(|m| {
                m.content.iter().find_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let system_prompt = if !system.is_empty() {
            Some(format!("{}\n\n{}", KRUSTY_SYSTEM_PROMPT, system))
        } else if options.system_prompt.is_some() {
            options.system_prompt.clone()
        } else {
            Some(KRUSTY_SYSTEM_PROMPT.to_string())
        };

        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);

        // Build request body based on API type
        let body = if is_chatgpt_codex {
            // ChatGPT Codex API format (different from standard Responses API)
            self.build_chatgpt_codex_body(
                &messages,
                &system_prompt,
                max_tokens,
                options,
                &format_handler,
            )
        } else {
            // Standard OpenAI format (Chat Completions or Responses API)
            let openai_messages =
                format_handler.convert_messages(&messages, Some(self.provider_id()));

            // Responses API uses "input", Chat Completions uses "messages"
            let (messages_key, max_tokens_key) = if matches!(
                self.config().api_format,
                crate::ai::models::ApiFormat::OpenAIResponses
            ) {
                ("input", "max_output_tokens")
            } else {
                ("messages", "max_tokens")
            };

            let mut body = serde_json::json!({
                "model": self.config().model,
                "stream": true,
            });
            body[max_tokens_key] = serde_json::json!(max_tokens);
            body[messages_key] = serde_json::json!(openai_messages);

            // Add system message at the start
            if let Some(sys) = system_prompt {
                if let Some(msgs) = body.get_mut(messages_key).and_then(|m| m.as_array_mut()) {
                    msgs.insert(
                        0,
                        serde_json::json!({
                            "role": "system",
                            "content": sys
                        }),
                    );
                }
            }

            // Add temperature
            if options.thinking.is_none() {
                if let Some(temp) = options.temperature {
                    body["temperature"] = serde_json::json!(temp);
                }
            }

            // Add tools
            if let Some(tools) = &options.tools {
                let openai_tools = format_handler.convert_tools(tools);
                if !openai_tools.is_empty() {
                    body["tools"] = serde_json::json!(openai_tools);
                }
            }

            body
        };

        debug!("OpenAI request to: {}", self.config().api_url());

        let request = self.build_request(&self.config().api_url());

        info!("Sending OpenAI format request...");
        let response = request.json(&body).send().await?;
        let request_duration = call_start.elapsed();

        let status = response.status();
        info!("API response: {} in {:?}", status, request_duration);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("API error response: {} - {}", status, error_text);
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        // Set up streaming channels
        let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
        spawn_buffer_processor(buffer_rx, tx.clone());

        let mut processor = SseStreamProcessor::new(tx, buffer_tx);
        let parser = OpenAIParser::new();

        info!("Starting OpenAI stream processing task");
        let stream = response.bytes_stream();
        tokio::spawn(async move {
            tokio::pin!(stream);
            let mut chunk_count = 0;
            while let Some(chunk) = stream.next().await {
                chunk_count += 1;
                match chunk {
                    Ok(bytes) => {
                        if let Err(e) = processor.process_chunk(bytes, &parser).await {
                            warn!("Error processing OpenAI chunk #{}: {}", chunk_count, e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("OpenAI stream read error at chunk #{}: {}", chunk_count, e);
                        break;
                    }
                }
            }
            info!("OpenAI stream ended after {} chunks", chunk_count);
            processor.finish().await;
        });

        Ok(rx)
    }

    /// Streaming call using Google format
    async fn call_streaming_google(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        info!("Using Google/Gemini format for {}", self.config().model);

        let format_handler = GoogleFormat::new();
        let contents = format_handler.convert_messages(&messages, Some(self.provider_id()));

        // Extract system prompt
        let system: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .filter_map(|m| {
                m.content.iter().find_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let system_instruction = if !system.is_empty() {
            Some(format!("{}\n\n{}", KRUSTY_SYSTEM_PROMPT, system))
        } else if let Some(custom) = &options.system_prompt {
            Some(custom.clone())
        } else {
            Some(KRUSTY_SYSTEM_PROMPT.to_string())
        };

        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);

        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": max_tokens,
            }
        });

        if let Some(sys) = system_instruction {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": sys}]
            });
        }

        if let Some(temp) = options.temperature {
            body["generationConfig"]["temperature"] = serde_json::json!(temp);
        }

        if let Some(tools) = &options.tools {
            let google_tools = format_handler.convert_tools(tools);
            if !google_tools.is_empty() {
                body["tools"] = serde_json::json!([{
                    "functionDeclarations": google_tools
                }]);
            }
        }

        debug!("Google request to: {}", self.config().api_url());

        let request = self.build_request(&self.config().api_url());

        info!("Sending Google format request...");
        let response = request.json(&body).send().await?;
        let request_duration = call_start.elapsed();

        let status = response.status();
        info!("API response: {} in {:?}", status, request_duration);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("API error response: {} - {}", status, error_text);
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        // Set up streaming channels
        let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
        spawn_buffer_processor(buffer_rx, tx.clone());

        let mut processor = SseStreamProcessor::new(tx, buffer_tx);
        let parser = GoogleParser::new();

        info!("Starting Google stream processing task");
        let stream = response.bytes_stream();
        tokio::spawn(async move {
            tokio::pin!(stream);
            let mut chunk_count = 0;
            while let Some(chunk) = stream.next().await {
                chunk_count += 1;
                match chunk {
                    Ok(bytes) => {
                        if let Err(e) = processor.process_chunk(bytes, &parser).await {
                            warn!("Error processing Google chunk #{}: {}", chunk_count, e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Google stream read error at chunk #{}: {}", chunk_count, e);
                        break;
                    }
                }
            }
            info!("Google stream ended after {} chunks", chunk_count);
            processor.finish().await;
        });

        Ok(rx)
    }

    /// Add server-executed tools (web search, web fetch) to the request
    fn add_server_tools(
        &self,
        all_tools: &mut Vec<Value>,
        body: &mut Value,
        options: &CallOptions,
        capabilities: &ProviderCapabilities,
    ) {
        // Anthropic server-executed web tools
        if capabilities.web_search {
            if let Some(search) = &options.web_search {
                let mut spec = serde_json::json!({
                    "type": "web_search_20250305",
                    "name": "web_search",
                });
                if let Some(max_uses) = search.max_uses {
                    spec["max_uses"] = serde_json::json!(max_uses);
                }
                all_tools.push(spec);
                debug!("Web search tool enabled (server-side)");
            }
        }

        if capabilities.web_fetch {
            if let Some(fetch) = &options.web_fetch {
                let mut spec = serde_json::json!({
                    "type": "web_fetch_20250910",
                    "name": "web_fetch",
                    "citations": { "enabled": fetch.citations_enabled },
                });
                if let Some(max_uses) = fetch.max_uses {
                    spec["max_uses"] = serde_json::json!(max_uses);
                }
                if let Some(max_tokens) = fetch.max_content_tokens {
                    spec["max_content_tokens"] = serde_json::json!(max_tokens);
                }
                all_tools.push(spec);
                debug!("Web fetch tool enabled (server-side)");
            }
        }

        // OpenRouter web search: append :online suffix to model name
        if capabilities.web_plugins && options.web_search.is_some() {
            if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
                if !model.ends_with(":online") {
                    let online_model = format!("{}:online", model);
                    body["model"] = serde_json::json!(online_model);
                    info!(
                        "OpenRouter web search enabled via model suffix: {}",
                        online_model
                    );
                }
            }
        }
    }

    /// Add reasoning/thinking config to the request body
    fn add_reasoning_config(
        &self,
        body: &mut Value,
        options: &CallOptions,
        reasoning_enabled: bool,
    ) {
        let budget_tokens = options.thinking.as_ref().map(|t| t.budget_tokens);

        if let Some(reasoning_config) = ReasoningConfig::build(
            options.reasoning_format,
            reasoning_enabled,
            budget_tokens,
            None,
        ) {
            match options.reasoning_format {
                Some(ReasoningFormat::Anthropic) => {
                    body["thinking"] = reasoning_config;
                    debug!(
                        "Anthropic thinking enabled with budget: {}",
                        budget_tokens.unwrap_or(32000)
                    );
                }
                Some(ReasoningFormat::OpenAI) => {
                    if let Some(obj) = reasoning_config.as_object() {
                        for (k, v) in obj {
                            body[k] = v.clone();
                        }
                    }
                    debug!("OpenAI reasoning enabled with high effort");
                }
                Some(ReasoningFormat::DeepSeek) => {
                    if let Some(obj) = reasoning_config.as_object() {
                        for (k, v) in obj {
                            body[k] = v.clone();
                        }
                    }
                    debug!("DeepSeek reasoning enabled");
                }
                None => {}
            }

            // Opus 4.5 effort config
            if let Some(effort_config) =
                ReasoningConfig::build_opus_effort(&self.config().model, reasoning_enabled)
            {
                body["output_config"] = effort_config;
                debug!("Using high effort for Opus 4.5");
            }
        } else if let Some(thinking) = &options.thinking {
            // Legacy support: if thinking is set without format, assume Anthropic
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": thinking.budget_tokens
            });
            debug!(
                "Legacy thinking enabled with budget: {}",
                thinking.budget_tokens
            );

            if let Some(effort_config) =
                ReasoningConfig::build_opus_effort(&self.config().model, true)
            {
                body["output_config"] = effort_config;
            }
        }
    }

    /// Add context management to the request body
    fn add_context_management(&self, body: &mut Value, options: &CallOptions) {
        if let Some(ctx_mgmt) = &options.context_management {
            let caps = ProviderCapabilities::for_provider(self.provider_id());
            if caps.context_management {
                body["context_management"] =
                    serde_json::to_value(ctx_mgmt).unwrap_or(serde_json::Value::Null);
                info!("Context management enabled: {} edits", ctx_mgmt.edits.len());
            } else {
                debug!(
                    "Skipping context_management for provider {:?} (not supported)",
                    self.provider_id()
                );
            }
        }
    }

    /// Add provider-specific parameters to the request body
    fn add_provider_params(&self, body: &mut Value, thinking_enabled: bool) {
        let provider_params =
            build_provider_params(&self.config().model, self.provider_id(), thinking_enabled);

        // Temperature incompatible with reasoning
        if !thinking_enabled {
            if let Some(temp) = provider_params.temperature {
                body["temperature"] = Value::Number(serde_json::Number::from(temp as i32));
                debug!(
                    "Setting temperature: {} for model {}",
                    temp,
                    self.config().model
                );
            }
        }

        if let Some(top_p) = provider_params.top_p {
            body["top_p"] = Value::Number(serde_json::Number::from_f64(top_p as f64).unwrap());
            debug!("Setting top_p: {} for model {}", top_p, self.config().model);
        }

        if let Some(top_k) = provider_params.top_k {
            body["top_k"] = Value::Number(serde_json::Number::from(top_k));
            debug!("Setting top_k: {} for model {}", top_k, self.config().model);
        }

        if let Some(chat_args) = provider_params.chat_template_args {
            body["chat_template_args"] = chat_args;
            info!(
                "Enabling chat_template_args for thinking model {}",
                self.config().model
            );
        }
    }

    /// Build beta headers based on options
    fn build_beta_headers(&self, options: &CallOptions) -> Vec<&'static str> {
        let mut beta_headers: Vec<&str> = Vec::new();

        // Add thinking beta headers for Anthropic reasoning format
        let anthropic_thinking =
            matches!(options.reasoning_format, Some(ReasoningFormat::Anthropic))
                || options.thinking.is_some();
        if anthropic_thinking {
            beta_headers.push("interleaved-thinking-2025-05-14");

            // Effort beta for Opus 4.5
            if self.config().model.contains("opus-4-5") {
                beta_headers.push("effort-2025-11-24");
            }
        }

        // Context management beta
        if options.context_management.is_some() {
            beta_headers.push("context-management-2025-06-27");
        }

        // Web tool beta headers
        let caps = ProviderCapabilities::for_provider(self.provider_id());
        if options.web_search.is_some() && caps.web_search {
            beta_headers.push("web-search-2025-03-05");
        }
        if options.web_fetch.is_some() && caps.web_fetch {
            beta_headers.push("web-fetch-2025-09-10");
        }

        beta_headers
    }

    /// Build request body for ChatGPT Codex API
    ///
    /// ChatGPT Codex has a different format than standard OpenAI APIs:
    /// - Uses "instructions" field for system prompt (required)
    /// - Uses "developer" role instead of "system"
    /// - Message content is array of {"type": "input_text", "text": "..."}
    fn build_chatgpt_codex_body(
        &self,
        messages: &[ModelMessage],
        system_prompt: &Option<String>,
        max_tokens: usize,
        options: &CallOptions,
        format_handler: &OpenAIFormat,
    ) -> Value {
        // Convert messages to Codex format
        let mut input_messages: Vec<Value> = Vec::new();

        for msg in messages.iter().filter(|m| m.role != Role::System) {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => continue, // Handled via instructions
            };

            // Check for tool results
            let has_tool_results = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolResult { .. }));

            if has_tool_results {
                for content in &msg.content {
                    if let Content::ToolResult {
                        tool_use_id,
                        output,
                        ..
                    } = content
                    {
                        let output_str = match output {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        input_messages.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": output_str
                        }));
                    }
                }
                continue;
            }

            // Check for tool calls
            let has_tool_use = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolUse { .. }));

            if has_tool_use && role == "assistant" {
                let mut tool_calls = Vec::new();
                let mut text_content = String::new();

                for content in &msg.content {
                    match content {
                        Content::Text { text } => text_content.push_str(text),
                        Content::ToolUse { id, name, input } => {
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

                let mut msg_obj = serde_json::json!({
                    "role": "assistant",
                    "tool_calls": tool_calls
                });
                if !text_content.is_empty() {
                    msg_obj["content"] = serde_json::json!([{
                        "type": "output_text",
                        "text": text_content
                    }]);
                }
                input_messages.push(msg_obj);
                continue;
            }

            // Regular message - extract text
            let text: String = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            if !text.is_empty() {
                // Codex format: content as array of typed objects
                input_messages.push(serde_json::json!({
                    "role": role,
                    "content": [{
                        "type": "input_text",
                        "text": text
                    }]
                }));
            }
        }

        // Build Codex request body
        let mut body = serde_json::json!({
            "model": self.config().model,
            "stream": true,
            "max_output_tokens": max_tokens,
            "input": input_messages,
            "tools": [],
            "reasoning": {
                "summary": "auto"
            }
        });

        // Instructions field is REQUIRED for Codex API
        if let Some(instructions) = system_prompt {
            body["instructions"] = serde_json::json!(instructions);
        } else {
            body["instructions"] = serde_json::json!(KRUSTY_SYSTEM_PROMPT);
        }

        // Add tools if provided
        if let Some(tools) = &options.tools {
            let codex_tools = format_handler.convert_tools(tools);
            if !codex_tools.is_empty() {
                body["tools"] = serde_json::json!(codex_tools);
                body["tool_choice"] = serde_json::json!("auto");
            }
        }

        debug!(
            "ChatGPT Codex request body: {} messages, {} tools",
            input_messages.len(),
            options.tools.as_ref().map(|t| t.len()).unwrap_or(0)
        );

        body
    }
}
