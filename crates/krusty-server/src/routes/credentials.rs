//! Credential management endpoints.

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::ai::providers::ProviderId;

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
    let oauth_store = krusty_core::auth::OAuthTokenStore::load().unwrap_or_default();

    let providers = ProviderId::all()
        .iter()
        .map(|id| ProviderStatus {
            id: id.storage_key().to_string(),
            name: id.to_string(),
            configured: store.has_key(id),
            has_oauth: oauth_store.has_token(id),
            supports_oauth: id.supports_oauth(),
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
    let oauth_store = krusty_core::auth::OAuthTokenStore::load().unwrap_or_default();

    Ok(Json(ProviderStatus {
        id: provider_id.storage_key().to_string(),
        name: provider_id.to_string(),
        configured: store.has_key(&provider_id),
        has_oauth: oauth_store.has_token(&provider_id),
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

    if provider_id == ProviderId::OpenRouter {
        let registry = state.model_registry.clone();
        let key = req.api_key;
        tokio::spawn(async move {
            match krusty_core::ai::openrouter::fetch_models(&key).await {
                Ok(models) => registry.set_models(ProviderId::OpenRouter, models).await,
                Err(e) => tracing::warn!("Failed to refresh OpenRouter models: {}", e),
            }
        });
    }

    let oauth_store = krusty_core::auth::OAuthTokenStore::load().unwrap_or_default();
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

    let oauth_store = krusty_core::auth::OAuthTokenStore::load().unwrap_or_default();
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
