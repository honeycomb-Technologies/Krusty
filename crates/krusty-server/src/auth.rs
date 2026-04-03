//! Lightweight auth middleware for self-host deployments.
//!
//! This keeps request-level user context optional:
//! - No auth headers => single-tenant local mode.
//! - `X-User-Id` + optional `X-Workspace-Dir` => scoped multi-user mode.

use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use crate::AppState;

pub(crate) const REMOTE_AUTH_COOKIE_NAME: &str = "krusty_remote_access";

/// User context attached to request extensions by middleware.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: Option<String>,
    pub home_dir: Option<PathBuf>,
}

impl AuthenticatedUser {
    pub fn local() -> Self {
        Self {
            user_id: None,
            home_dir: None,
        }
    }
}

/// Extractor for routes that want user context.
pub struct CurrentUser(pub AuthenticatedUser);

#[async_trait]
impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .map(CurrentUser)
            .ok_or((StatusCode::UNAUTHORIZED, "Not authenticated"))
    }
}

/// Middleware that attaches optional user info to request extensions.
pub async fn auth_middleware(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Response {
    if let Err(rejection) = authorize_request_surface(&state, addr, request.headers()).await {
        tracing::warn!(
            remote_ip = %addr.ip(),
            "Rejected non-loopback API request"
        );
        return rejection.into_response();
    }

    let mut user = AuthenticatedUser::local();

    if let Some(user_id) = request
        .headers()
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
    {
        user.user_id = Some(user_id.to_string());
        let requested_workspace = request
            .headers()
            .get("X-Workspace-Dir")
            .and_then(|v| v.to_str().ok());
        match resolve_workspace_dir(&state.working_dir, requested_workspace) {
            Ok(workspace_dir) => user.home_dir = Some(workspace_dir),
            Err(rejection) => return rejection.into_response(),
        }
    }

    request.extensions_mut().insert(user);
    next.run(request).await
}

async fn authorize_request_surface(
    state: &AppState,
    addr: SocketAddr,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, &'static str)> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if addr.ip().is_loopback() && is_local_host(host) {
        Ok(())
    } else {
        authorize_remote_request(state, headers).await
    }
}

async fn authorize_remote_request(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, &'static str)> {
    let remote_access = state.remote_access.read().await.clone();
    if !remote_access.enabled {
        return Err((
            StatusCode::FORBIDDEN,
            "Remote API access is disabled for this server",
        ));
    }

    let Some(provided_token) = bearer_token(headers) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Remote API access requires a valid bearer token",
        ));
    };

    if provided_token == remote_access.token {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            "Remote API access requires a valid bearer token",
        ))
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn request_remote_access_token(headers: &HeaderMap) -> Option<String> {
    cookie_token(headers).or_else(|| bearer_token(headers).map(ToString::to_string))
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| {
            raw.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                if name.trim() == REMOTE_AUTH_COOKIE_NAME {
                    let token = value.trim();
                    if token.is_empty() {
                        None
                    } else {
                        Some(token.to_string())
                    }
                } else {
                    None
                }
            })
        })
}

pub(crate) fn is_local_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() {
        return true;
    }

    let host = extract_host_authority(host).to_ascii_lowercase();

    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1")
}

fn extract_host_authority(host: &str) -> &str {
    if let Some(stripped) = host.strip_prefix('[') {
        return stripped
            .split_once(']')
            .map(|(inner, _)| inner)
            .unwrap_or(host);
    }

    match host.rsplit_once(':') {
        Some((candidate, port))
            if !candidate.contains(':') && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            candidate
        }
        _ => host,
    }
}

fn resolve_workspace_dir(
    server_root: &Path,
    requested: Option<&str>,
) -> Result<PathBuf, (StatusCode, &'static str)> {
    let workspace_dir = match requested.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => {
            let candidate = PathBuf::from(raw);
            if candidate.is_absolute() {
                candidate
            } else {
                server_root.join(candidate)
            }
        }
        None => server_root.to_path_buf(),
    };

    crate::utils::paths::validate_path_within(server_root, &workspace_dir).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Workspace directory must stay within the configured server root",
        )
    })?;

    Ok(workspace_dir)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::PathBuf;
    use std::sync::{atomic::AtomicUsize, Arc};

    use axum::http::{header, HeaderMap, HeaderValue};
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::tools::registry::ToolRegistry;

    use super::{authorize_request_surface, is_local_host, resolve_workspace_dir};
    use crate::{remote_access::RemoteAccessConfig, AppState};

    #[test]
    fn is_local_host_accepts_loopback_names() {
        assert!(is_local_host("localhost:3000"));
        assert!(is_local_host("127.0.0.1:3000"));
        assert!(is_local_host("[::1]:3000"));
        assert!(is_local_host("::1"));
        assert!(!is_local_host("krusty.example.ts.net"));
    }

    #[test]
    fn resolve_workspace_dir_rejects_paths_outside_server_root() {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-auth-test-{}", uuid::Uuid::new_v4()));
        let server_root = temp_dir.join("workspace");
        let outside = temp_dir.join("outside");
        std::fs::create_dir_all(&server_root).expect("server root should exist");
        std::fs::create_dir_all(&outside).expect("outside dir should exist");

        let result = resolve_workspace_dir(&server_root, Some(outside.to_string_lossy().as_ref()));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_workspace_dir_allows_relative_paths_within_server_root() {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-auth-test-{}", uuid::Uuid::new_v4()));
        let server_root = temp_dir.join("workspace");
        let project_dir = server_root.join("project");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");

        let result = resolve_workspace_dir(&server_root, Some("project"))
            .expect("relative project dir should resolve");
        assert_eq!(result, PathBuf::from(project_dir));
    }

    fn test_state() -> AppState {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-auth-test-{}", uuid::Uuid::new_v4()));
        let db_path = temp_dir.join("test.db");
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        krusty_core::storage::Database::new(&db_path).expect("database should initialize");

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
            remote_access: Arc::new(RwLock::new(RemoteAccessConfig {
                enabled: true,
                token: "secret".to_string(),
            })),
            active_agent_streams: Arc::new(AtomicUsize::new(0)),
            peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            push_service: None,
            oauth_flows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tokio::test]
    async fn authorize_request_surface_rejects_remote_without_token() {
        let state = test_state();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)), 3000);
        let headers = HeaderMap::new();

        let result = authorize_request_surface(&state, addr, &headers).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn authorize_request_surface_accepts_remote_with_valid_token() {
        let state = test_state();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)), 3000);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );

        let result = authorize_request_surface(&state, addr, &headers).await;
        assert!(result.is_ok());
    }
}
