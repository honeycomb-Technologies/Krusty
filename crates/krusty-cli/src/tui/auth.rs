//! Authentication helpers for the TUI
//!
//! Extracted auth logic to reduce app.rs complexity.

use crate::ai::client::AiClientConfig;
use crate::ai::format_detection::detect_api_format;
use crate::ai::models::SharedModelRegistry;
use crate::ai::providers::{get_provider, translate_model_id, ProviderId};
use crate::storage::CredentialStore;

/// Create AiClientConfig for a provider/model combination
///
/// Handles special cases:
/// - OpenAI: OAuth vs API key detection for endpoint routing
/// - Others: format detection based on provider and model
pub fn create_client_config(
    provider: ProviderId,
    model: &str,
    credential_store: &CredentialStore,
    _model_registry: &SharedModelRegistry,
) -> AiClientConfig {
    // Anthropic requires special handling to detect OAuth vs API key
    // and set the correct auth header (Bearer for OAuth, x-api-key for API key)
    if provider == ProviderId::Anthropic {
        return AiClientConfig::for_anthropic_with_auth_detection(model, credential_store);
    }

    // OpenAI requires special handling to detect OAuth vs API key
    // and route to the correct endpoint (ChatGPT Responses API vs OpenAI Chat Completions)
    if provider == ProviderId::OpenAI {
        return AiClientConfig::for_openai_with_auth_detection(model, credential_store);
    }

    // Grok uses the Grok CLI proxy with model-specific OpenAI-compatible routing.
    if provider == ProviderId::Grok {
        return AiClientConfig::for_grok(model);
    }

    let provider_config = match get_provider(provider) {
        Some(config) => config,
        None => {
            tracing::warn!("Provider {:?} not found, falling back to MiniMax", provider);
            match get_provider(ProviderId::MiniMax) {
                Some(config) => config,
                None => {
                    tracing::error!(
                        "MiniMax fallback provider not available, using default config"
                    );
                    return AiClientConfig {
                        model: model.to_string(),
                        ..AiClientConfig::default()
                    };
                }
            }
        }
    };

    let api_format = detect_api_format(provider, model);

    AiClientConfig {
        model: model.to_string(),
        max_tokens: crate::constants::ai::MAX_OUTPUT_TOKENS,
        base_url: Some(provider_config.base_url.clone()),
        auth_header: provider_config.auth_header,
        provider_id: provider_config.id,
        api_format,
        custom_headers: provider_config.custom_headers.clone(),
    }
}

pub fn infer_provider_for_model(
    model_registry: &SharedModelRegistry,
    model: &str,
) -> Option<ProviderId> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    if let Some(metadata) = model_registry.try_get_model(model) {
        return Some(metadata.provider);
    }

    ProviderId::all().iter().find_map(|provider| {
        get_provider(*provider)
            .filter(|config| config.has_model(model))
            .map(|config| config.id)
    })
}

/// Translate a selected model to an equivalent for another provider.
///
/// Returns `None` when no model is selected or when there is no equivalent model
/// for the target provider.
pub fn translate_model_for_provider(
    current_model: &str,
    from_provider: ProviderId,
    to_provider: ProviderId,
) -> Option<String> {
    let current_model = current_model.trim();
    if current_model.is_empty() {
        return None;
    }

    if from_provider == to_provider {
        return Some(current_model.to_string());
    }

    translate_model_id(current_model, from_provider, to_provider)
}
