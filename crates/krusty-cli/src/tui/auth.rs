//! Authentication helpers for the TUI
//!
//! Extracted auth logic to reduce app.rs complexity.

use crate::ai::client::AiClientConfig;
use crate::ai::format_detection::detect_api_format;
use crate::ai::models::SharedModelRegistry;
use crate::ai::providers::{get_provider, translate_model_or_default, ProviderId};
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
    // OpenAI requires special handling to detect OAuth vs API key
    // and route to the correct endpoint (ChatGPT Responses API vs OpenAI Chat Completions)
    if provider == ProviderId::OpenAI {
        return AiClientConfig::for_openai_with_auth_detection(model, credential_store);
    }

    let provider_config = match get_provider(provider) {
        Some(config) => config,
        None => {
            tracing::warn!("Provider {:?} not found, falling back to MiniMax", provider);
            get_provider(ProviderId::MiniMax).expect("MiniMax provider must be available")
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

/// Translate model ID when switching providers and validate it exists
///
/// Returns (translated_model, changed) where changed indicates if translation occurred
pub fn translate_model_for_provider(
    current_model: &str,
    from_provider: ProviderId,
    to_provider: ProviderId,
) -> (String, bool) {
    let translated = translate_model_or_default(current_model, from_provider, to_provider);
    let changed = translated != current_model;

    if changed {
        tracing::info!(
            "Translated model '{}' -> '{}' for {}",
            current_model,
            translated,
            to_provider
        );
    }

    (translated, changed)
}

/// Validate model exists for provider, returning default if not
///
/// Returns (validated_model, was_fallback)
pub fn validate_model_for_provider(model: &str, provider: ProviderId) -> (String, bool) {
    if let Some(provider_config) = get_provider(provider) {
        if !provider_config.has_model(model) {
            let default = provider_config.default_model().to_string();
            tracing::info!(
                "Model '{}' not available for {}, using default '{}'",
                model,
                provider,
                default
            );
            return (default, true);
        }
    }
    (model.to_string(), false)
}
