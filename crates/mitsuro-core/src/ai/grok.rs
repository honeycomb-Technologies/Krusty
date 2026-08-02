//! Grok/X subscription model catalog helpers.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info};

use crate::ai::models::{ApiFormat, ModelMetadata};
use crate::ai::providers::{ProviderId, ReasoningControl, ReasoningEffort, ReasoningFormat};

const DEFAULT_GROK_PROXY_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";

#[derive(Debug, Deserialize)]
struct GrokModelsResponse {
    data: Vec<GrokModel>,
}

#[derive(Debug, Deserialize)]
struct GrokModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    context_window: Option<usize>,
    #[serde(default)]
    max_completion_tokens: Option<usize>,
    #[serde(default)]
    api_backend: Option<String>,
    #[serde(default)]
    supports_reasoning_effort: Option<bool>,
    #[serde(default)]
    supports_reasoning_efforts: Option<bool>,
}

/// Fetch available Grok Build models for the authenticated X/Grok account.
pub async fn fetch_models(access_token: &str) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(access_token, None).await
}

/// Fetch Grok models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    access_token: &str,
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

    let models_url = grok_models_url();
    info!(url = %models_url, "Fetching models from Grok CLI proxy...");
    let response = client
        .get(&models_url)
        .bearer_auth(access_token)
        .header(
            "x-grok-client-version",
            std::env::var("GROK_CLIENT_VERSION")
                .unwrap_or_else(|_| grok_auth::DEFAULT_CLIENT_VERSION.to_string()),
        )
        .header("x-grok-client-identifier", "mitsuro")
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .send()
        .await?;

    if !response.status().is_success() {
        let error = crate::ai::retry::provider_http_error(response, "Grok models API error").await;
        error.log();
        return Err(error.into());
    }

    let mut models: Vec<ModelMetadata> = response
        .json::<GrokModelsResponse>()
        .await?
        .data
        .into_iter()
        .filter_map(parse_model)
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    debug!(count = models.len(), "Fetched usable Grok models");
    Ok(models)
}

fn grok_models_url() -> String {
    let base = std::env::var("GROK_CLI_CHAT_PROXY_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_GROK_PROXY_BASE_URL.to_string());
    let base = base
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .trim_end_matches("/responses")
        .trim_end_matches("/models");
    format!("{base}/models")
}

fn parse_model(model: GrokModel) -> Option<ModelMetadata> {
    let id = model.id.or(model.model)?.trim().to_string();
    if id.is_empty() {
        return None;
    }

    let api_format = match model.api_backend.as_deref() {
        Some("chat_completions") => ApiFormat::OpenAI,
        Some("messages") => ApiFormat::Anthropic,
        _ => ApiFormat::OpenAIResponses,
    };
    let context_window = model.context_window.unwrap_or_else(|| {
        crate::ai::models::resolve_context_window(ProviderId::Grok, &id, api_format)
    });
    let max_output = model
        .max_completion_tokens
        .unwrap_or_else(|| default_max_output(context_window));
    // Proxy field names are overloaded: historically they meant "emits
    // reasoning", not "accepts effort". Graded effort is decided separately.
    let advertises_reasoning = model
        .supports_reasoning_effort
        .or(model.supports_reasoning_efforts)
        .unwrap_or_else(|| is_grok_build_reasoning_model(&id));
    let graded_effort = graded_effort_for_grok_model(&id);

    let display_name = model
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| id.clone());

    let mut metadata = ModelMetadata::new(&id, &display_name, ProviderId::Grok)
        .with_context(context_window, max_output);
    metadata.api_format = api_format;
    metadata.supports_tools = true;
    metadata.supports_vision = true;
    if let Some((levels, default, mandatory)) = graded_effort {
        // Public-style Grok 4.x graded effort (low/medium/high).
        metadata = metadata
            .with_thinking(ReasoningFormat::OpenAI)
            .with_reasoning_levels(levels, Some(default), mandatory)
            .with_reasoning_control(ReasoningControl::OpenAiEffort);
    } else if advertises_reasoning {
        metadata = metadata.with_thinking(ReasoningFormat::OpenAI);
        // Classic Grok Build / Composer subscription rows: show reasoning
        // output when present, but do not send an effort selector the proxy
        // historically rejected.
        metadata.reasoning_control = Some(ReasoningControl::OutputOnly);
    }

    // Preserve the upstream description in logs/debuggable metadata indirectly by
    // keeping the display name from the catalog. ModelMetadata has no description
    // field today, so the raw description is intentionally not stored.
    let _ = model.description;

    Some(metadata)
}

/// Graded `reasoning_effort` support for Grok model IDs.
///
/// Public xAI Grok 4.5 is low/medium/high (default high, always-on).
/// Older subscription-only rows (grok-build, composer) stay output-only.
fn graded_effort_for_grok_model(
    model_id: &str,
) -> Option<(Vec<ReasoningEffort>, ReasoningEffort, bool)> {
    let normalized = model_id.trim().to_ascii_lowercase().replace('_', "-");
    if normalized.starts_with("grok-4.5")
        || normalized.starts_with("grok-4-5")
        || normalized.contains("grok-4.5")
        || normalized.contains("grok-4-5")
    {
        return Some((
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            ReasoningEffort::High,
            true,
        ));
    }
    if normalized.starts_with("grok-4.3")
        || normalized.starts_with("grok-4-3")
        || normalized.contains("grok-4.3")
        || normalized.contains("grok-4-3")
    {
        // 4.3 family can accept none on some surfaces; keep low/med/high as the
        // stable UI set and default low for agentic speed.
        return Some((
            vec![
                ReasoningEffort::None,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            ReasoningEffort::Low,
            false,
        ));
    }
    None
}

fn default_max_output(context_window: usize) -> usize {
    if context_window >= 400_000 {
        32_768
    } else {
        16_384
    }
}

fn is_grok_build_reasoning_model(model_id: &str) -> bool {
    let normalized = model_id.trim().to_ascii_lowercase();
    normalized == "grok-build" || normalized.starts_with("grok-composer-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grok_proxy_model_response() {
        let raw = GrokModel {
            id: Some("grok-build".to_string()),
            model: Some("ignored".to_string()),
            name: Some("Grok Build".to_string()),
            description: Some("Best for advanced coding tasks".to_string()),
            context_window: Some(512_000),
            max_completion_tokens: None,
            api_backend: Some("responses".to_string()),
            supports_reasoning_effort: None,
            supports_reasoning_efforts: None,
        };

        let model = parse_model(raw).expect("model should parse");

        assert_eq!(model.id, "grok-build");
        assert_eq!(model.display_name, "Grok Build");
        assert_eq!(model.provider, ProviderId::Grok);
        assert_eq!(model.context_window, 512_000);
        assert_eq!(model.api_format, ApiFormat::OpenAIResponses);
        assert_eq!(model.reasoning_format, Some(ReasoningFormat::OpenAI));
        assert_eq!(model.reasoning_control, Some(ReasoningControl::OutputOnly));
    }

    #[test]
    fn parses_composer_model_as_responses_reasoning_model() {
        let raw = GrokModel {
            id: Some("grok-composer-2.5-fast".to_string()),
            model: None,
            name: Some("Composer 2.5".to_string()),
            description: None,
            context_window: Some(200_000),
            max_completion_tokens: None,
            api_backend: Some("responses".to_string()),
            supports_reasoning_effort: None,
            supports_reasoning_efforts: None,
        };

        let model = parse_model(raw).expect("model should parse");

        assert_eq!(model.id, "grok-composer-2.5-fast");
        assert_eq!(model.display_name, "Composer 2.5");
        assert_eq!(model.context_window, 200_000);
        assert_eq!(model.api_format, ApiFormat::OpenAIResponses);
        assert!(model.supports_thinking);
        assert_eq!(model.reasoning_control, Some(ReasoningControl::OutputOnly));
    }

    #[test]
    fn parses_grok_45_with_graded_effort() {
        let raw = GrokModel {
            id: Some("grok-4.5".to_string()),
            model: None,
            name: Some("Grok 4.5".to_string()),
            description: None,
            context_window: Some(500_000),
            max_completion_tokens: Some(32_768),
            api_backend: Some("responses".to_string()),
            supports_reasoning_effort: Some(true),
            supports_reasoning_efforts: None,
        };

        let model = parse_model(raw).expect("model should parse");

        assert_eq!(model.id, "grok-4.5");
        assert!(model.supports_thinking);
        assert_eq!(
            model.reasoning_control,
            Some(ReasoningControl::OpenAiEffort)
        );
        assert_eq!(
            model.supported_reasoning_levels,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ]
        );
        assert_eq!(model.default_reasoning_level, Some(ReasoningEffort::High));
        assert!(model.reasoning_is_mandatory);
    }
}
