use anyhow::Result;
use reqwest::Client;
use tracing::{debug, error, info};

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
        .header("HTTP-Referer", "https://github.com/BurgessTG/Krusty")
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!("OpenRouter API error: {} - {}", status, error_text);
        return Err(anyhow::anyhow!(
            "OpenRouter API error: {} - {}",
            status,
            error_text
        ));
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
