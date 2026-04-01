//! OAuth authentication endpoints for PWA.

use std::time::Instant;

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::{
    anthropic_oauth_config, openai_oauth_config, HostedBrowserOAuthFlow, OAuthTokenStore,
    OpenAIDeviceAuthFlow, OpenAIDeviceCodeResponse, PasteCodeOAuthFlow, PkceVerifier,
};

use crate::error::AppError;
use crate::AppState;

const FLOW_TTL_SECS: u64 = 300;
const OAUTH_RESULT_STORAGE_KEY: &str = "krusty:oauth-result";
const OAUTH_RESULT_CHANNEL: &str = "krusty:oauth";

/// In-flight OAuth flow state stored on the server.
#[derive(Clone)]
pub struct OAuthFlowState {
    pub started_at: Instant,
    pub provider_id: ProviderId,
    pub kind: OAuthFlowKind,
}

#[derive(Clone)]
pub enum OAuthFlowKind {
    PkceVerifier {
        verifier_str: String,
    },
    BrowserCallback {
        state: String,
        verifier_str: String,
        redirect_uri: String,
    },
    DeviceFlow {
        flow_id: String,
    },
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/start", post(start_oauth))
        .route("/exchange", post(exchange_code))
        .route("/status/:provider", get(oauth_status))
        .route("/revoke/:provider", delete(revoke_oauth))
}

pub fn callback_router() -> Router<AppState> {
    Router::new().route("/auth/oauth/callback/:provider", get(oauth_callback))
}

#[derive(Serialize)]
struct OAuthStartResponse {
    auth_url: String,
    provider: String,
    flow_type: String,
    paste_code: bool,
    device_code: Option<OAuthDeviceCodeResponse>,
}

#[derive(Serialize)]
struct OAuthDeviceCodeResponse {
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize)]
struct OAuthStartRequest {
    provider: String,
}

async fn start_oauth(
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

    match provider_id {
        ProviderId::OpenAI => start_openai_oauth(state, provider_id, &headers).await,
        ProviderId::Anthropic => start_anthropic_oauth(state, provider_id).await,
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

async fn start_openai_device_oauth(
    state: AppState,
    provider_id: ProviderId,
) -> Result<Json<OAuthStartResponse>, AppError> {
    let config = openai_oauth_config();
    let flow = OpenAIDeviceAuthFlow::new(config);
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
    let config = anthropic_oauth_config();
    let flow = PasteCodeOAuthFlow::new(config);
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

#[derive(Deserialize)]
struct OAuthExchangeRequest {
    provider: String,
    code: String,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Serialize)]
struct OAuthExchangeResponse {
    success: bool,
}

async fn exchange_code(
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

    let mut store = OAuthTokenStore::load().map_err(|error| {
        tracing::error!("Failed to load OAuth token store: {}", error);
        AppError::Internal(error.to_string())
    })?;
    store.set(provider_id, token_data);
    store.save().map_err(|error| {
        tracing::error!("Failed to save OAuth token: {}", error);
        AppError::Internal(error.to_string())
    })?;

    tracing::info!("OAuth token stored successfully for {}", provider_id);
    state
        .oauth_flows
        .lock()
        .await
        .remove(provider_id.storage_key());

    Ok(Json(OAuthExchangeResponse { success: true }))
}

#[derive(Serialize)]
struct OAuthStatusResponse {
    has_token: bool,
    flow_active: bool,
}

async fn oauth_status(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<OAuthStatusResponse>, AppError> {
    let provider_id = parse_provider(&provider)?;

    let has_token = OAuthTokenStore::load()
        .map(|store| store.has_token(&provider_id))
        .unwrap_or(false);

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

async fn revoke_oauth(
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

#[derive(Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    let provider_id = match parse_provider(&provider) {
        Ok(provider_id) => provider_id,
        Err(_) => {
            return callback_error_page(
                provider,
                "Unknown provider for OAuth callback.".to_string(),
            );
        }
    };

    if provider_id != ProviderId::OpenAI {
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "This callback route is not enabled for that provider.".to_string(),
        );
    }

    let flow_state = {
        let flows = state.oauth_flows.lock().await;
        flows.get(provider_id.storage_key()).cloned()
    };

    let Some(flow_state) = flow_state else {
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "No active sign-in is waiting for this callback.".to_string(),
        );
    };

    if flow_state.started_at.elapsed().as_secs() >= FLOW_TTL_SECS {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "This sign-in session expired. Start the login again from Krusty.".to_string(),
        );
    }

    let (expected_state, verifier_str, redirect_uri) = match flow_state.kind {
        OAuthFlowKind::BrowserCallback {
            state,
            verifier_str,
            redirect_uri,
        } => (state, verifier_str, redirect_uri),
        OAuthFlowKind::PkceVerifier { .. } | OAuthFlowKind::DeviceFlow { .. } => {
            return callback_error_page(
                provider_id.storage_key().to_string(),
                "The active sign-in for this provider is not using a browser callback.".to_string(),
            );
        }
    };

    if let Some(error_code) = query.error.as_deref() {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        let description = query
            .error_description
            .as_deref()
            .unwrap_or("Sign-in was canceled or denied.");
        return callback_error_page(
            provider_id.storage_key().to_string(),
            format!("{} ({})", description, error_code),
        );
    }

    if query.state.as_deref() != Some(expected_state.as_str()) {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "The returned sign-in state did not match the active request.".to_string(),
        );
    }

    let Some(code) = query.code.as_deref() else {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "The provider callback did not include an authorization code.".to_string(),
        );
    };

    let verifier = PkceVerifier::from_string(verifier_str);
    let flow = HostedBrowserOAuthFlow::new(openai_oauth_config(), redirect_uri);
    let exchange_result = flow.exchange_code(code, &verifier).await;

    state
        .oauth_flows
        .lock()
        .await
        .remove(provider_id.storage_key());

    match exchange_result {
        Ok(token_data) => {
            let mut store = match OAuthTokenStore::load() {
                Ok(store) => store,
                Err(error) => {
                    return callback_error_page(
                        provider_id.storage_key().to_string(),
                        format!("Failed to load token storage: {}", error),
                    );
                }
            };
            store.set(provider_id, token_data);
            if let Err(error) = store.save() {
                return callback_error_page(
                    provider_id.storage_key().to_string(),
                    format!("Failed to save your sign-in: {}", error),
                );
            }

            callback_success_page(provider_id.storage_key().to_string())
        }
        Err(error) => callback_error_page(provider_id.storage_key().to_string(), error.to_string()),
    }
}

fn parse_provider(input: &str) -> Result<ProviderId, AppError> {
    crate::utils::providers::parse_provider(input)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {}", input)))
}

fn device_code_response(code: &OpenAIDeviceCodeResponse) -> OAuthDeviceCodeResponse {
    OAuthDeviceCodeResponse {
        user_code: code.user_code.clone(),
        verification_uri: code.verification_uri.clone(),
        verification_uri_complete: code.verification_uri_complete.clone(),
        expires_in: code.expires_in,
    }
}

fn callback_success_page(provider: String) -> Response {
    callback_page(provider, true, "Sign-in complete".to_string(), None)
}

fn callback_error_page(provider: String, error: String) -> Response {
    callback_page(
        provider,
        false,
        "Sign-in did not finish".to_string(),
        Some(error),
    )
}

fn callback_page(
    provider: String,
    success: bool,
    headline: String,
    error: Option<String>,
) -> Response {
    let message = if success {
        "You can return to Krusty now. This tab will close if your browser allows it.".to_string()
    } else {
        error
            .clone()
            .unwrap_or_else(|| "Something went wrong during sign-in.".to_string())
    };
    let payload = json!({
        "type": "krusty-oauth-complete",
        "provider": provider,
        "success": success,
        "error": error,
        "issued_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
    });
    let payload_json =
        serde_json::to_string(&payload).unwrap_or_else(|_| "{\"success\":false}".to_string());
    let return_url = serde_json::to_string("/").unwrap_or_else(|_| "\"/\"".to_string());
    let headline_class = if success { "status ok" } else { "status error" };
    let headline_text = escape_html(&headline);
    let message_text = escape_html(&message);

    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Cache-Control" content="no-store">
  <title>{headline_text}</title>
  <style>
    :root {{
      color-scheme: light;
      font-family: ui-sans-serif, system-ui, sans-serif;
      background: #f4efe7;
      color: #1f1a17;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background:
        radial-gradient(circle at top, rgba(217, 119, 6, 0.18), transparent 35%),
        linear-gradient(180deg, #fbf7f0 0%, #f1ebe0 100%);
    }}
    main {{
      width: min(92vw, 30rem);
      padding: 1.5rem;
      border-radius: 1.5rem;
      background: rgba(255, 255, 255, 0.9);
      box-shadow: 0 20px 60px rgba(61, 42, 24, 0.16);
      border: 1px solid rgba(95, 72, 53, 0.12);
    }}
    h1 {{
      margin: 0 0 0.75rem;
      font-size: 1.5rem;
      line-height: 1.2;
    }}
    p {{
      margin: 0;
      color: #5c4a3b;
      line-height: 1.5;
    }}
    .status {{
      display: inline-flex;
      align-items: center;
      gap: 0.5rem;
      margin-bottom: 1rem;
      font-size: 0.8rem;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}
    .ok {{
      color: #166534;
    }}
    .error {{
      color: #b42318;
    }}
    a {{
      display: inline-flex;
      margin-top: 1.25rem;
      padding: 0.75rem 1rem;
      border-radius: 999px;
      text-decoration: none;
      color: white;
      background: #111827;
    }}
  </style>
</head>
<body>
  <main>
    <div class="{headline_class}">{provider}</div>
    <h1>{headline_text}</h1>
    <p>{message_text}</p>
    <a href="/">Return to Krusty</a>
  </main>
  <script>
    const payload = {payload_json};
    const returnUrl = {return_url};

    try {{
      localStorage.setItem({storage_key:?}, JSON.stringify(payload));
    }} catch (error) {{
      console.debug('oauth localStorage signal failed', error);
    }}

    try {{
      if (window.opener && !window.opener.closed) {{
        window.opener.postMessage(payload, window.location.origin);
        window.opener.focus();
      }}
    }} catch (error) {{
      console.debug('oauth opener signal failed', error);
    }}

    try {{
      if ('BroadcastChannel' in window) {{
        const channel = new BroadcastChannel({channel_name:?});
        channel.postMessage(payload);
        channel.close();
      }}
    }} catch (error) {{
      console.debug('oauth broadcast signal failed', error);
    }}

    setTimeout(() => window.close(), 150);
    setTimeout(() => {{
      if (window.location.pathname !== returnUrl) {{
        window.location.replace(returnUrl);
      }}
    }}, 1200);
  </script>
</body>
</html>"#,
        provider = escape_html(&provider),
        headline_class = headline_class,
        headline_text = headline_text,
        message_text = message_text,
        payload_json = payload_json,
        return_url = return_url,
        storage_key = OAUTH_RESULT_STORAGE_KEY,
        channel_name = OAUTH_RESULT_CHANNEL,
    );

    Html(html).into_response()
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
