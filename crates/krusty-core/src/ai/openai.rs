//! OpenAI model catalog integration.
//!
//! Fetches available text-generation models from OpenAI's `/v1/models` API.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, error, info};

use super::models::{ApiFormat, ModelMetadata};
use super::providers::{ProviderConfig, ProviderId, ReasoningFormat};

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
}

/// Fetch available OpenAI models for the authenticated account.
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(api_key, None).await
}

/// Fetch OpenAI models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    api_key: &str,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    let owned_client;
    let client = match client {
        Some(client) => client,
        None => {
            owned_client = Client::new();
            &owned_client
        }
    };

    info!("Fetching models from OpenAI...");
    let response = client
        .get(OPENAI_MODELS_URL)
        .bearer_auth(api_key)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!("OpenAI models API error: {} - {}", status, error_text);
        return Err(anyhow::anyhow!(
            "OpenAI models API error: {} - {}",
            status,
            error_text
        ));
    }

    let mut models: Vec<ModelMetadata> = response
        .json::<ModelsResponse>()
        .await?
        .data
        .into_iter()
        .filter(|model| is_useful_model(&model.id))
        .map(parse_model)
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    debug!("Fetched {} usable OpenAI models", models.len());
    Ok(models)
}

fn is_useful_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let normalized = id.strip_prefix("openai/").unwrap_or(&id);

    let supported_prefix = normalized.starts_with("gpt-")
        || is_openai_o_series(normalized)
        || normalized.contains("codex");

    if !supported_prefix {
        return false;
    }

    let excluded_patterns = [
        "audio",
        "transcribe",
        "tts",
        "realtime",
        "image",
        "embedding",
        "moderation",
        "search",
        "whisper",
    ];

    !excluded_patterns.iter().any(|pattern| id.contains(pattern))
}

fn parse_model(raw: OpenAiModel) -> ModelMetadata {
    let id = raw.id;
    let id_lower = id.to_ascii_lowercase();
    let normalized = id_lower.strip_prefix("openai/").unwrap_or(&id_lower);
    let prefers_responses = ProviderConfig::openai_prefers_responses_api(&id);
    let reasoning_format = if prefers_responses {
        Some(ReasoningFormat::OpenAI)
    } else {
        None
    };

    let (context_window, max_output) = if normalized.contains("codex")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5)
    {
        (400_000, 128_000)
    } else if is_openai_o_series(normalized) || normalized.contains("gpt-4.1") {
        (200_000, 100_000)
    } else {
        (128_000, 32_768)
    };

    let supports_vision = normalized.contains("gpt-4o")
        || normalized.contains("gpt-4.1")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5);

    let mut metadata = ModelMetadata::new(&id, &display_name(&id), ProviderId::OpenAI)
        .with_context(context_window, max_output);
    if let Some(format) = reasoning_format {
        metadata = metadata.with_thinking(format);
    }
    metadata.supports_tools = true;
    metadata.supports_vision = supports_vision;
    metadata.api_format = if prefers_responses {
        ApiFormat::OpenAIResponses
    } else {
        ApiFormat::OpenAI
    };
    metadata
}

fn display_name(id: &str) -> String {
    id.split('-')
        .map(|part| match part {
            "gpt" => "GPT".to_string(),
            other if is_openai_o_series(other) => other.to_ascii_uppercase(),
            "codex" => "Codex".to_string(),
            other if other.chars().all(|c| c.is_ascii_digit()) => other.to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_openai_o_series(model_id: &str) -> bool {
    model_id
        .strip_prefix('o')
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|ch| ch.is_ascii_digit())
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{display_name, is_useful_model, parse_model, OpenAiModel};
    use crate::ai::models::ApiFormat;

    #[test]
    fn filters_text_models_only() {
        assert!(is_useful_model("gpt-5-codex"));
        assert!(is_useful_model("o3"));
        assert!(is_useful_model("o5"));
        assert!(!is_useful_model("whisper-1"));
        assert!(!is_useful_model("gpt-image-1"));
    }

    #[test]
    fn builds_readable_display_name() {
        assert_eq!(display_name("gpt-5-codex"), "GPT 5 Codex");
    }

    #[test]
    fn parses_future_gpt_major_as_responses_with_large_context() {
        let metadata = parse_model(OpenAiModel {
            id: "gpt-6.4".to_string(),
        });

        assert_eq!(metadata.context_window, 400_000);
        assert_eq!(metadata.api_format, ApiFormat::OpenAIResponses);
        assert!(metadata.supports_vision);
    }
}
