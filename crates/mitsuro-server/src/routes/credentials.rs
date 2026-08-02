//! Credential management endpoints.

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use mitsuro_core::ai::providers::ProviderId;
use mitsuro_core::auth::{AuthMethod, OAuthTokenStore};

use crate::ai_bootstrap::{
    invalidate_provider_model_catalog, spawn_provider_model_catalog_refresh,
};
use crate::error::AppError;
use crate::AppState;

/// Build the credentials router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_providers))
        .route("/:provider", get(get_provider))
        .route("/:provider", post(set_credential))
        .route("/:provider", delete(delete_credential))
}

#[derive(Serialize)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub configured: bool,
    pub has_oauth: bool,
    pub supports_oauth: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<String>,
}

async fn list_providers(State(state): State<AppState>) -> Json<Vec<ProviderStatus>> {
    let store = state.credential_store.read().await;
    let oauth_store = load_oauth_store_or_default("listing credential providers");

    let providers = ProviderId::all()
        .iter()
        .map(|id| {
            let has_oauth = has_provider_oauth(*id, &oauth_store, &store);
            ProviderStatus {
                id: id.storage_key().to_string(),
                name: crate::utils::providers::provider_display_name(*id).to_string(),
                configured: store.has_key(id) || has_oauth,
                has_oauth,
                supports_oauth: id.supports_oauth(),
                auth_methods: auth_method_keys(*id),
            }
        })
        .collect();

    Json(providers)
}

async fn get_provider(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<ProviderStatus>, AppError> {
    let provider_id = parse_provider(&provider)?;
    let store = state.credential_store.read().await;
    let oauth_store = load_oauth_store_or_default("loading credential provider");

    let has_oauth = has_provider_oauth(provider_id, &oauth_store, &store);

    Ok(Json(ProviderStatus {
        id: provider_id.storage_key().to_string(),
        name: crate::utils::providers::provider_display_name(provider_id).to_string(),
        configured: store.has_key(&provider_id) || has_oauth,
        has_oauth,
        supports_oauth: provider_id.supports_oauth(),
        auth_methods: auth_method_keys(provider_id),
    }))
}

#[derive(Deserialize)]
pub struct SetCredentialRequest {
    pub api_key: String,
}

async fn set_credential(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(req): Json<SetCredentialRequest>,
) -> Result<Json<ProviderStatus>, AppError> {
    let provider_id = parse_provider(&provider)?;

    if req.api_key.trim().is_empty() {
        return Err(AppError::BadRequest("API key cannot be empty".to_string()));
    }

    {
        let mut store = state.credential_store.write().await;
        store.set(provider_id, req.api_key.clone());
        store
            .save()
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    if mitsuro_core::ai::catalog::supports_dynamic_models(provider_id) {
        invalidate_provider_model_catalog(
            &state.model_registry,
            state.db_path.as_path(),
            provider_id,
        )
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        spawn_provider_model_catalog_refresh(
            state.model_registry.clone(),
            state.credential_store.clone(),
            state.db_path.clone(),
            provider_id,
            false,
        );
    }

    let oauth_store = load_oauth_store_or_default("setting credential provider");
    let credentials = state.credential_store.read().await;
    let has_oauth = has_provider_oauth(provider_id, &oauth_store, &credentials);
    Ok(Json(ProviderStatus {
        id: provider_id.storage_key().to_string(),
        name: crate::utils::providers::provider_display_name(provider_id).to_string(),
        configured: true,
        has_oauth,
        supports_oauth: provider_id.supports_oauth(),
        auth_methods: auth_method_keys(provider_id),
    }))
}

async fn delete_credential(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<ProviderStatus>, AppError> {
    let provider_id = parse_provider(&provider)?;

    {
        let mut store = state.credential_store.write().await;
        store.remove(&provider_id);
        store
            .save()
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    if mitsuro_core::ai::catalog::supports_dynamic_models(provider_id) {
        invalidate_provider_model_catalog(
            &state.model_registry,
            state.db_path.as_path(),
            provider_id,
        )
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        // An OAuth or environment identity may remain after deleting the stored
        // API key. The canonical refresh either restores that catalog or safely
        // leaves the curated fallback when no credential remains.
        spawn_provider_model_catalog_refresh(
            state.model_registry.clone(),
            state.credential_store.clone(),
            state.db_path.clone(),
            provider_id,
            false,
        );
    }

    let oauth_store = load_oauth_store_or_default("deleting credential provider");
    let store = state.credential_store.read().await;
    let has_oauth = has_provider_oauth(provider_id, &oauth_store, &store);
    Ok(Json(ProviderStatus {
        id: provider_id.storage_key().to_string(),
        name: crate::utils::providers::provider_display_name(provider_id).to_string(),
        configured: has_oauth,
        has_oauth,
        supports_oauth: provider_id.supports_oauth(),
        auth_methods: auth_method_keys(provider_id),
    }))
}

fn parse_provider(s: &str) -> Result<ProviderId, AppError> {
    crate::utils::providers::parse_provider(s)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {}", s)))
}

pub(crate) fn has_provider_oauth(
    provider_id: ProviderId,
    oauth_store: &OAuthTokenStore,
    credentials: &mitsuro_core::storage::CredentialStore,
) -> bool {
    if oauth_store.has_token(&provider_id) {
        return true;
    }

    provider_id == ProviderId::Grok
        && mitsuro_core::auth::resolve_grok_auth(credentials)
            .credential
            .is_some()
}

fn auth_method_keys(provider_id: ProviderId) -> Vec<String> {
    provider_id
        .auth_methods()
        .into_iter()
        .map(auth_method_key)
        .collect()
}

fn auth_method_key(method: AuthMethod) -> String {
    match method {
        AuthMethod::ApiKey => "api_key".to_string(),
        AuthMethod::OAuthBrowser => "oauth_browser".to_string(),
        AuthMethod::OAuthDevice => "oauth_device".to_string(),
    }
}

fn load_oauth_store_or_default(context: &'static str) -> OAuthTokenStore {
    load_oauth_store_or_else(context, OAuthTokenStore::load)
}

fn load_oauth_store_or_else(
    context: &'static str,
    load: impl FnOnce() -> anyhow::Result<OAuthTokenStore>,
) -> OAuthTokenStore {
    match load() {
        Ok(store) => store,
        Err(error) => {
            warn!(context, error = %error, "Failed to load OAuth token store");
            OAuthTokenStore::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::load_oauth_store_or_else;
    use mitsuro_core::auth::OAuthTokenStore;

    #[test]
    fn load_oauth_store_or_else_returns_loaded_store() {
        let store = load_oauth_store_or_else("test", || Ok(OAuthTokenStore::default()));

        assert!(!store.has_token(&mitsuro_core::ai::providers::ProviderId::OpenAI));
    }

    #[test]
    fn load_oauth_store_or_else_falls_back_to_default_on_error() {
        let store = load_oauth_store_or_else("test", || Err(anyhow::anyhow!("boom")));

        assert!(!store.has_token(&mitsuro_core::ai::providers::ProviderId::OpenAI));
    }
}
