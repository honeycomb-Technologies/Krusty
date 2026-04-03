use std::sync::atomic::Ordering;

use axum::{extract::State, routing::get, Json, Router};

use krusty_core::storage::Database;
use krusty_core::SessionManager;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::presence::snapshot_presence;
use crate::types::{
    ActiveSessionStatusResponse, ServerAccessResponse, ServerMemoryStatusResponse,
    ServerStatusResponse, TailscaleAccessResponse, UpdateServerAccessRequest,
};
use crate::{observe_process_memory, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/access",
            get(get_server_access).patch(update_server_access),
        )
        .route("/status", get(get_server_status))
}

async fn get_server_access(
    State(state): State<AppState>,
) -> Result<Json<ServerAccessResponse>, AppError> {
    let remote_access = state.remote_access.read().await.clone();
    let tailscale = tailscale_status(state.server_port);

    Ok(Json(ServerAccessResponse {
        local_url: format!("http://localhost:{}", state.server_port),
        remote_access_enabled: remote_access.enabled,
        remote_access_token: remote_access.token.clone(),
        remote_launch_url: tailscale
            .url
            .as_ref()
            .map(|url| format!("{url}/#krusty-remote-token={}", remote_access.token)),
        tailscale,
    }))
}

async fn update_server_access(
    State(state): State<AppState>,
    Json(req): Json<UpdateServerAccessRequest>,
) -> Result<Json<ServerAccessResponse>, AppError> {
    let mut remote_access = state.remote_access.write().await;

    if let Some(enabled) = req.enabled {
        remote_access.enabled = enabled;
    }
    if req.rotate_token.unwrap_or(false) {
        remote_access.rotate(&state.db_path)?;
    } else {
        remote_access.persist(&state.db_path)?;
    }

    let tailscale = tailscale_status(state.server_port);
    Ok(Json(ServerAccessResponse {
        local_url: format!("http://localhost:{}", state.server_port),
        remote_access_enabled: remote_access.enabled,
        remote_access_token: remote_access.token.clone(),
        remote_launch_url: tailscale
            .url
            .as_ref()
            .map(|url| format!("{url}/#krusty-remote-token={}", remote_access.token)),
        tailscale,
    }))
}

async fn get_server_status(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<ServerStatusResponse>, AppError> {
    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let mut presence = state.session_presence.write().await;

    let mut active_sessions = session_manager
        .list_active_sessions()?
        .into_iter()
        .filter_map(|(session_id, agent_state)| {
            if !session_manager
                .verify_session_ownership(&session_id, user_id)
                .unwrap_or(false)
                && user_id.is_some()
            {
                return None;
            }

            let session = session_manager.get_session(&session_id).ok().flatten()?;
            let presence_snapshot = snapshot_presence(&mut presence, &session_id);

            Some(ActiveSessionStatusResponse {
                id: session_id,
                title: session.title,
                agent_state: agent_state.state,
                started_at: agent_state.started_at,
                last_event_at: agent_state.last_event_at,
                working_dir: session.working_dir,
                project_dir: session.project_dir,
                workspace_mode: session.workspace_mode,
                active_viewers: presence_snapshot.active_viewers,
                active_controllers: presence_snapshot.active_controllers,
                stale_clients: presence_snapshot.stale_clients,
            })
        })
        .collect::<Vec<_>>();
    active_sessions.sort_by(|left, right| left.id.cmp(&right.id));
    let memory = observe_process_memory(&state);

    Ok(Json(ServerStatusResponse {
        active_agent_streams: state.active_agent_streams.load(Ordering::Relaxed),
        active_sessions,
        memory: ServerMemoryStatusResponse {
            rss_bytes: memory.rss_bytes,
            virtual_bytes: memory.virtual_bytes,
            peak_rss_bytes: Some(state.peak_rss_bytes.load(Ordering::Relaxed)),
            peak_virtual_bytes: Some(state.peak_virtual_bytes.load(Ordering::Relaxed)),
        },
        tailscale: tailscale_status(state.server_port),
    }))
}

fn tailscale_status(port: u16) -> TailscaleAccessResponse {
    if !krusty_core::tailscale::is_installed() {
        return TailscaleAccessResponse {
            status: "not_installed".to_string(),
            url: None,
            detail: None,
        };
    }

    match krusty_core::tailscale::device_info() {
        Ok(info) if !info.online => TailscaleAccessResponse {
            status: "offline".to_string(),
            url: None,
            detail: None,
        },
        Ok(_) => TailscaleAccessResponse {
            status: "available".to_string(),
            url: krusty_core::tailscale::device_url(port).ok(),
            detail: None,
        },
        Err(error) => TailscaleAccessResponse {
            status: "failed".to_string(),
            url: None,
            detail: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{atomic::AtomicUsize, Arc};

    use axum::extract::State;
    use axum::Json;
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::Database;
    use krusty_core::tools::registry::ToolRegistry;

    use super::{get_server_access, update_server_access};
    use crate::types::UpdateServerAccessRequest;
    use crate::AppState;

    fn create_test_state() -> (AppState, std::path::PathBuf) {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-server-test-{}", uuid::Uuid::new_v4()));
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
                    token: "test-token".to_string(),
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
    async fn update_server_access_rotates_token() {
        let (state, _temp_dir) = create_test_state();
        let Json(before) = match get_server_access(State(state.clone())).await {
            Ok(response) => response,
            Err(_) => panic!("access request should succeed"),
        };

        let Json(after) = update_server_access(
            State(state),
            Json(UpdateServerAccessRequest {
                enabled: None,
                rotate_token: Some(true),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("rotate request should succeed"));

        assert_ne!(before.remote_access_token, after.remote_access_token);
    }
}
