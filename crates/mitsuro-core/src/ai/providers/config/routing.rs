use crate::ai::models::ApiFormat;
use crate::auth::{AnthropicAuthType, OpenAIAuthType};

use super::types::{
    AuthHeader, ProviderConfig, CHATGPT_RESPONSES_API, OPENAI_CHAT_API, OPENAI_RESPONSES_API,
};

impl ProviderConfig {
    /// Whether the model should prefer the Responses API for best behavior.
    pub fn openai_prefers_responses_api(model: &str) -> bool {
        let model = model.trim().to_ascii_lowercase();
        let normalized = model.strip_prefix("openai/").unwrap_or(&model);

        normalized.contains("codex")
            || is_openai_o_series(normalized)
            || normalized
                .strip_prefix("gpt-")
                .and_then(|suffix| {
                    suffix
                        .split(|ch: char| !ch.is_ascii_digit())
                        .find(|segment| !segment.is_empty())
                })
                .and_then(|major| major.parse::<u32>().ok())
                .is_some_and(|major| major >= 5)
    }

    /// Get the API base URL for OpenAI based on auth type + model.
    pub fn openai_url_for_auth(model: &str, auth_type: OpenAIAuthType) -> &'static str {
        match auth_type {
            OpenAIAuthType::ChatGptOAuth => CHATGPT_RESPONSES_API,
            OpenAIAuthType::ApiKey | OpenAIAuthType::None => {
                if Self::openai_prefers_responses_api(model) {
                    OPENAI_RESPONSES_API
                } else {
                    OPENAI_CHAT_API
                }
            }
        }
    }

    /// Get the API format for OpenAI based on auth type + model.
    pub fn openai_format_for_auth(model: &str, auth_type: OpenAIAuthType) -> ApiFormat {
        match auth_type {
            OpenAIAuthType::ChatGptOAuth => ApiFormat::OpenAIResponses,
            OpenAIAuthType::ApiKey | OpenAIAuthType::None => {
                if Self::openai_prefers_responses_api(model) {
                    ApiFormat::OpenAIResponses
                } else {
                    ApiFormat::OpenAI
                }
            }
        }
    }

    /// Get the auth header for Anthropic based on auth type.
    pub fn anthropic_auth_header_for_auth(auth_type: AnthropicAuthType) -> AuthHeader {
        match auth_type {
            AnthropicAuthType::OAuth => AuthHeader::Bearer,
            AnthropicAuthType::ApiKey | AnthropicAuthType::None => AuthHeader::XApiKey,
        }
    }
}

fn is_openai_o_series(model_id: &str) -> bool {
    model_id
        .strip_prefix('o')
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|ch| ch.is_ascii_digit())
}
