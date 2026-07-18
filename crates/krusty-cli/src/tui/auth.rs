//! Authentication helpers for the TUI
//!
//! Extracted auth logic to reduce app.rs complexity.

use crate::ai::client::AiClientConfig;
use crate::ai::format_detection::detect_api_format;
use crate::ai::models::{ModelAuthScope, SharedModelRegistry};
use crate::ai::providers::{get_provider, translate_model_id, ProviderId};
use crate::storage::CredentialStore;
use krusty_core::ai::catalog::{
    credentials_for_dynamic_models, CatalogAuthKind, CatalogCredential,
};
use krusty_core::auth::{OpenAIAuthResolution, OpenAIAuthType};

/// Create AiClientConfig for a provider/model combination
///
/// Handles special cases:
/// - OpenAI: OAuth vs API key detection for endpoint routing
/// - Others: format detection based on provider and model
pub fn create_client_config(
    provider: ProviderId,
    model: &str,
    credential_store: &CredentialStore,
    model_registry: &SharedModelRegistry,
) -> AiClientConfig {
    // Anthropic requires special handling to detect OAuth vs API key
    // and set the correct auth header (Bearer for OAuth, x-api-key for API key)
    if provider == ProviderId::Anthropic {
        return AiClientConfig::for_anthropic_with_auth_detection(model, credential_store);
    }

    // OpenAI requires special handling to detect OAuth vs API key
    // and route to the correct endpoint (ChatGPT Responses API vs OpenAI Chat Completions)
    if provider == ProviderId::OpenAI {
        let resolution = resolve_openai_auth_for_model(model, credential_store, model_registry);
        return AiClientConfig::for_openai_with_auth_resolution(model, resolution);
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

/// Resolve the credential surface that advertised the selected OpenAI model.
///
/// API-key and ChatGPT OAuth catalogs can contain the same model ID with
/// different request transports. When live metadata has provenance, fail
/// closed if that exact credential surface is no longer available instead of
/// silently routing the model through the other account.
pub fn resolve_openai_auth_for_model(
    model: &str,
    credential_store: &CredentialStore,
    model_registry: &SharedModelRegistry,
) -> OpenAIAuthResolution {
    let auth_scope = model_registry
        .try_get_model(model)
        .and_then(|metadata| metadata.auth_scope);

    let Some(auth_scope) = auth_scope else {
        return krusty_core::auth::resolve_openai_auth(credential_store, model);
    };

    resolve_scoped_openai_auth(
        auth_scope,
        credentials_for_dynamic_models(ProviderId::OpenAI, credential_store),
    )
}

fn resolve_scoped_openai_auth(
    auth_scope: ModelAuthScope,
    identities: Vec<CatalogCredential>,
) -> OpenAIAuthResolution {
    let desired = match auth_scope {
        ModelAuthScope::ApiKey => CatalogAuthKind::ApiKey,
        ModelAuthScope::OAuth => CatalogAuthKind::OAuth,
    };

    identities
        .into_iter()
        .find(|identity| identity.kind == desired)
        .map(openai_resolution_for_catalog_identity)
        .unwrap_or(OpenAIAuthResolution {
            auth_type: OpenAIAuthType::None,
            credential: None,
            account_id: None,
        })
}

fn openai_resolution_for_catalog_identity(identity: CatalogCredential) -> OpenAIAuthResolution {
    OpenAIAuthResolution {
        auth_type: match identity.kind {
            CatalogAuthKind::ApiKey => OpenAIAuthType::ApiKey,
            CatalogAuthKind::OAuth => OpenAIAuthType::ChatGptOAuth,
        },
        credential: Some(identity.credential().to_string()),
        account_id: identity.account_id,
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

#[cfg(test)]
mod tests {
    use super::resolve_scoped_openai_auth;
    use crate::ai::models::ModelAuthScope;
    use krusty_core::ai::catalog::CatalogCredential;
    use krusty_core::auth::OpenAIAuthType;

    #[test]
    fn scoped_openai_auth_uses_the_exact_catalog_identity() {
        let identities = vec![
            CatalogCredential::api_key("api-secret".to_string()),
            CatalogCredential::oauth("oauth-secret".to_string(), Some("account-id".to_string())),
        ];

        let api_key = resolve_scoped_openai_auth(ModelAuthScope::ApiKey, identities.clone());
        assert_eq!(api_key.auth_type, OpenAIAuthType::ApiKey);
        assert_eq!(api_key.credential.as_deref(), Some("api-secret"));
        assert_eq!(api_key.account_id, None);

        let oauth = resolve_scoped_openai_auth(ModelAuthScope::OAuth, identities);
        assert_eq!(oauth.auth_type, OpenAIAuthType::ChatGptOAuth);
        assert_eq!(oauth.credential.as_deref(), Some("oauth-secret"));
        assert_eq!(oauth.account_id.as_deref(), Some("account-id"));
    }

    #[test]
    fn scoped_openai_auth_does_not_fall_back_to_another_surface() {
        let resolution = resolve_scoped_openai_auth(
            ModelAuthScope::OAuth,
            vec![CatalogCredential::api_key("api-secret".to_string())],
        );

        assert_eq!(resolution.auth_type, OpenAIAuthType::None);
        assert!(resolution.credential.is_none());
        assert!(resolution.account_id.is_none());
    }
}
