use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::{clear_grok_cli_auth, OAuthTokenStore};

use super::parse_provider;
use crate::ai_bootstrap::{
    invalidate_provider_model_catalog, spawn_provider_model_catalog_refresh,
};
use crate::error::AppError;
use crate::routes::credentials::has_provider_oauth;
use crate::AppState;

#[derive(Serialize)]
pub(super) struct OAuthStatusResponse {
    has_token: bool,
    flow_active: bool,
}

pub(super) fn provider_has_oauth_token(
    provider_id: ProviderId,
    oauth_store: &OAuthTokenStore,
    credentials: &krusty_core::storage::CredentialStore,
) -> bool {
    has_provider_oauth(provider_id, oauth_store, credentials)
}

pub(super) async fn oauth_status(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<OAuthStatusResponse>, AppError> {
    let provider_id = parse_provider(&provider)?;

    let oauth_store = OAuthTokenStore::load().map_err(|error| {
        tracing::warn!(
            provider = %provider_id,
            error = %error,
            "Failed to load OAuth token store while serving OAuth status"
        );
        AppError::Internal(error.to_string())
    })?;
    let credentials = state.credential_store.read().await;
    let has_token = provider_has_oauth_token(provider_id, &oauth_store, &credentials);

    let flow_active = state
        .oauth_flows
        .lock()
        .await
        .contains_key(provider_id.storage_key());

    Ok(Json(OAuthStatusResponse {
        has_token,
        flow_active,
    }))
}

pub(super) async fn revoke_oauth(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<OAuthStatusResponse>, AppError> {
    let provider_id = parse_provider(&provider)?;

    let mut store =
        OAuthTokenStore::load().map_err(|error| AppError::Internal(error.to_string()))?;
    store.remove(&provider_id);
    store
        .save()
        .map_err(|error| AppError::Internal(error.to_string()))?;

    if provider_id == ProviderId::Grok {
        if let Err(error) = clear_grok_cli_auth() {
            tracing::warn!(error = %error, "Failed to clear Grok CLI auth file during OAuth revoke");
        }
    }

    if krusty_core::ai::catalog::supports_dynamic_models(provider_id) {
        invalidate_provider_model_catalog(
            &state.model_registry,
            state.db_path.as_path(),
            provider_id,
        )
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        // A stored API key or environment credential may remain after OAuth
        // revocation. Rebuild from it; otherwise the curated fallback remains.
        spawn_provider_model_catalog_refresh(
            state.model_registry.clone(),
            state.credential_store.clone(),
            state.db_path.clone(),
            provider_id,
            false,
        );
    }

    state
        .oauth_flows
        .lock()
        .await
        .remove(provider_id.storage_key());

    Ok(Json(OAuthStatusResponse {
        has_token: false,
        flow_active: false,
    }))
}
