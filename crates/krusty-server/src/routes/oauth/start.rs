use std::time::Instant;

use axum::{extract::State, http::HeaderMap, Json};
use serde::{Deserialize, Serialize};

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::{
    anthropic_oauth_config, force_grok_browser_login, grok_auth_token_to_oauth_data,
    openai_oauth_config, BrowserOAuthFlow, HostedBrowserOAuthFlow, OAuthTokenStore,
    OpenAIDeviceAuthFlow, OpenAIDeviceCodeResponse, PasteCodeOAuthFlow,
};

use super::{parse_provider, OAuthFlowKind, OAuthFlowState, FLOW_TTL_SECS};
use crate::error::AppError;
use crate::AppState;

#[derive(Serialize)]
pub(super) struct OAuthStartResponse {
    auth_url: String,
    provider: String,
    flow_type: String,
    paste_code: bool,
    device_code: Option<OAuthDeviceCodeResponsePayload>,
}

#[derive(Serialize)]
struct OAuthDeviceCodeResponsePayload {
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize)]
pub(super) struct OAuthStartRequest {
    provider: String,
    #[serde(default)]
    flow_type: Option<String>,
}

pub(super) async fn start_oauth(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OAuthStartRequest>,
) -> Result<Json<OAuthStartResponse>, AppError> {
    let provider_id = parse_provider(&req.provider)?;

    if !provider_id.supports_oauth() {
        return Err(AppError::BadRequest(format!(
            "Provider {} does not support OAuth",
            req.provider
        )));
    }

    {
        let flows = state.oauth_flows.lock().await;
        if let Some(existing) = flows.get(provider_id.storage_key()) {
            if existing.started_at.elapsed().as_secs() < FLOW_TTL_SECS {
                return Err(AppError::Conflict(
                    "OAuth flow already in progress for this provider".to_string(),
                ));
            }
        }
    }

    match (provider_id, req.flow_type.as_deref()) {
        (ProviderId::OpenAI, Some("device")) => start_openai_device_oauth(state, provider_id).await,
        (ProviderId::OpenAI, Some("browser")) => {
            start_openai_local_browser_oauth(state, provider_id).await
        }
        (ProviderId::OpenAI, Some("browser_callback") | Some("callback")) => {
            start_openai_oauth(state, provider_id, &headers).await
        }
        (ProviderId::OpenAI, _) => start_openai_device_oauth(state, provider_id).await,
        (ProviderId::Anthropic, Some("device")) => Err(AppError::BadRequest(
            "Anthropic does not support device-code OAuth".to_string(),
        )),
        (ProviderId::Anthropic, _) => start_anthropic_oauth(state, provider_id).await,
        (ProviderId::Grok, Some("device")) => Err(AppError::BadRequest(
            "xAI/Grok does not support device-code OAuth".to_string(),
        )),
        (ProviderId::Grok, _) => start_grok_browser_oauth(state, provider_id).await,
        _ => Err(AppError::BadRequest(
            "OAuth not implemented for this provider".to_string(),
        )),
    }
}

async fn start_openai_oauth(
    state: AppState,
    provider_id: ProviderId,
    headers: &HeaderMap,
) -> Result<Json<OAuthStartResponse>, AppError> {
    match try_start_openai_browser_oauth(state.clone(), provider_id, headers).await {
        Ok(response) => Ok(response),
        Err(error) => {
            tracing::warn!(
                "Falling back to OpenAI device auth for {}: {}",
                provider_id,
                error
            );
            start_openai_device_oauth(state, provider_id).await
        }
    }
}

async fn try_start_openai_browser_oauth(
    state: AppState,
    provider_id: ProviderId,
    headers: &HeaderMap,
) -> anyhow::Result<Json<OAuthStartResponse>> {
    let public_base_url = crate::utils::public_url::resolve_public_base_url(headers)?;
    let redirect_uri = format!(
        "{}/auth/oauth/callback/{}",
        public_base_url,
        provider_id.storage_key()
    );
    let flow = HostedBrowserOAuthFlow::new(openai_oauth_config(), redirect_uri.clone());
    let (auth_url, verifier, state_token) = flow.get_auth_url()?;

    {
        let mut flows = state.oauth_flows.lock().await;
        flows.insert(
            provider_id.storage_key().to_string(),
            OAuthFlowState {
                started_at: Instant::now(),
                provider_id,
                kind: OAuthFlowKind::BrowserCallback {
                    state: state_token,
                    verifier_str: verifier.as_str().to_string(),
                    redirect_uri,
                },
            },
        );
    }

    Ok(Json(OAuthStartResponse {
        auth_url,
        provider: provider_id.storage_key().to_string(),
        flow_type: "browser_callback".to_string(),
        paste_code: false,
        device_code: None,
    }))
}

async fn start_openai_local_browser_oauth(
    state: AppState,
    provider_id: ProviderId,
) -> Result<Json<OAuthStartResponse>, AppError> {
    mark_spawned_oauth_flow(&state, provider_id).await;

    let oauth_flows = state.oauth_flows.clone();
    let model_registry = state.model_registry.clone();
    tokio::spawn(async move {
        let result = BrowserOAuthFlow::new(openai_oauth_config()).run().await;
        match result {
            Ok(token_data) => {
                if let Ok(mut store) = OAuthTokenStore::load() {
                    store.set(provider_id, token_data);
                    if let Err(error) = store.save() {
                        tracing::error!("Failed to save OpenAI OAuth token: {}", error);
                    } else {
                        tracing::info!("OpenAI browser OAuth token stored successfully");
                        refresh_openai_models(model_registry.clone()).await;
                    }
                }
            }
            Err(error) => tracing::warn!("OpenAI browser OAuth failed: {}", error),
        }

        oauth_flows.lock().await.remove(provider_id.storage_key());
    });

    Ok(Json(OAuthStartResponse {
        auth_url: String::new(),
        provider: provider_id.storage_key().to_string(),
        flow_type: "browser_process".to_string(),
        paste_code: false,
        device_code: None,
    }))
}

async fn start_grok_browser_oauth(
    state: AppState,
    provider_id: ProviderId,
) -> Result<Json<OAuthStartResponse>, AppError> {
    mark_spawned_oauth_flow(&state, provider_id).await;

    let oauth_flows = state.oauth_flows.clone();
    tokio::spawn(async move {
        let result = force_grok_browser_login().await;
        match result {
            Ok(token) => {
                if let Ok(mut store) = OAuthTokenStore::load() {
                    store.set(provider_id, grok_auth_token_to_oauth_data(&token));
                    if let Err(error) = store.save() {
                        tracing::error!("Failed to save xAI/Grok OAuth token: {}", error);
                    } else {
                        tracing::info!("xAI/Grok browser OAuth token stored successfully");
                    }
                }
            }
            Err(error) => tracing::warn!("xAI/Grok browser OAuth failed: {}", error),
        }

        oauth_flows.lock().await.remove(provider_id.storage_key());
    });

    Ok(Json(OAuthStartResponse {
        auth_url: String::new(),
        provider: provider_id.storage_key().to_string(),
        flow_type: "browser_process".to_string(),
        paste_code: false,
        device_code: None,
    }))
}

async fn mark_spawned_oauth_flow(state: &AppState, provider_id: ProviderId) {
    let mut flows = state.oauth_flows.lock().await;
    flows.insert(
        provider_id.storage_key().to_string(),
        OAuthFlowState {
            started_at: Instant::now(),
            provider_id,
            kind: OAuthFlowKind::DeviceFlow {
                flow_id: "browser-process".to_string(),
            },
        },
    );
}

async fn start_openai_device_oauth(
    state: AppState,
    provider_id: ProviderId,
) -> Result<Json<OAuthStartResponse>, AppError> {
    let flow = OpenAIDeviceAuthFlow::new(openai_oauth_config());
    let code_response = flow
        .request_code()
        .await
        .map_err(|e| AppError::BadGateway(e.to_string()))?;

    {
        let mut flows = state.oauth_flows.lock().await;
        flows.insert(
            provider_id.storage_key().to_string(),
            OAuthFlowState {
                started_at: Instant::now(),
                provider_id,
                kind: OAuthFlowKind::DeviceFlow {
                    flow_id: code_response.device_auth_id.clone(),
                },
            },
        );
    }

    let oauth_flows = state.oauth_flows.clone();
    let model_registry = state.model_registry.clone();
    let device_auth_id = code_response.device_auth_id.clone();
    let user_code = code_response.user_code.clone();
    let poll_interval = code_response.interval;
    let expires_in = code_response.expires_in;
    tokio::spawn(async move {
        match OpenAIDeviceAuthFlow::new(openai_oauth_config())
            .poll_for_token(&device_auth_id, &user_code, poll_interval, expires_in)
            .await
        {
            Ok(token_data) => {
                let flow_still_active = oauth_flows
                    .lock()
                    .await
                    .contains_key(provider_id.storage_key());

                if !flow_still_active {
                    tracing::info!("Discarding OpenAI OAuth token for canceled flow");
                    return;
                }

                if let Ok(mut store) = OAuthTokenStore::load() {
                    store.set(provider_id, token_data);
                    if let Err(error) = store.save() {
                        tracing::error!("Failed to save OAuth token: {}", error);
                    } else {
                        tracing::info!("OpenAI OAuth token stored successfully");
                        refresh_openai_models(model_registry.clone()).await;
                    }
                }
            }
            Err(error) => {
                tracing::warn!("OpenAI device-code auth failed: {}", error);
            }
        }

        oauth_flows.lock().await.remove(provider_id.storage_key());
    });

    let auth_url = code_response
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| code_response.verification_uri.clone());

    Ok(Json(OAuthStartResponse {
        auth_url,
        provider: provider_id.storage_key().to_string(),
        flow_type: "device".to_string(),
        paste_code: false,
        device_code: Some(device_code_response(&code_response)),
    }))
}

async fn start_anthropic_oauth(
    state: AppState,
    provider_id: ProviderId,
) -> Result<Json<OAuthStartResponse>, AppError> {
    let flow = PasteCodeOAuthFlow::new(anthropic_oauth_config());
    let (auth_url, verifier, _state) = flow
        .get_auth_url()
        .map_err(|e| AppError::Internal(e.to_string()))?;

    {
        let mut flows = state.oauth_flows.lock().await;
        flows.insert(
            provider_id.storage_key().to_string(),
            OAuthFlowState {
                started_at: Instant::now(),
                provider_id,
                kind: OAuthFlowKind::PkceVerifier {
                    verifier_str: verifier.as_str().to_string(),
                },
            },
        );
    }

    Ok(Json(OAuthStartResponse {
        auth_url,
        provider: provider_id.storage_key().to_string(),
        flow_type: "paste_code".to_string(),
        paste_code: true,
        device_code: None,
    }))
}

fn device_code_response(code: &OpenAIDeviceCodeResponse) -> OAuthDeviceCodeResponsePayload {
    OAuthDeviceCodeResponsePayload {
        user_code: code.user_code.clone(),
        verification_uri: code.verification_uri.clone(),
        verification_uri_complete: code.verification_uri_complete.clone(),
        expires_in: code.expires_in,
    }
}

pub(super) async fn refresh_openai_models(registry: krusty_core::ai::models::SharedModelRegistry) {
    refresh_provider_models(registry, ProviderId::OpenAI).await;
}

pub(super) async fn refresh_provider_models(
    registry: krusty_core::ai::models::SharedModelRegistry,
    provider: ProviderId,
) {
    let credentials = match krusty_core::storage::CredentialStore::load() {
        Ok(credentials) => credentials,
        Err(error) => {
            tracing::warn!(
                "Failed to load credentials for {} model refresh: {}",
                provider,
                error
            );
            return;
        }
    };

    if krusty_core::ai::catalog::credentials_for_dynamic_models(provider, &credentials)
        .is_empty()
    {
        tracing::debug!(
            "Skipping {} model refresh after OAuth: no catalog credential is available",
            provider
        );
        return;
    }

    match krusty_core::ai::catalog::fetch_dynamic_models_for_store(provider, &credentials).await {
        Ok(models) => registry.set_models(provider, models).await,
        Err(error) => tracing::warn!("Failed to refresh {} models after OAuth: {}", provider, error),
    }
}
