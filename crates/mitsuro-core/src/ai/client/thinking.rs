//! Extended thinking API calls
//!
//! Used for complex tasks where deep analysis is needed.

use anyhow::Result;
use serde_json::Value;
use tracing::info;

use super::config::CallOptions;
use super::core::AiClient;
use crate::ai::types::ThinkingConfig;

impl AiClient {
    /// Call the API with extended thinking enabled (non-streaming)
    ///
    /// Used for complex summarization tasks where we want deep analysis.
    /// Returns the text content after thinking completes.
    pub async fn call_with_thinking(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        thinking_budget: u32,
    ) -> Result<String> {
        self.ensure_run_model(model)?;
        // For thinking, max_tokens must be > budget_tokens
        let max_tokens = thinking_budget + 16000;
        let options = self.canonical_call_options(
            model,
            &CallOptions {
                max_tokens: Some(max_tokens as usize),
                system_prompt: Some(system_prompt.to_string()),
                thinking: Some(ThinkingConfig::default()),
                ..Default::default()
            },
        );
        let prompt_sections =
            self.system_prompt_sections(model, &[], options.system_prompt.as_deref(), None);
        let system_prompt = prompt_sections.combined();

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": options.max_tokens.unwrap_or(max_tokens as usize),
            "messages": [{
                "role": "user",
                "content": user_message
            }],
            "system": system_prompt,
        });

        if options.thinking.is_some() {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": thinking_budget
            });
        }

        // Effort parameter is ONLY supported by Opus 4.5
        if options.thinking.is_some() && model.contains("opus-4-5") {
            body["output_config"] = serde_json::json!({
                "effort": "high"
            });
        }

        // Build beta headers for thinking
        let mut beta_parts = Vec::new();
        if options.thinking.is_some() {
            beta_parts.push("interleaved-thinking-2025-05-14");
        }
        if options.thinking.is_some() && model.contains("opus-4-5") {
            beta_parts.push("effort-2025-11-24");
        }

        let request = self.build_request_with_beta(&self.config().api_url(), &beta_parts);

        info!(
            "Calling API with extended thinking (budget: {})",
            thinking_budget
        );
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        // Extract text from response (skip thinking blocks, get text blocks)
        let mut text_content = String::new();
        if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if let Some(block_type) = block.get("type").and_then(|t| t.as_str()) {
                    if block_type == "text" {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            text_content.push_str(text);
                        }
                    }
                }
            }
        }

        Ok(text_content.trim().to_string())
    }
}
