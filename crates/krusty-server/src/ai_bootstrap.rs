use std::path::Path;

use anyhow::Result;
use krusty_core::ai::client::{AiClient, AiClientConfig};
use krusty_core::ai::format_detection::detect_api_format;
use krusty_core::ai::models::{resolve_model_metadata, ModelMetadata, SharedModelRegistry};
use krusty_core::ai::providers::{builtin_providers, get_provider, ProviderId};
use krusty_core::constants;
use krusty_core::storage::credentials::{ActiveProviderStore, CredentialStore};
use krusty_core::storage::{Database, Preferences};

use crate::utils;

#[derive(Debug, Clone, Copy, Default)]
struct AiBootstrapPolicy;

impl AiBootstrapPolicy {
    fn explicit_provider_from_env(self) -> Option<ProviderId> {
        std::env::var("KRUSTY_PROVIDER")
            .ok()
            .as_deref()
            .and_then(utils::providers::parse_provider)
    }

    fn explicit_model_from_env(self) -> Option<String> {
        std::env::var("KRUSTY_MODEL")
            .ok()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
    }

    fn current_model_preference(self, db_path: &Path, user_id: Option<&str>) -> Option<String> {
        load_current_model_preference(db_path, user_id)
    }

    fn credential_env_key(self, provider: ProviderId) -> &'static str {
        match provider {
            ProviderId::MiniMax => "MINIMAX_API_KEY",
            ProviderId::OpenRouter => "OPENROUTER_API_KEY",
            ProviderId::ZAi => "Z_AI_API_KEY",
            ProviderId::Anthropic => "ANTHROPIC_API_KEY",
            ProviderId::OpenAI => "OPENAI_API_KEY",
            ProviderId::Grok => "GROK_ACCESS_TOKEN",
        }
    }

    async fn provider_for_model(
        self,
        model_registry: &SharedModelRegistry,
        model: &str,
    ) -> Option<ProviderId> {
        if let Some(metadata) = model_registry.get_model(model).await {
            return Some(metadata.provider);
        }

        builtin_providers()
            .iter()
            .filter(|provider| !provider.dynamic_models)
            .find_map(|provider| {
                provider
                    .models
                    .iter()
                    .any(|candidate| candidate.id.eq_ignore_ascii_case(model))
                    .then_some(provider.id)
            })
    }

    async fn resolve_model_selection(
        self,
        model_registry: &SharedModelRegistry,
        db_path: &Path,
        requested_model: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<(ProviderId, String)> {
        let requested_model = requested_model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(ToOwned::to_owned);
        let env_provider = self.explicit_provider_from_env();
        let model = requested_model
            .or_else(|| self.explicit_model_from_env())
            .or_else(|| self.current_model_preference(db_path, user_id))?;
        let provider = self
            .provider_for_model(model_registry, &model)
            .await
            .or(env_provider)?;
        Some((provider, model))
    }
}

pub(crate) fn load_current_model_preference(
    db_path: &Path,
    user_id: Option<&str>,
) -> Option<String> {
    let db = Database::new(db_path).ok()?;
    let prefs = if let Some(user_id) = user_id {
        Preferences::for_user(db, user_id)
    } else {
        Preferences::new(db)
    };

    prefs
        .get_current_model()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

pub(crate) fn resolve_preferred_model(db_path: &Path, user_id: Option<&str>) -> Option<String> {
    let policy = AiBootstrapPolicy;
    policy
        .explicit_model_from_env()
        .or_else(|| policy.current_model_preference(db_path, user_id))
}

pub(crate) async fn persist_current_model_selection(
    model_registry: &SharedModelRegistry,
    db_path: &Path,
    user_id: Option<&str>,
    model_id: &str,
) -> Result<()> {
    persist_current_model_preference(db_path, user_id, model_id)?;

    if let Some(provider) = AiBootstrapPolicy
        .provider_for_model(model_registry, model_id)
        .await
    {
        ActiveProviderStore::save(provider)?;
    }

    Ok(())
}

pub(crate) fn persist_current_model_preference(
    db_path: &Path,
    user_id: Option<&str>,
    model_id: &str,
) -> Result<()> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Ok(());
    }

    let db = Database::new(db_path)?;
    if let Some(user_id) = user_id {
        Preferences::for_user(db, user_id).set_current_model(model_id)
    } else {
        Preferences::new(db).set_current_model(model_id)
    }
}

pub(crate) fn clear_current_model_preference(db_path: &Path, user_id: Option<&str>) -> Result<()> {
    let db = Database::new(db_path)?;
    if let Some(user_id) = user_id {
        Preferences::for_user(db, user_id).delete("current_model")
    } else {
        Preferences::new(db).delete("current_model")
    }
}

/// Build a bootstrap AI client for server-owned startup paths.
///
/// This prefers the explicit/persisted current model. When none exists yet,
/// it falls back to the first authenticated provider's curated default so the
/// server can still register safety/runtime hooks. User-facing routes should
/// resolve through `create_ai_client_for_model` instead.
pub async fn create_ai_client(
    credentials: &CredentialStore,
    model_registry: &SharedModelRegistry,
    db_path: &Path,
) -> Option<AiClient> {
    let policy = AiBootstrapPolicy;
    if let Some(client) =
        create_ai_client_for_model(credentials, model_registry, db_path, None, None).await
    {
        return Some(client);
    }

    if let Some(provider) = policy.explicit_provider_from_env() {
        let model = get_provider(provider)?.default_model().to_string();
        if let Some(client) = create_ai_client_for_provider(credentials, provider, model) {
            return Some(client);
        }
    }

    let provider = credentials.providers_with_auth().into_iter().next()?;
    let model = get_provider(provider)?.default_model().to_string();
    create_ai_client_for_provider(credentials, provider, model)
}

pub async fn create_ai_client_for_model(
    credentials: &CredentialStore,
    model_registry: &SharedModelRegistry,
    db_path: &Path,
    requested_model: Option<&str>,
    user_id: Option<&str>,
) -> Option<AiClient> {
    let (provider, model) = AiBootstrapPolicy
        .resolve_model_selection(model_registry, db_path, requested_model, user_id)
        .await?;

    create_ai_client_for_provider(credentials, provider, model)
}

fn create_ai_client_for_provider(
    credentials: &CredentialStore,
    provider: ProviderId,
    model: String,
) -> Option<AiClient> {
    let policy = AiBootstrapPolicy;
    let auth = if provider == ProviderId::Grok {
        credentials.get_auth(&provider)
    } else {
        credentials
            .get_auth(&provider)
            .or_else(|| std::env::var(policy.credential_env_key(provider)).ok())
    };

    let provider_cfg = get_provider(provider)?;

    let (config, api_key) = match provider {
        ProviderId::OpenAI => {
            let config = AiClientConfig::for_openai_with_auth_detection(&model, credentials);
            let resolved = krusty_core::auth::resolve_openai_auth(credentials, &model);

            let auth = resolved
                .credential
                .or_else(|| std::env::var("OPENAI_API_KEY").ok());
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No OpenAI credentials found for resolved auth mode ({:?}); chat API unavailable",
                        resolved.auth_type
                    );
                    return None;
                }
            };
            (config, api_key)
        }
        ProviderId::Anthropic => {
            let config = AiClientConfig::for_anthropic_with_auth_detection(&model, credentials);
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No credentials found for provider {}; chat API will be unavailable until credentials are configured",
                        provider
                    );
                    return None;
                }
            };
            (config, api_key)
        }
        ProviderId::Grok => {
            let config = AiClientConfig::for_grok(&model);
            let resolved = krusty_core::auth::resolve_grok_auth(credentials);
            let api_key = match resolved.credential {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No Grok X-subscription credentials found; run Grok OAuth or `grok login`"
                    );
                    return None;
                }
            };
            (config, api_key)
        }
        _ => {
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No credentials found for provider {}; chat API will be unavailable until credentials are configured",
                        provider
                    );
                    return None;
                }
            };
            (
                AiClientConfig {
                    model,
                    max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
                    base_url: Some(provider_cfg.base_url.clone()),
                    auth_header: provider_cfg.auth_header,
                    provider_id: provider,
                    api_format: Default::default(),
                    custom_headers: provider_cfg.custom_headers.clone(),
                },
                api_key,
            )
        }
    };

    Some(AiClient::new(config, api_key))
}

/// Initialize models in the shared registry.
pub async fn initialize_models(registry: &SharedModelRegistry, credentials: &CredentialStore) {
    for provider in builtin_providers() {
        let models: Vec<ModelMetadata> = provider
            .models
            .iter()
            .map(|m| {
                let api_format = detect_api_format(provider.id, &m.id);
                let mut model = ModelMetadata::new(&m.id, &m.display_name, provider.id)
                    .with_context(m.context_window, m.max_output);
                let inferred = resolve_model_metadata(provider.id, &m.id, api_format);

                if let Some(reasoning) = m.reasoning {
                    model = model.with_thinking(reasoning);
                }

                model.supports_tools = provider.supports_tools;
                model.supports_vision = inferred.supports_vision;
                model.api_format = inferred.api_format;
                model
            })
            .collect();

        registry.set_models(provider.id, models).await;
    }

    for provider in [ProviderId::OpenRouter, ProviderId::OpenAI, ProviderId::Grok] {
        let Some(credential) =
            krusty_core::ai::catalog::credential_for_dynamic_models(provider, credentials)
        else {
            tracing::debug!(
                "Skipping {:?} dynamic model refresh: no catalog API key configured",
                provider
            );
            continue;
        };

        match krusty_core::ai::catalog::fetch_dynamic_models(provider, &credential).await {
            Ok(models) => {
                tracing::info!("Fetched {} {:?} models", models.len(), provider);
                registry.set_models(provider, models).await;
            }
            Err(e) => tracing::warn!("Failed to fetch {:?} models: {}", provider, e),
        }
    }
}
