//! Tool execution endpoint

use std::path::PathBuf;

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use super::session_access::request_workspace_scope;
use krusty_core::agent::AgentConfig as RuntimeAgentConfig;
use krusty_core::tools::registry::{PermissionMode, ToolContext};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{ToolExecuteRequest, ToolExecuteResponse};
use crate::utils::workspace::resolve_scoped_workspace_path;
use crate::AppState;

/// Build the tools router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tools))
        .route("/execute", post(execute_tool))
}

/// Tool info for API response
#[derive(Serialize)]
pub struct ToolResponse {
    pub name: String,
    pub description: String,
}

/// List all available tools
async fn list_tools(State(state): State<AppState>) -> Json<Vec<ToolResponse>> {
    let tools = state.tool_registry.get_ai_tools().await;

    let response: Vec<ToolResponse> = tools
        .into_iter()
        .map(|t| ToolResponse {
            name: t.name,
            description: t.description,
        })
        .collect();

    Json(response)
}

/// Execute a tool
async fn execute_tool(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ToolExecuteRequest>,
) -> Result<Json<ToolExecuteResponse>, AppError> {
    let working_dir = resolve_tool_working_dir(&state, user.as_ref(), req.working_dir.as_deref())?;

    // Create tool context
    let ctx = ToolContext {
        working_dir,
        process_registry: Some(state.process_registry.clone()),
        plan_mode: req.mode == Some(krusty_core::storage::WorkMode::Plan),
        ..Default::default()
    }
    .with_permission_mode(PermissionMode::Autonomous)
    .with_subagent_max_turns(RuntimeAgentConfig::default().subagent_max_turns)
    .with_mcp_manager(state.mcp_manager.clone())
    .with_skills_manager(state.skills_manager.clone());

    let ctx = if let Some(client) = state.resolve_ai_client(None).await {
        ctx.with_ai_client(client)
    } else {
        ctx
    };

    // Execute tool
    let result = state
        .tool_registry
        .execute(&req.tool_name, req.params, &ctx)
        .await
        .ok_or_else(|| AppError::NotFound(format!("Tool '{}' not found", req.tool_name)))?;

    Ok(Json(ToolExecuteResponse {
        output: result.output,
        is_error: result.is_error,
    }))
}

fn resolve_tool_working_dir(
    state: &AppState,
    user: Option<&CurrentUser>,
    requested: Option<&str>,
) -> Result<PathBuf, AppError> {
    let workspace_scope = request_workspace_scope(state, user);
    resolve_scoped_workspace_path(
        requested,
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::Database;
    use krusty_core::tools::registry::ToolRegistry;

    use super::resolve_tool_working_dir;
    use crate::auth::{AuthenticatedUser, CurrentUser};
    use crate::error::AppError;
    use crate::AppState;

    fn create_test_state() -> (AppState, PathBuf) {
        let temp_dir =
            std::env::temp_dir().join(format!("krusty-server-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let db_path = temp_dir.join("krusty.db");
        Database::new(&db_path).expect("database should initialize");
        let working_dir = temp_dir.join("workspace");
        std::fs::create_dir_all(&working_dir).expect("workspace should exist");

        (
            AppState {
                server_port: 3000,
                db_path: Arc::new(db_path),
                working_dir: Arc::new(working_dir.clone()),
                ai_client: None,
                tool_registry: Arc::new(ToolRegistry::new()),
                process_registry: Arc::new(ProcessRegistry::new()),
                model_registry: create_model_registry(),
                credential_store: Arc::new(RwLock::new(CredentialStore::default())),
                mcp_manager: Arc::new(McpManager::new(working_dir.clone())),
                hook_manager: Arc::new(RwLock::new(UserHookManager::new())),
                skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(&working_dir))),
                cancellation: AgentCancellation::new(),
                session_locks: Arc::new(RwLock::new(HashMap::new())),
                session_inputs: Arc::new(RwLock::new(HashMap::new())),
                session_presence: Arc::new(RwLock::new(HashMap::new())),
                delegated_state: Arc::new(RwLock::new(HashMap::new())),
                remote_access: Arc::new(RwLock::new(crate::remote_access::RemoteAccessConfig {
                    enabled: true,
                    token: "test-token".to_string(),
                })),
                active_agent_streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                peak_rss_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                peak_virtual_bytes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                push_service: None,
                apns_service: None,
                oauth_flows: Arc::new(Mutex::new(HashMap::new())),
                mako_runtime: crate::mako_runtime::MakoRuntimeManager::new(),
            },
            temp_dir,
        )
    }

    fn current_user(user_id: &str, home_dir: &std::path::Path) -> CurrentUser {
        CurrentUser(AuthenticatedUser {
            user_id: Some(user_id.to_string()),
            home_dir: Some(home_dir.to_path_buf()),
        })
    }

    #[tokio::test]
    async fn resolve_tool_working_dir_rejects_paths_outside_user_root() {
        let (state, temp_dir) = create_test_state();
        let user_root = temp_dir.join("user");
        let outside_root = temp_dir.join("outside");
        std::fs::create_dir_all(&user_root).expect("user root should exist");
        std::fs::create_dir_all(&outside_root).expect("outside root should exist");

        let result = resolve_tool_working_dir(
            &state,
            Some(&current_user("alice", &user_root)),
            Some(outside_root.to_string_lossy().as_ref()),
        );

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn resolve_tool_working_dir_allows_relative_paths_within_user_root() {
        let (state, temp_dir) = create_test_state();
        let user_root = temp_dir.join("user");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let result = resolve_tool_working_dir(
            &state,
            Some(&current_user("alice", &user_root)),
            Some("repo"),
        );
        let working_dir = match result {
            Ok(working_dir) => working_dir,
            Err(_) => panic!("working dir should resolve"),
        };

        assert_eq!(working_dir, repo_dir);
    }
}
