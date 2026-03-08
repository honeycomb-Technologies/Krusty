//! Simple (non-streaming) API calls
//!
//! Used for quick tasks like title generation where streaming is overkill.
//! Also provides `call_with_conversation` for cache-safe fork operations
//! (summarization/compaction) that reuse the parent conversation's cached prefix.

use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::core::AiClient;
use super::streaming::partition_system_messages;
use crate::ai::format::anthropic::AnthropicFormat;
use crate::ai::format::google::GoogleFormat;
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::format::FormatHandler;
use crate::ai::providers::ProviderCapabilities;
use crate::ai::types::ModelMessage;

fn trim_or_empty(text: Option<&str>) -> String {
    text.unwrap_or("").trim().to_string()
}

fn collect_anthropic_text(blocks: &[Value]) -> String {
    let mut text = String::new();
    for block in blocks {
        // MiniMax and other providers may return thinking blocks before text blocks.
        if block.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        if let Some(chunk) = block.get("text").and_then(|t| t.as_str()) {
            text.push_str(chunk);
        }
    }
    text
}

impl AiClient {
    /// Make a simple non-streaming API call
    ///
    /// Used for quick tasks like title generation where streaming is overkill.
    /// Returns the text content directly. Routes to appropriate format handler.
    pub async fn call_simple(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        // ChatGPT Codex format requires streaming - handle specially
        if self.config().uses_chatgpt_codex_format() {
            return self
                .call_simple_chatgpt_codex(model, system_prompt, user_message)
                .await;
        }

        // Route to appropriate format handler based on API format
        if self.config().uses_openai_format() {
            return self
                .call_simple_openai(model, system_prompt, user_message, max_tokens)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_simple_google(model, system_prompt, user_message, max_tokens)
                .await;
        }

        // Anthropic format (default)
        self.call_simple_anthropic(model, system_prompt, user_message, max_tokens)
            .await
    }

    /// Simple non-streaming call using Anthropic format
    ///
    /// Uses cache_control on the system prompt when the provider supports it,
    /// so repeated calls with the same system prompt benefit from caching.
    async fn call_simple_anthropic(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        // Only apply cache_control for providers that support prompt caching.
        // MiniMax, Z.ai, etc. use Anthropic format but may reject cache_control blocks.
        let capabilities =
            crate::ai::providers::ProviderCapabilities::for_provider(self.provider_id());

        let system_value: serde_json::Value = if capabilities.prompt_caching {
            serde_json::json!([{
                "type": "text",
                "text": system_prompt,
                "cache_control": {"type": "ephemeral"}
            }])
        } else {
            serde_json::Value::String(system_prompt.to_string())
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{
                "role": "user",
                "content": user_message
            }],
            "system": system_value
        });

        // Auto-caching: API places breakpoint on the last cacheable block
        if capabilities.prompt_caching {
            body["cache_control"] = serde_json::json!({"type": "ephemeral"});
        }

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        let text = json
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| collect_anthropic_text(arr))
            .unwrap_or_default();

        Ok(trim_or_empty(Some(&text)))
    }

    /// Simple non-streaming call using OpenAI format
    async fn call_simple_openai(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_message}
            ]
        });

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        // Extract text from OpenAI response format
        Ok(trim_or_empty(
            json.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|msg| msg.get("content"))
                .and_then(|t| t.as_str()),
        ))
    }

    /// Simple non-streaming call using Google format
    async fn call_simple_google(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let body = serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": user_message}]
            }],
            "systemInstruction": {
                "parts": [{"text": system_prompt}]
            },
            "generationConfig": {
                "maxOutputTokens": max_tokens
            }
        });

        let request = self.build_request(&self.config().api_url());
        debug!("Google simple call to model: {}", model);

        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        // Extract text from Google response format
        Ok(trim_or_empty(
            json.get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.as_array())
                .and_then(|arr| arr.first())
                .and_then(|part| part.get("text"))
                .and_then(|t| t.as_str()),
        ))
    }

    /// Simple call using ChatGPT Codex (Responses API) format
    ///
    /// Codex requires `stream: true`, so we stream and collect the response.
    async fn call_simple_chatgpt_codex(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String> {
        use futures::StreamExt;

        // Build Codex-format request body
        let body = serde_json::json!({
            "model": model,
            "instructions": system_prompt,
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": user_message
                }]
            }],
            "tools": [],
            "store": false,
            "stream": true  // Required by Codex
        });

        debug!("ChatGPT Codex simple call to model: {}", model);

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        // Stream and collect text
        let mut collected_text = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);

            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        continue;
                    }
                    if let Ok(json) = serde_json::from_str::<Value>(data) {
                        // Handle text delta events
                        if json.get("type").and_then(|t| t.as_str())
                            == Some("response.output_text.delta")
                        {
                            if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                                collected_text.push_str(delta);
                            }
                        }
                    }
                }
            }
        }

        Ok(trim_or_empty(Some(&collected_text)))
    }

    /// Non-streaming API call that reuses the parent conversation's cached prefix.
    ///
    /// Instead of flattening conversation into a single user message (which shares
    /// zero cache prefix with the parent conversation), this sends the actual
    /// conversation messages as API messages and appends a new user instruction.
    ///
    /// The cached prefix from the parent conversation (system prompt + conversation
    /// history) is fully reused, so the only uncached tokens are the appended message.
    /// This follows Thariq's lesson: "When we run compaction, we use the exact same
    /// system prompt, user context, system context, and tool definitions."
    pub async fn call_with_conversation(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        if self.config().uses_chatgpt_codex_format() {
            // Codex requires streaming — combine system prompt and fall back to simple
            return self
                .call_simple_chatgpt_codex(model, base_system_prompt, appended_user_message)
                .await;
        }

        if self.config().uses_openai_format() {
            return self
                .call_conversation_openai(
                    model,
                    base_system_prompt,
                    conversation,
                    appended_user_message,
                    max_tokens,
                )
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_conversation_google(
                    model,
                    base_system_prompt,
                    conversation,
                    appended_user_message,
                    max_tokens,
                )
                .await;
        }

        self.call_conversation_anthropic(
            model,
            base_system_prompt,
            conversation,
            appended_user_message,
            max_tokens,
        )
        .await
    }

    /// Cache-safe conversation call using Anthropic format.
    ///
    /// Builds the same multi-block system prompt structure as the streaming path:
    /// base prompt (cached) → project context (cached) → session context (not cached).
    /// Conversation messages are converted using the same format handler, so the
    /// entire prefix matches what the parent conversation built.
    async fn call_conversation_anthropic(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let capabilities = ProviderCapabilities::for_provider(self.provider_id());
        let format_handler = AnthropicFormat::new();

        // Convert parent conversation messages (System role filtered by format handler)
        let mut api_messages =
            format_handler.convert_messages(conversation, Some(self.provider_id()));

        // Append the new user message at the end
        api_messages.push(serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": appended_user_message}]
        }));

        // Build system prompt with the same multi-block structure as streaming.
        // This ensures the cached prefix from the parent conversation is reused.
        let (project_context, session_context) = partition_system_messages(conversation);

        let system_value: Value = if capabilities.prompt_caching {
            let mut blocks: Vec<Value> = Vec::new();

            // Block 1: Base system prompt — cached
            if !base_system_prompt.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": base_system_prompt,
                    "cache_control": {"type": "ephemeral"}
                }));
            }

            // Block 2 (optional): Project context — cached
            if !project_context.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": project_context,
                    "cache_control": {"type": "ephemeral"}
                }));
            }

            // Block 3 (optional): Session context — not cached (dynamic)
            if !session_context.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": session_context
                }));
            }

            Value::Array(blocks)
        } else {
            // No caching: combine into single string
            let mut system = base_system_prompt.to_string();
            if !project_context.is_empty() {
                system.push_str("\n\n---\n\n");
                system.push_str(&project_context);
            }
            if !session_context.is_empty() {
                system.push_str("\n\n---\n\n");
                system.push_str(&session_context);
            }
            Value::String(system)
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system_value,
            "messages": api_messages,
        });

        // Auto-caching: API places breakpoint on the last cacheable block.
        // Combined with block-level caching on system prompt blocks, this
        // ensures both the static prefix and conversation are cached.
        if capabilities.prompt_caching {
            body["cache_control"] = serde_json::json!({"type": "ephemeral"});
        }

        debug!(
            "Cache-safe Anthropic call: {} conversation messages + appended user message",
            conversation.len()
        );

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        let text = json
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| collect_anthropic_text(arr))
            .unwrap_or_default();

        Ok(trim_or_empty(Some(&text)))
    }

    /// Cache-safe conversation call using OpenAI format.
    ///
    /// Combines system content in the same stability order as streaming
    /// (base → project → session) for optimal automatic prefix caching.
    async fn call_conversation_openai(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let format_handler = OpenAIFormat::new(self.config().api_format);

        let mut api_messages =
            format_handler.convert_messages(conversation, Some(self.provider_id()));

        // Prepend system message with combined prompt (same order as streaming)
        let system_prompt = build_combined_system_prompt(base_system_prompt, conversation);
        api_messages.insert(
            0,
            serde_json::json!({"role": "system", "content": system_prompt}),
        );

        // Append user message
        api_messages.push(serde_json::json!({
            "role": "user",
            "content": appended_user_message
        }));

        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": api_messages,
        });

        debug!(
            "Cache-safe OpenAI call: {} conversation messages + appended user message",
            conversation.len()
        );

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        Ok(trim_or_empty(
            json.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|msg| msg.get("content"))
                .and_then(|t| t.as_str()),
        ))
    }

    /// Cache-safe conversation call using Google format.
    async fn call_conversation_google(
        &self,
        _model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let format_handler = GoogleFormat::new();

        let mut contents = format_handler.convert_messages(conversation, Some(self.provider_id()));

        // Append user message
        contents.push(serde_json::json!({
            "role": "user",
            "parts": [{"text": appended_user_message}]
        }));

        let system_prompt = build_combined_system_prompt(base_system_prompt, conversation);

        let body = serde_json::json!({
            "contents": contents,
            "systemInstruction": {
                "parts": [{"text": system_prompt}]
            },
            "generationConfig": {
                "maxOutputTokens": max_tokens
            }
        });

        debug!(
            "Cache-safe Google call: {} conversation messages + appended user message",
            conversation.len()
        );

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        Ok(trim_or_empty(
            json.get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.as_array())
                .and_then(|arr| arr.first())
                .and_then(|part| part.get("text"))
                .and_then(|t| t.as_str()),
        ))
    }
}

/// Build a combined system prompt for non-Anthropic providers.
///
/// Orders content by stability (base → project → session) matching
/// the streaming path for optimal automatic prefix caching.
fn build_combined_system_prompt(base: &str, conversation: &[ModelMessage]) -> String {
    let (project_context, session_context) = partition_system_messages(conversation);

    let mut prompt = base.to_string();
    if !project_context.is_empty() {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str(&project_context);
    }
    if !session_context.is_empty() {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str(&session_context);
    }
    prompt
}
