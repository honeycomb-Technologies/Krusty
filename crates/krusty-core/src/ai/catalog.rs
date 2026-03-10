//! Shared provider model catalog helpers.

use anyhow::Result;

use super::models::ModelMetadata;
use super::providers::{get_provider, ProviderId};

/// Whether the provider supports runtime model discovery.
pub fn supports_dynamic_models(provider: ProviderId) -> bool {
    get_provider(provider)
        .map(|config| config.dynamic_models)
        .unwrap_or(false)
}

/// Fetch runtime model metadata for a provider.
pub async fn fetch_dynamic_models(
    provider: ProviderId,
    credential: &str,
) -> Result<Vec<ModelMetadata>> {
    match provider {
        ProviderId::OpenRouter => super::openrouter::fetch_models(credential).await,
        ProviderId::OpenAI => super::openai::fetch_models(credential).await,
        _ => anyhow::bail!(
            "Provider {:?} does not support dynamic model discovery",
            provider
        ),
    }
}
