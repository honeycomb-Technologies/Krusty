use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::OAuthTokenStore;

use super::parse_provider;
use crate::error::AppError;
use crate::AppState;

#[derive(Serialize)]
pub(super) struct OAuthStatusResponse {
    has_token: bool,
    flow_active: bool,
}

pub(super) fn load_oauth_token_presence(
    provider_id: ProviderId,
    load: impl FnOnce() -> anyhow::Result<OAuthTokenStore>,
) -> Result<bool, AppError> {
    match load() {
        Ok(store) => Ok(store.has_token(&provider_id)),
        Err(error) => {
            tracing::warn!(
                provider = %provider_id,
                error = %error,
                "Failed to load OAuth token store while serving OAuth status"
            );
            Err(AppError::Internal(error.to_string()))
        }
    }
}

pub(super) async fn oauth_status(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<OAuthStatusResponse>, AppError> {
    let provider_id = parse_provider(&provider)?;

    let has_token = load_oauth_token_presence(provider_id, OAuthTokenStore::load)?;

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
