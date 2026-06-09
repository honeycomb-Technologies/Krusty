use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::super::core::AiClient;
use super::shared::trim_or_empty;
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::format::FormatHandler;
use crate::ai::models::ApiFormat;
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
        let body = openai_simple_body(
            self.config().api_format,
            model,
            max_tokens,
            vec![
                serde_json::json!({"role": "system", "content": system_prompt}),
                serde_json::json!({"role": "user", "content": user_message}),
            ],
        );

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        Ok(trim_or_empty(extract_openai_text(&json)))
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

        let body = openai_simple_body(self.config().api_format, model, max_tokens, api_messages);

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

        Ok(trim_or_empty(extract_openai_text(&json)))
    }
}

fn openai_simple_body(
    api_format: ApiFormat,
    model: &str,
    max_tokens: usize,
    messages: Vec<Value>,
) -> Value {
    if matches!(api_format, ApiFormat::OpenAIResponses) {
        serde_json::json!({
            "model": model,
            "max_output_tokens": max_tokens,
            "input": messages,
        })
    } else {
        serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
        })
    }
}

fn extract_openai_text(json: &Value) -> Option<&str> {
    json.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|content| content.as_str())
        .or_else(|| json.get("output_text").and_then(|text| text.as_str()))
        .or_else(|| extract_responses_output_text(json))
}

fn extract_responses_output_text(json: &Value) -> Option<&str> {
    json.get("output")
        .and_then(|output| output.as_array())
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.get("content")
                    .and_then(|content| content.as_array())
                    .and_then(|content| {
                        content.iter().find_map(|part| {
                            part.get("text")
                                .or_else(|| part.get("content"))
                                .and_then(|text| text.as_str())
                        })
                    })
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn simple_body_uses_responses_shape() {
        let body = openai_simple_body(
            ApiFormat::OpenAIResponses,
            "grok-build",
            42,
            vec![json!({"role": "user", "content": "hello"})],
        );

        assert!(body.get("messages").is_none());
        assert_eq!(body["input"][0]["content"], "hello");
        assert_eq!(body["max_output_tokens"], 42);
    }

    #[test]
    fn extracts_responses_output_text() {
        let json = json!({
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "hello"}]
            }]
        });

        assert_eq!(extract_openai_text(&json), Some("hello"));
    }
}
