use anyhow::Result;
use reqwest::Client;
use tracing::{debug, info};

use crate::ai::models::ModelMetadata;

use super::mapping::{is_useful_model, parse_model};
use super::types::ModelsResponse;

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// Fetch all available models from OpenRouter.
///
/// Optionally accepts a shared HTTP client to avoid creating new connections.
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(api_key, None).await
}

/// Fetch models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    api_key: &str,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    let owned_client;
    let client = match client {
        Some(c) => c,
        None => {
            owned_client = Client::new();
            &owned_client
        }
    };

    info!("Fetching models from OpenRouter...");

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header(
            "HTTP-Referer",
            "https://github.com/honeycomb-Technologies/Mitsuro",
        )
        .send()
        .await?;

    if !response.status().is_success() {
        let error = crate::ai::retry::provider_http_error(response, "OpenRouter API error").await;
        error.log();
        return Err(error.into());
    }

    let data: ModelsResponse = response.json().await?;
    info!("OpenRouter returned {} models", data.data.len());

    let models: Vec<ModelMetadata> = data
        .data
        .into_iter()
        .filter(|m| is_useful_model(&m.id))
        .map(parse_model)
        .filter(|m| m.supports_tools)
        .collect();

    debug!("Filtered to {} models with tool support", models.len());
    Ok(models)
}
