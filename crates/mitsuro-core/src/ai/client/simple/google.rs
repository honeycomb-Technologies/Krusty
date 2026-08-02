use anyhow::Result;
use serde_json::Value;
use tracing::debug;

use super::super::core::AiClient;
use super::shared::trim_or_empty;
use super::SimpleCallResult;
use crate::ai::format::google::GoogleFormat;
use crate::ai::format::FormatHandler;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::ModelMessage;
use crate::ai::usage::parse_google_usage;

impl AiClient {
    /// Simple non-streaming call using Google format
    pub(super) async fn call_simple_google(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<SimpleCallResult> {
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

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());
        debug!("Google simple call to model: {}", model);

        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        // Extract text from Google response format
        Ok(simple_google_result(&json))
    }

    /// Cache-safe conversation call using Google format.
    pub(super) async fn call_conversation_google(
        &self,
        model: &str,
        base_system_prompt: &str,
        conversation: &[ModelMessage],
        appended_user_message: &str,
        max_tokens: usize,
    ) -> Result<SimpleCallResult> {
        let format_handler = GoogleFormat::new();
        let prompt_sections =
            self.system_prompt_sections(model, conversation, Some(base_system_prompt), None);

        let mut contents = format_handler.convert_messages(conversation, Some(self.provider_id()));

        // Append user message
        contents.push(serde_json::json!({
            "role": "user",
            "parts": [{"text": appended_user_message}]
        }));

        let system_prompt = prompt_sections.combined();

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

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());
        let response = request.json(&body).send().await?;
        let response = self.handle_error_response(response).await?;

        let json: Value = response.json().await?;

        Ok(simple_google_result(&json))
    }
}

fn simple_google_result(json: &Value) -> SimpleCallResult {
    SimpleCallResult {
        text: trim_or_empty(
            json.get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|candidate| candidate.get("content"))
                .and_then(|content| content.get("parts"))
                .and_then(|parts| parts.as_array())
                .and_then(|arr| arr.first())
                .and_then(|part| part.get("text"))
                .and_then(|t| t.as_str()),
        ),
        usage: parse_google_usage(json),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::simple_google_result;

    #[test]
    fn simple_result_preserves_google_usage() {
        let result = simple_google_result(&json!({
            "candidates": [{"content": {"parts": [{"text": "hello"}]}}],
            "usageMetadata": {
                "promptTokenCount": 1000,
                "cachedContentTokenCount": 700,
                "candidatesTokenCount": 50,
                "thoughtsTokenCount": 500,
                "totalTokenCount": 1550
            }
        }));

        assert_eq!(result.text, "hello");
        let usage = result.usage.expect("usage");
        assert_eq!(usage.prompt_tokens, 300);
        assert_eq!(usage.reasoning_tokens, 500);
        assert_eq!(usage.logical_total_tokens(), 1_550);
    }
}
