//! OpenAI API format handler
//!
//! Handles conversion to OpenAI chat/completions and responses API formats.
//! Includes message alternation validation and thinking block preservation
//! for parity with Anthropic format handling.

mod messages;
mod request;
pub mod responses_input;

use serde_json::Value;

use super::{FormatHandler, RequestOptions};
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, ModelMessage};

/// OpenAI format handler
pub struct OpenAIFormat {
    api_format: ApiFormat,
    endpoint: String,
}

impl OpenAIFormat {
    pub fn new(format: ApiFormat) -> Self {
        let endpoint = match format {
            ApiFormat::OpenAIResponses => "/v1/responses".to_string(),
            _ => "/v1/chat/completions".to_string(),
        };
        Self {
            api_format: format,
            endpoint,
        }
    }

    pub(super) fn is_responses_format(&self) -> bool {
        matches!(self.api_format, ApiFormat::OpenAIResponses)
    }
}

impl FormatHandler for OpenAIFormat {
    fn convert_messages(
        &self,
        messages: &[ModelMessage],
        provider_id: Option<ProviderId>,
    ) -> Vec<Value> {
        self.convert_messages_impl(messages, provider_id)
    }

    fn convert_tools(&self, tools: &[AiTool]) -> Vec<Value> {
        self.convert_tools_impl(tools)
    }

    fn build_request_body(
        &self,
        model: &str,
        messages: Vec<Value>,
        options: &RequestOptions,
    ) -> Value {
        self.build_request_body_impl(model, messages, options)
    }

    fn endpoint_path(&self, _model: &str) -> &str {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{Content, ImageContent, Role};

    #[test]
    fn convert_messages_openai_chat_preserves_user_image_parts() {
        let format = OpenAIFormat::new(ApiFormat::OpenAI);
        let messages = vec![ModelMessage {
            role: Role::User,
            content: vec![
                Content::Text {
                    text: "Describe this".to_string(),
                },
                Content::Image {
                    image: ImageContent {
                        url: None,
                        base64: Some("AAA".to_string()),
                        media_type: Some("image/jpeg".to_string()),
                    },
                    detail: Some("high".to_string()),
                },
            ],
        }];

        let converted = format.convert_messages(&messages, None);
        assert_eq!(converted.len(), 1);
        let content = converted[0]
            .get("content")
            .and_then(|c| c.as_array())
            .expect("content should be a multimodal array");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Describe this");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/jpeg;base64,AAA");
        assert_eq!(content[1]["image_url"]["detail"], "high");
    }

    #[test]
    fn convert_messages_openai_responses_uses_input_image() {
        let format = OpenAIFormat::new(ApiFormat::OpenAIResponses);
        let messages = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::Image {
                image: ImageContent {
                    url: Some("https://example.com/cat.png".to_string()),
                    base64: None,
                    media_type: None,
                },
                detail: Some("low".to_string()),
            }],
        }];

        let converted = format.convert_messages(&messages, None);
        assert_eq!(converted.len(), 1);
        let content = converted[0]
            .get("content")
            .and_then(|c| c.as_array())
            .expect("content should be a multimodal array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "input_image");
        assert_eq!(content[0]["image_url"], "https://example.com/cat.png");
        assert_eq!(content[0]["detail"], "low");
    }
}
