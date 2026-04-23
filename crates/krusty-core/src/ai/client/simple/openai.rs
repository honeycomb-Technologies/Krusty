use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::super::core::AiClient;
use super::shared::trim_or_empty;
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::format::FormatHandler;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Simple non-streaming call using OpenAI format
    pub(super) async fn call_simple_openai(
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

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
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

    /// Cache-safe conversation call using OpenAI format.
    ///
    /// Combines system content in the same stability order as streaming
    /// (base → project → session) for optimal automatic prefix caching.
    pub(super) async fn call_conversation_openai(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let format_handler = OpenAIFormat::new(self.config().api_format);
        let prompt_sections =
            self.system_prompt_sections(model, conversation, Some(base_system_prompt), None);

        let mut api_messages =
            format_handler.convert_messages(conversation, Some(self.provider_id()));

        // Prepend system message with combined prompt (same order as streaming)
        let system_prompt = prompt_sections.combined();
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

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
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
}
