use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use krusty_core::ai::catalog::{CatalogAuthKind, CatalogCredential};
use krusty_core::ai::client::{AiClient, AiClientConfig};
use krusty_core::ai::format_detection::detect_api_format;
use krusty_core::ai::models::{
    resolve_model_metadata, ModelAuthScope, ModelMetadata, SharedModelRegistry,
};
use krusty_core::ai::providers::{builtin_providers, get_provider, ProviderId};
use krusty_core::auth::{OpenAIAuthResolution, OpenAIAuthType};
use krusty_core::constants;
use krusty_core::storage::credentials::{ActiveProviderStore, CredentialStore};
use krusty_core::storage::{Database, Preferences};
use tokio::sync::{Mutex, RwLock};

use crate::utils;

struct CatalogRefreshSlot {
    fetch_lock: Mutex<()>,
    commit_lock: Mutex<()>,
    auth_generation: AtomicU64,
}

impl CatalogRefreshSlot {
    fn new() -> Self {
        Self {
            fetch_lock: Mutex::new(()),
            commit_lock: Mutex::new(()),
            auth_generation: AtomicU64::new(0),
        }
    }
}

static CATALOG_REFRESH_SLOTS: LazyLock<HashMap<ProviderId, CatalogRefreshSlot>> =
    LazyLock::new(|| {
        ProviderId::all()
            .iter()
            .copied()
            .map(|provider| (provider, CatalogRefreshSlot::new()))
            .collect()
    });

fn catalog_refresh_slot(provider: ProviderId) -> &'static CatalogRefreshSlot {
    CATALOG_REFRESH_SLOTS
        .get(&provider)
        .expect("every provider has a catalog refresh slot")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogRefreshOutcome {
    Refreshed(usize),
    SkippedFresh,
    SkippedNoCredentials,
    Superseded,
}

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

    let auth_scope = model_registry
        .get_model(&model)
        .await
        .and_then(|metadata| metadata.auth_scope);

    create_ai_client_for_provider_with_scope(credentials, provider, model, auth_scope)
}

fn create_ai_client_for_provider(
    credentials: &CredentialStore,
    provider: ProviderId,
    model: String,
) -> Option<AiClient> {
    create_ai_client_for_provider_with_scope(credentials, provider, model, None)
}

fn create_ai_client_for_provider_with_scope(
    credentials: &CredentialStore,
    provider: ProviderId,
    model: String,
    auth_scope: Option<ModelAuthScope>,
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
            let resolved = resolve_openai_auth_for_model(credentials, &model, auth_scope);

            let auth = resolved
                .as_ref()
                .and_then(|resolution| resolution.credential.clone());
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        ?auth_scope,
                        "No OpenAI credential matches the selected model catalog; chat API unavailable"
                    );
                    return None;
                }
            };
            let config = AiClientConfig::for_openai_with_auth_resolution(
                &model,
                resolved.expect("credential checked above"),
            );
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
            let api_format = detect_api_format(provider, &model);
            (
                AiClientConfig {
                    model,
                    max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
                    base_url: Some(provider_cfg.base_url.clone()),
                    auth_header: provider_cfg.auth_header,
                    provider_id: provider,
                    api_format,
                    custom_headers: provider_cfg.custom_headers.clone(),
                },
                api_key,
            )
        }
    };

    Some(AiClient::new(config, api_key))
}

fn resolve_openai_auth_for_model(
    credentials: &CredentialStore,
    model: &str,
    auth_scope: Option<ModelAuthScope>,
) -> Option<OpenAIAuthResolution> {
    let Some(scope) = auth_scope else {
        let resolved = krusty_core::auth::resolve_openai_auth(credentials, model);
        return resolved.credential.is_some().then_some(resolved);
    };

    let desired = match scope {
        ModelAuthScope::ApiKey => CatalogAuthKind::ApiKey,
        ModelAuthScope::OAuth => CatalogAuthKind::OAuth,
    };
    let identity =
        krusty_core::ai::catalog::credentials_for_dynamic_models(ProviderId::OpenAI, credentials)
            .into_iter()
            .find(|identity| identity.kind == desired)?;

    Some(openai_resolution_for_catalog_identity(identity))
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

fn curated_model_metadata(provider: ProviderId) -> Vec<ModelMetadata> {
    let Some(config) = get_provider(provider) else {
        return Vec::new();
    };

    config
        .models
        .iter()
        .map(|model_info| {
            let api_format = detect_api_format(provider, &model_info.id);
            let inferred = resolve_model_metadata(provider, &model_info.id, api_format);
            let mut model = ModelMetadata::new(&model_info.id, &model_info.display_name, provider)
                .with_context(model_info.context_window, model_info.max_output);
            if let Some(reasoning) = model_info.reasoning {
                model = model.with_thinking(reasoning);
            }
            model.supported_reasoning_levels = model_info.supported_reasoning_levels.clone();
            model.default_reasoning_level = model_info.default_reasoning_level;
            model.reasoning_is_mandatory = model_info.reasoning_is_mandatory;
            model.reasoning_control = model_info.reasoning_control;
            model.fast_mode = model_info.fast_mode;
            model.supports_tools = config.supports_tools;
            model.supports_vision = inferred.supports_vision;
            model.api_format = inferred.api_format;
            model
        })
        .collect()
}

async fn install_curated_catalog(
    registry: &SharedModelRegistry,
    db_path: &Path,
    provider: ProviderId,
) {
    registry
        .set_models(provider, curated_model_metadata(provider))
        .await;
    if let Ok(db) = Database::new(db_path) {
        let preferences = Preferences::new(db);
        for custom in preferences.get_custom_models(provider) {
            registry.upsert_model(custom).await;
        }
    }
}

/// Initialize static fallbacks and restore durable last-known-good snapshots.
/// Network discovery is intentionally excluded so router startup never waits on
/// a provider. `spawn_model_catalog_refresh` performs an immediate forced sweep.
pub async fn initialize_models(registry: &SharedModelRegistry, db_path: &Path) {
    for provider in builtin_providers() {
        registry
            .set_models(provider.id, curated_model_metadata(provider.id))
            .await;
    }

    if let Ok(db) = Database::new(db_path) {
        let preferences = Preferences::new(db);
        for provider in krusty_core::ai::catalog::dynamic_model_providers() {
            if let Some(models) = preferences.get_cached_models(provider) {
                registry.set_models(provider, models).await;
            }
            for custom in preferences.get_custom_models(provider) {
                registry.upsert_model(custom).await;
            }
        }
    }
}

/// Invalidate entitlement-scoped state after any credential mutation.
///
/// Generation changes happen before local reset so an older in-flight fetch is
/// guaranteed to discard its result. The commit lock closes the small race
/// between a fetch's final generation check and its registry/cache writes.
pub(crate) async fn invalidate_provider_model_catalog(
    registry: &SharedModelRegistry,
    db_path: &Path,
    provider: ProviderId,
) -> Result<()> {
    let slot = catalog_refresh_slot(provider);
    slot.auth_generation.fetch_add(1, Ordering::AcqRel);
    let _commit_guard = slot.commit_lock.lock().await;

    let clear_result = Database::new(db_path)
        .map(Preferences::new)
        .and_then(|preferences| preferences.clear_model_cache(provider));
    install_curated_catalog(registry, db_path, provider).await;
    clear_result
}

/// Canonical provider refresh. Every successful result is persisted before it
/// becomes visible, and custom model overlays are always restored afterward.
pub(crate) async fn refresh_provider_model_catalog(
    registry: &SharedModelRegistry,
    credentials: &Arc<RwLock<CredentialStore>>,
    db_path: &Path,
    provider: ProviderId,
    stale_only: bool,
) -> Result<CatalogRefreshOutcome> {
    let slot = catalog_refresh_slot(provider);
    let _fetch_guard = slot.fetch_lock.lock().await;

    if stale_only {
        let fresh = Database::new(db_path)
            .ok()
            .map(Preferences::new)
            .is_some_and(|preferences| !preferences.is_model_cache_stale(provider));
        if fresh {
            return Ok(CatalogRefreshOutcome::SkippedFresh);
        }
    }

    let generation = slot.auth_generation.load(Ordering::Acquire);
    let credential_snapshot = credentials.read().await.clone();
    if krusty_core::ai::catalog::credentials_for_dynamic_models(provider, &credential_snapshot)
        .is_empty()
    {
        // Cached catalogs are entitlement-scoped. Credentials can disappear
        // outside the HTTP mutation routes (for example between restarts), so
        // a forced startup sweep must retire the previous account's snapshot.
        let _commit_guard = slot.commit_lock.lock().await;
        if slot.auth_generation.load(Ordering::Acquire) != generation {
            return Ok(CatalogRefreshOutcome::Superseded);
        }
        let clear_result = Database::new(db_path)
            .map(Preferences::new)
            .and_then(|preferences| preferences.clear_model_cache(provider));
        install_curated_catalog(registry, db_path, provider).await;
        clear_result?;
        return Ok(CatalogRefreshOutcome::SkippedNoCredentials);
    }

    let models =
        krusty_core::ai::catalog::fetch_dynamic_models_for_store(provider, &credential_snapshot)
            .await?;

    let _commit_guard = slot.commit_lock.lock().await;
    if slot.auth_generation.load(Ordering::Acquire) != generation {
        return Ok(CatalogRefreshOutcome::Superseded);
    }

    let preferences = Preferences::new(Database::new(db_path)?);
    preferences.cache_models(provider, &models)?;
    let custom_models = preferences.get_custom_models(provider);
    registry.set_models(provider, models.clone()).await;
    for custom in custom_models {
        registry.upsert_model(custom).await;
    }

    Ok(CatalogRefreshOutcome::Refreshed(models.len()))
}

pub(crate) fn spawn_provider_model_catalog_refresh(
    registry: SharedModelRegistry,
    credentials: Arc<RwLock<CredentialStore>>,
    db_path: Arc<PathBuf>,
    provider: ProviderId,
    stale_only: bool,
) {
    tokio::spawn(async move {
        match refresh_provider_model_catalog(
            &registry,
            &credentials,
            db_path.as_path(),
            provider,
            stale_only,
        )
        .await
        {
            Ok(CatalogRefreshOutcome::Refreshed(count)) => {
                tracing::info!(?provider, count, "Refreshed model catalog");
            }
            Ok(CatalogRefreshOutcome::Superseded) => {
                tracing::debug!(?provider, "Discarded superseded model catalog refresh");
            }
            Ok(CatalogRefreshOutcome::SkippedFresh)
            | Ok(CatalogRefreshOutcome::SkippedNoCredentials) => {}
            Err(error) => tracing::warn!(?provider, %error, "Failed to refresh model catalog"),
        }
    });
}

async fn refresh_model_catalogs(
    registry: &SharedModelRegistry,
    credentials: &Arc<RwLock<CredentialStore>>,
    db_path: &Path,
    stale_only: bool,
) {
    let mut refreshes = tokio::task::JoinSet::new();
    for provider in krusty_core::ai::catalog::dynamic_model_providers() {
        let registry = registry.clone();
        let credentials = credentials.clone();
        let db_path = db_path.to_path_buf();
        refreshes.spawn(async move {
            (
                provider,
                refresh_provider_model_catalog(
                    &registry,
                    &credentials,
                    &db_path,
                    provider,
                    stale_only,
                )
                .await,
            )
        });
    }

    while let Some(result) = refreshes.join_next().await {
        match result {
            Ok((provider, Ok(CatalogRefreshOutcome::Refreshed(count)))) => {
                tracing::info!(?provider, count, "Refreshed stale model catalog");
            }
            Ok((provider, Ok(CatalogRefreshOutcome::Superseded))) => {
                tracing::debug!(?provider, "Discarded superseded model catalog refresh");
            }
            Ok((_, Ok(CatalogRefreshOutcome::SkippedFresh)))
            | Ok((_, Ok(CatalogRefreshOutcome::SkippedNoCredentials))) => {}
            Ok((provider, Err(error))) => {
                tracing::warn!(?provider, %error, "Failed to refresh stale model catalog");
            }
            Err(error) => tracing::warn!(%error, "Model catalog refresh task failed to join"),
        }
    }
}

/// Keep server-owned catalogs current without delaying request startup. The
/// first sweep always revalidates account-scoped snapshots so credentials that
/// changed outside server routes cannot reuse another account's fresh cache.
/// Later sweeps only issue network requests after the provider TTL expires.
pub fn spawn_model_catalog_refresh(
    registry: SharedModelRegistry,
    credentials: Arc<RwLock<CredentialStore>>,
    db_path: Arc<PathBuf>,
) {
    tokio::spawn(async move {
        refresh_model_catalogs(&registry, &credentials, db_path.as_path(), false).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
        // Consume Tokio's immediate first tick: the forced startup sweep above
        // already performed the initial network revalidation.
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh_model_catalogs(&registry, &credentials, db_path.as_path(), true).await;
        }
    });
}

#[cfg(test)]
mod catalog_refresh_tests {
    use super::{openai_resolution_for_catalog_identity, CatalogCredential, OpenAIAuthType};

    #[test]
    fn catalog_identity_selects_matching_openai_transport() {
        let api = openai_resolution_for_catalog_identity(CatalogCredential::api_key(
            "sk-api".to_string(),
        ));
        assert_eq!(api.auth_type, OpenAIAuthType::ApiKey);
        assert_eq!(api.credential.as_deref(), Some("sk-api"));

        let oauth = openai_resolution_for_catalog_identity(CatalogCredential::oauth(
            "oauth-token".to_string(),
            Some("account-1".to_string()),
        ));
        assert_eq!(oauth.auth_type, OpenAIAuthType::ChatGptOAuth);
        assert_eq!(oauth.account_id.as_deref(), Some("account-1"));
    }
}
