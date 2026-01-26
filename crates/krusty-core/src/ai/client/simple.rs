//! Simple (non-streaming) API calls
//!
//! Used for quick tasks like title generation where streaming is overkill.

use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::core::AiClient;

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
    async fn call_simple_anthropic(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{
                "role": "user",
                "content": user_message
            }],
            "system": system_prompt
        });

        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        // Extract text from Anthropic response
        // MiniMax and other providers may return thinking blocks before text blocks,
        // so we need to iterate through all blocks to find text content
        let text = json
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|block| {
                        // Only extract from text blocks, skip thinking blocks
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            block.get("text").and_then(|t| t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
            .trim()
            .to_string();

        Ok(text)
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
        let text = json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|msg| msg.get("content"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(text)
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
        let text = json
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.as_array())
            .and_then(|arr| arr.first())
            .and_then(|part| part.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(text)
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

        Ok(collected_text.trim().to_string())
    }
}
