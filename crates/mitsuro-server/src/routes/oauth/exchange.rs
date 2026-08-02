use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use mitsuro_core::auth::{
    anthropic_oauth_config, OAuthTokenStore, PasteCodeOAuthFlow, PkceVerifier,
};

use super::{parse_provider, OAuthFlowKind};
use crate::ai_bootstrap::{
    invalidate_provider_model_catalog, spawn_provider_model_catalog_refresh,
};
use crate::error::AppError;
use crate::AppState;

#[derive(Deserialize)]
pub(super) struct OAuthExchangeRequest {
    provider: String,
    code: String,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Serialize)]
pub(super) struct OAuthExchangeResponse {
    success: bool,
}

pub(super) async fn exchange_code(
    State(state): State<AppState>,
    Json(req): Json<OAuthExchangeRequest>,
) -> Result<Json<OAuthExchangeResponse>, AppError> {
    let provider_id = parse_provider(&req.provider)?;

    let verifier_str = {
        let flows = state.oauth_flows.lock().await;
        flows
            .get(provider_id.storage_key())
            .and_then(|flow| match &flow.kind {
                OAuthFlowKind::PkceVerifier { verifier_str } => Some(verifier_str.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                AppError::BadRequest(
                    "No active paste-code OAuth flow for this provider".to_string(),
                )
            })?
    };

    let verifier = PkceVerifier::from_string(verifier_str);
    let flow = PasteCodeOAuthFlow::new(anthropic_oauth_config());
    let token_data = flow
        .exchange_code(&req.code, req.state.as_deref(), &verifier)
        .await
        .map_err(|error| {
            tracing::error!("OAuth token exchange failed for {}: {}", provider_id, error);
            AppError::Internal(error.to_string())
        })?;

    OAuthTokenStore::set_persisted(provider_id, token_data).map_err(|error| {
        tracing::error!("Failed to save OAuth token: {}", error);
        AppError::Internal(error.to_string())
    })?;

    invalidate_provider_model_catalog(&state.model_registry, state.db_path.as_path(), provider_id)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    spawn_provider_model_catalog_refresh(
        state.model_registry.clone(),
        state.credential_store.clone(),
        state.db_path.clone(),
        provider_id,
        false,
    );

    tracing::info!("OAuth token stored successfully for {}", provider_id);
    state
        .oauth_flows
        .lock()
        .await
        .remove(provider_id.storage_key());

    Ok(Json(OAuthExchangeResponse { success: true }))
}
