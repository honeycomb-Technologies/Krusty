use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};

use crate::auth::{is_local_host, request_remote_access_token, REMOTE_AUTH_COOKIE_NAME};
use crate::error::ApiError;
use crate::types::{RemoteAuthBootstrapRequest, RemoteAuthStatusResponse};
use crate::AppState;

const REMOTE_AUTH_COOKIE_MAX_AGE_SECS: u64 = 60 * 60 * 24 * 30;

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/remote-auth/status", get(get_remote_auth_status))
        .route("/remote-auth/bootstrap", post(post_remote_auth_bootstrap))
}

async fn get_remote_auth_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<RemoteAuthStatusResponse> {
    let remote_access = state.remote_access.read().await.clone();
    let authenticated = remote_access.enabled
        && request_remote_access_token(&headers)
            .map(|token| token == remote_access.token)
            .unwrap_or(false);

    Json(RemoteAuthStatusResponse { authenticated })
}

async fn post_remote_auth_bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RemoteAuthBootstrapRequest>,
) -> Response {
    let remote_access = state.remote_access.read().await.clone();
    if !remote_access.enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiError {
                error: "Remote access is disabled for this server".to_string(),
                code: "REMOTE_DISABLED".to_string(),
            }),
        )
            .into_response();
    }

    let provided = req.token.trim();
    if provided.is_empty() || provided != remote_access.token {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "Remote API access requires a valid bearer token".to_string(),
                code: "REMOTE_TOKEN_INVALID".to_string(),
            }),
        )
            .into_response();
    }

    (
        AppendHeaders([(
            header::SET_COOKIE,
            build_remote_auth_cookie(&headers, provided),
        )]),
        Json(RemoteAuthStatusResponse {
            authenticated: true,
        }),
    )
        .into_response()
}

fn build_remote_auth_cookie(headers: &HeaderMap, token: &str) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let mut cookie = format!(
        "{REMOTE_AUTH_COOKIE_NAME}={token}; Path=/; Max-Age={REMOTE_AUTH_COOKIE_MAX_AGE_SECS}; HttpOnly; SameSite=Lax"
    );

    if !is_local_host(host) {
        cookie.push_str("; Secure");
    }

    cookie
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{atomic::AtomicUsize, Arc};

    use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
    use axum::response::IntoResponse;
    use axum::{extract::State, Json};
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::Database;
    use krusty_core::tools::registry::ToolRegistry;

    use super::{build_remote_auth_cookie, get_remote_auth_status, post_remote_auth_bootstrap};
    use crate::types::RemoteAuthBootstrapRequest;
    use crate::AppState;

    fn test_state() -> (AppState, std::path::PathBuf) {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-remote-auth-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("krusty.db");
        Database::new(&db_path).expect("database should initialize");

        (
            AppState {
                server_port: 3000,
                db_path: Arc::new(db_path),
                working_dir: Arc::new(temp_dir.clone()),
                ai_client: None,
                tool_registry: Arc::new(ToolRegistry::new()),
                process_registry: Arc::new(ProcessRegistry::new()),
                model_registry: create_model_registry(),
                credential_store: Arc::new(RwLock::new(CredentialStore::default())),
                mcp_manager: Arc::new(McpManager::new(temp_dir.clone())),
                hook_manager: Arc::new(RwLock::new(UserHookManager::new())),
                skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(&temp_dir))),
                cancellation: AgentCancellation::new(),
                session_locks: Arc::new(RwLock::new(HashMap::new())),
                session_inputs: Arc::new(RwLock::new(HashMap::new())),
                session_presence: Arc::new(RwLock::new(HashMap::new())),
                delegated_state: Arc::new(RwLock::new(HashMap::new())),
                remote_access: Arc::new(RwLock::new(crate::remote_access::RemoteAccessConfig {
                    enabled: true,
                    token: "secret".to_string(),
                })),
                active_agent_streams: Arc::new(AtomicUsize::new(0)),
                peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                push_service: None,
                apns_service: None,
                oauth_flows: Arc::new(Mutex::new(HashMap::new())),
            },
            temp_dir,
        )
    }

    #[tokio::test]
    async fn status_reports_authenticated_when_cookie_matches() {
        let (state, _dir) = test_state();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("krusty_remote_access=secret"),
        );

        let Json(response) = get_remote_auth_status(State(state), headers).await;
        assert!(response.authenticated);
    }

    #[tokio::test]
    async fn bootstrap_sets_secure_cookie_for_remote_hosts() {
        let (state, _dir) = test_state();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static("honey-1.taila8563f.ts.net:8443"),
        );

        let response = post_remote_auth_bootstrap(
            State(state),
            headers,
            Json(RemoteAuthBootstrapRequest {
                token: "secret".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("bootstrap should set cookie");
        assert!(cookie.contains("krusty_remote_access=secret"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
    }

    #[test]
    fn local_hosts_omit_secure_cookie_flag() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));

        let cookie = build_remote_auth_cookie(&headers, "secret");
        assert!(!cookie.contains("Secure"));
    }
}
