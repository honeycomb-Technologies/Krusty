use anyhow::Result;
use reqwest::Client;
use tracing::{debug, error, info};

use crate::ai::models::ModelMetadata;

use super::mapping::{is_useful_model, parse_model};
use super::types::ModelsResponse;

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

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
