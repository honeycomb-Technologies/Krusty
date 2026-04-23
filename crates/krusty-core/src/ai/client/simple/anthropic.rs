use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::super::core::AiClient;
use super::shared::{collect_anthropic_text, trim_or_empty};
use crate::ai::format::anthropic::AnthropicFormat;
use crate::ai::format::FormatHandler;
use crate::ai::providers::ProviderCapabilities;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Simple non-streaming call using Anthropic format
    ///
    /// Uses cache_control on the system prompt when the provider supports it,
    /// so repeated calls with the same system prompt benefit from caching.
    pub(super) async fn call_simple_anthropic(
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

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
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

    /// Cache-safe conversation call using Anthropic format.
    ///
    /// Builds the same multi-block system prompt structure as the streaming path:
    /// base prompt (cached) → project context (cached) → session context (not cached).
    /// Conversation messages are converted using the same format handler, so the
    /// entire prefix matches what the parent conversation built.
    pub(super) async fn call_conversation_anthropic(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let capabilities = ProviderCapabilities::for_provider(self.provider_id());
        let format_handler = AnthropicFormat::new();
        let prompt_sections =
            self.system_prompt_sections(model, conversation, Some(base_system_prompt), None);

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
        let system_value: Value = if capabilities.prompt_caching {
            let mut blocks: Vec<Value> = Vec::new();

            // Block 1: Base system prompt — cached
            if !prompt_sections.base_prompt.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": prompt_sections.base_prompt.as_str(),
                    "cache_control": {"type": "ephemeral"}
                }));
            }

            // Block 2 (optional): Project context — cached
            if !prompt_sections.project_context.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": prompt_sections.project_context.as_str(),
                    "cache_control": {"type": "ephemeral"}
                }));
            }

            // Block 3 (optional): Session context — not cached (dynamic)
            if !prompt_sections.session_context.is_empty() {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": prompt_sections.session_context.as_str()
                }));
            }

            Value::Array(blocks)
        } else {
            Value::String(prompt_sections.combined())
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

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
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
}
