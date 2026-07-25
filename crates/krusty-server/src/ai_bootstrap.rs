use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use krusty_core::ai::catalog::{CatalogAuthKind, CatalogCredential};
use krusty_core::ai::client::{AiClient, AiClientConfig};
use krusty_core::ai::format_detection::detect_api_format;
use krusty_core::ai::models::{
    resolve_model_metadata, ModelAuthScope, ModelCatalogSource, ModelKey, ModelLookupError,
    ModelMetadata, SharedModelRegistry,
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

    fn current_model_key_preference(
        self,
        db_path: &Path,
        user_id: Option<&str>,
    ) -> Option<ModelKey> {
        load_current_model_key_preference(db_path, user_id)
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
        match model_registry.resolve_legacy_key(model).await {
            Ok(key) => return Some(key.provider),
            Err(ModelLookupError::Ambiguous { candidates, .. }) => {
                tracing::warn!(
                    %model,
                    ?candidates,
                    "Refusing to infer an active provider for an ambiguous model slug"
                );
                return None;
            }
            Err(ModelLookupError::NotFound { .. }) => {}
        }

        // Compatibility for curated static rows that have not yet been loaded
        // into the registry. This is deliberately unreachable for ambiguity.
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
    ) -> Option<ModelKey> {
        let requested_model = requested_model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(ToOwned::to_owned);
        let env_provider = self.explicit_provider_from_env();
        if let Some(model) = requested_model.or_else(|| self.explicit_model_from_env()) {
            match model_registry.resolve_legacy_key(&model).await {
                Ok(key) => return Some(key),
                Err(ModelLookupError::Ambiguous { candidates, .. }) => {
                    let provider = env_provider?;
                    let mut matching = candidates
                        .into_iter()
                        .filter(|candidate| candidate.provider == provider);
                    let key = matching.next()?;
                    if matching.next().is_none() {
                        return Some(key);
                    }
                    tracing::warn!(
                        %model,
                        ?provider,
                        "Explicit provider still leaves multiple auth/transport model rows; exact model_key required"
                    );
                    return None;
                }
                Err(ModelLookupError::NotFound { .. }) => {}
            }
            let provider = env_provider?;
            return Some(ModelKey::new(
                provider,
                model.clone(),
                detect_api_format(provider, &model),
            ));
        }

        if let Some(key) = self.current_model_key_preference(db_path, user_id) {
            if model_registry.get_model_by_key(&key).await.is_some() {
                return Some(key);
            }
            tracing::warn!(?key, "Refusing unavailable persisted model key");
            return None;
        }

        let model = self.current_model_preference(db_path, user_id)?;
        match model_registry.resolve_legacy_key(&model).await {
            Ok(key) => Some(key),
            Err(ModelLookupError::Ambiguous { candidates, .. }) => {
                let provider = env_provider?;
                let mut matching = candidates
                    .into_iter()
                    .filter(|candidate| candidate.provider == provider);
                let key = matching.next()?;
                matching.next().is_none().then_some(key)
            }
            Err(ModelLookupError::NotFound { .. }) => env_provider.map(|provider| {
                ModelKey::new(provider, model.clone(), detect_api_format(provider, &model))
            }),
        }
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

pub(crate) fn load_current_model_key_preference(
    db_path: &Path,
    user_id: Option<&str>,
) -> Option<ModelKey> {
    let db = Database::new(db_path).ok()?;
    let prefs = if let Some(user_id) = user_id {
        Preferences::for_user(db, user_id)
    } else {
        Preferences::new(db)
    };
    prefs.get_current_model_key()
}

pub(crate) fn resolve_preferred_model(db_path: &Path, user_id: Option<&str>) -> Option<String> {
    let policy = AiBootstrapPolicy;
    policy
        .explicit_model_from_env()
        .or_else(|| policy.current_model_preference(db_path, user_id))
}

pub(crate) fn resolve_preferred_model_key(
    db_path: &Path,
    user_id: Option<&str>,
) -> Option<ModelKey> {
    AiBootstrapPolicy.current_model_key_preference(db_path, user_id)
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

pub(crate) async fn persist_current_model_key_selection(
    model_registry: &SharedModelRegistry,
    db_path: &Path,
    user_id: Option<&str>,
    key: &ModelKey,
) -> Result<()> {
    if model_registry.get_model_by_key(key).await.is_none() {
        anyhow::bail!("model key {key:?} is not available");
    }
    let db = Database::new(db_path)?;
    if let Some(user_id) = user_id {
        Preferences::for_user(db, user_id).set_current_model_key(key)?;
    } else {
        Preferences::new(db).set_current_model_key(key)?;
    }
    ActiveProviderStore::save(key.provider)?;
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
        Preferences::for_user(db, user_id).clear_current_model()
    } else {
        Preferences::new(db).clear_current_model()
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
    let key = AiBootstrapPolicy
        .resolve_model_selection(model_registry, db_path, requested_model, user_id)
        .await?;
    let metadata = model_registry.get_model_by_key(&key).await;

    create_ai_client_for_provider_with_metadata(
        credentials,
        key.provider,
        key.model_id,
        metadata.as_ref(),
    )
}

/// Build an AI client from one exact provider/auth/transport catalog identity.
///
/// Unlike the legacy slug resolver, this never guesses between providers or
/// credential surfaces. A stale or unavailable key is rejected rather than
/// silently rebound to a different catalog row.
pub async fn create_ai_client_for_key(
    credentials: &CredentialStore,
    model_registry: &SharedModelRegistry,
    key: &ModelKey,
) -> Option<AiClient> {
    let metadata = model_registry.get_model_by_key(key).await?;
    create_ai_client_for_provider_with_metadata(
        credentials,
        key.provider,
        key.model_id.clone(),
        Some(&metadata),
    )
}

fn create_ai_client_for_provider(
    credentials: &CredentialStore,
    provider: ProviderId,
    model: String,
) -> Option<AiClient> {
    create_ai_client_for_provider_with_metadata(credentials, provider, model, None)
}

fn create_ai_client_for_provider_with_metadata(
    credentials: &CredentialStore,
    provider: ProviderId,
    model: String,
    metadata: Option<&ModelMetadata>,
) -> Option<AiClient> {
    let auth_scope = metadata.and_then(|metadata| metadata.auth_scope);
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
            let config = metadata
                .map(AiClientConfig::for_grok_with_metadata)
                .unwrap_or_else(|| AiClientConfig::for_grok(&model));
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
            let api_format = metadata
                .map(|metadata| metadata.api_format)
                .unwrap_or_else(|| detect_api_format(provider, &model));
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

    let owned_metadata;
    let metadata = if let Some(metadata) = metadata {
        metadata
    } else {
        let mut resolved =
            resolve_model_metadata(config.provider_id, &config.model, config.api_format);
        if config.provider_id == ProviderId::OpenAI {
            resolved.auth_scope = Some(if config.uses_chatgpt_codex_format() {
                ModelAuthScope::OAuth
            } else {
                ModelAuthScope::ApiKey
            });
        }
        owned_metadata = resolved;
        &owned_metadata
    };

    match AiClient::new_with_resolved_model(config, api_key, metadata.resolve_runtime()) {
        Ok(client) => Some(client),
        Err(error) => {
            tracing::error!(
                provider = ?provider,
                model = %metadata.id,
                %error,
                "Refusing AI client whose transport disagrees with catalog metadata"
            );
            None
        }
    }
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
        .set_models_with_catalog(
            provider,
            curated_model_metadata(provider),
            Some(ModelCatalogSource::Curated),
            Some("bundled".to_string()),
        )
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
            .set_models_with_catalog(
                provider.id,
                curated_model_metadata(provider.id),
                Some(ModelCatalogSource::Curated),
                Some("bundled".to_string()),
            )
            .await;
    }

    if let Ok(db) = Database::new(db_path) {
        let preferences = Preferences::new(db);
        for provider in krusty_core::ai::catalog::dynamic_model_providers() {
            if let Some(models) = preferences.get_cached_models(provider) {
                let revision = preferences
                    .get_model_cache_metadata(provider)
                    .map(|metadata| format!("{:016x}", metadata.fingerprint));
                registry
                    .set_models_with_catalog(
                        provider,
                        models,
                        Some(ModelCatalogSource::CachedDynamic),
                        revision,
                    )
                    .await;
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
    let revision = format!(
        "{:016x}",
        krusty_core::ai::models::model_catalog_fingerprint(&models)
    );
    registry
        .set_models_with_catalog(
            provider,
            models.clone(),
            Some(ModelCatalogSource::LiveDynamic),
            Some(revision),
        )
        .await;
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
    use std::sync::Arc;

    use super::{
        openai_resolution_for_catalog_identity, AiBootstrapPolicy, CatalogCredential,
        OpenAIAuthType,
    };
    use krusty_core::ai::models::{
        ApiFormat, ModelAuthScope, ModelKey, ModelMetadata, ModelRegistry,
    };
    use krusty_core::ai::providers::ProviderId;
    use krusty_core::storage::{Database, Preferences};

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

    #[tokio::test]
    async fn ambiguous_bare_model_never_chooses_an_auth_surface() {
        let registry = Arc::new(ModelRegistry::new());
        let mut api = ModelMetadata::new("shared", "API", ProviderId::OpenAI)
            .with_transport(ApiFormat::OpenAIResponses);
        api.auth_scope = Some(ModelAuthScope::ApiKey);
        let mut oauth = ModelMetadata::new("shared", "OAuth", ProviderId::OpenAI)
            .with_transport(ApiFormat::OpenAIResponses);
        oauth.auth_scope = Some(ModelAuthScope::OAuth);
        registry
            .set_models(ProviderId::OpenAI, vec![api, oauth])
            .await;

        let temp = tempfile::tempdir().unwrap();
        let selected = AiBootstrapPolicy
            .resolve_model_selection(
                &registry,
                &temp.path().join("krusty.db"),
                Some("shared"),
                None,
            )
            .await;

        assert!(selected.is_none());
    }

    #[tokio::test]
    async fn ambiguous_bare_model_never_falls_back_to_builtin_provider_inference() {
        let registry = Arc::new(ModelRegistry::new());
        let minimax = ModelMetadata::new("MiniMax-M2.5", "MiniMax", ProviderId::MiniMax)
            .with_transport(ApiFormat::Anthropic);
        let router = ModelMetadata::new("MiniMax-M2.5", "Router", ProviderId::OpenRouter)
            .with_transport(ApiFormat::Anthropic);
        registry
            .set_models(ProviderId::MiniMax, vec![minimax])
            .await;
        registry
            .set_models(ProviderId::OpenRouter, vec![router])
            .await;

        let provider = AiBootstrapPolicy
            .provider_for_model(&registry, "MiniMax-M2.5")
            .await;

        assert_eq!(provider, None);
    }

    #[tokio::test]
    async fn unavailable_persisted_exact_key_is_not_rebound_through_legacy_slug() {
        let registry = Arc::new(ModelRegistry::new());
        let available = ModelMetadata::new("shared", "API", ProviderId::OpenAI)
            .with_transport(ApiFormat::OpenAIResponses);
        registry
            .set_models(ProviderId::OpenAI, vec![available])
            .await;

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("krusty.db");
        let preferences = Preferences::new(Database::new(&db_path).unwrap());
        preferences.set_current_model("shared").unwrap();
        let stale = ModelKey::new(ProviderId::OpenAI, "shared", ApiFormat::OpenAIResponses)
            .with_auth_scope(ModelAuthScope::OAuth);
        preferences.set_current_model_key(&stale).unwrap();

        let selected = AiBootstrapPolicy
            .resolve_model_selection(&registry, &db_path, None, None)
            .await;

        assert!(selected.is_none());
    }
}
