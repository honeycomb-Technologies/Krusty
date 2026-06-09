//! Credential management endpoints.

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::OAuthTokenStore;

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
                name: id.to_string(),
                configured: store.has_key(id) || has_oauth,
                has_oauth,
                supports_oauth: id.supports_oauth(),
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
        name: provider_id.to_string(),
        configured: store.has_key(&provider_id) || has_oauth,
        has_oauth,
        supports_oauth: provider_id.supports_oauth(),
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

    if matches!(provider_id, ProviderId::OpenRouter | ProviderId::OpenAI) {
        spawn_dynamic_model_refresh(state.model_registry.clone(), provider_id, req.api_key);
    }

    let oauth_store = load_oauth_store_or_default("setting credential provider");
    Ok(Json(ProviderStatus {
        id: provider_id.storage_key().to_string(),
        name: provider_id.to_string(),
        configured: true,
        has_oauth: oauth_store.has_token(&provider_id),
        supports_oauth: provider_id.supports_oauth(),
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

    if provider_id == ProviderId::OpenRouter {
        state.model_registry.set_models(provider_id, vec![]).await;
    }

    let oauth_store = load_oauth_store_or_default("deleting credential provider");
    Ok(Json(ProviderStatus {
        id: provider_id.storage_key().to_string(),
        name: provider_id.to_string(),
        configured: false,
        has_oauth: oauth_store.has_token(&provider_id),
        supports_oauth: provider_id.supports_oauth(),
    }))
}

fn parse_provider(s: &str) -> Result<ProviderId, AppError> {
    crate::utils::providers::parse_provider(s)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {}", s)))
}

fn has_provider_oauth(
    provider_id: ProviderId,
    oauth_store: &OAuthTokenStore,
    credentials: &krusty_core::storage::CredentialStore,
) -> bool {
    if oauth_store.has_token(&provider_id) {
        return true;
    }

    provider_id == ProviderId::Grok
        && krusty_core::auth::resolve_grok_auth(credentials)
            .credential
            .is_some()
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

fn spawn_dynamic_model_refresh(
    registry: krusty_core::ai::models::SharedModelRegistry,
    provider_id: ProviderId,
    credential: String,
) {
    tokio::spawn(async move {
        let result = match provider_id {
            ProviderId::OpenRouter => krusty_core::ai::openrouter::fetch_models(&credential).await,
            ProviderId::OpenAI => krusty_core::ai::openai::fetch_models(&credential).await,
            _ => return,
        };

        match result {
            Ok(models) => registry.set_models(provider_id, models).await,
            Err(e) => tracing::warn!("Failed to refresh {} models: {}", provider_id, e),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::load_oauth_store_or_else;
    use krusty_core::auth::OAuthTokenStore;

    #[test]
    fn load_oauth_store_or_else_returns_loaded_store() {
        let store = load_oauth_store_or_else("test", || Ok(OAuthTokenStore::default()));

        assert!(!store.has_token(&krusty_core::ai::providers::ProviderId::OpenAI));
    }

    #[test]
    fn load_oauth_store_or_else_falls_back_to_default_on_error() {
        let store = load_oauth_store_or_else("test", || Err(anyhow::anyhow!("boom")));

        assert!(!store.has_token(&krusty_core::ai::providers::ProviderId::OpenAI));
    }
}
