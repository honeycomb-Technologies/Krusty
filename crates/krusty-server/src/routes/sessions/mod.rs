//! Session management endpoints

mod approvals;
mod crud;
mod pinch;
mod presence;
mod state;

use axum::{
    routing::{get, post},
    Router,
};

use krusty_core::storage::Database;
use krusty_core::SessionManager;

use self::approvals::tool_approval_for_session;
use self::crud::{
    create_session, delete_session, get_session, list_directories, list_sessions, update_session,
};
use self::pinch::pinch_session;
use self::presence::{get_session_presence, heartbeat_session_presence, remove_session_presence};
use self::state::{get_session_state, get_session_trace};

use super::session_access::{
    current_user_id, ensure_owned_session, load_agent_state_or_idle, load_owned_session,
    request_workspace_scope,
};
use crate::error::AppError;
use crate::AppState;

/// Build the sessions router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/directories", get(list_directories))
        .route(
            "/:id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/:id/state", get(get_session_state))
        .route("/:id/trace", get(get_session_trace))
        .route(
            "/:id/presence",
            get(get_session_presence).put(heartbeat_session_presence),
        )
        .route(
            "/:id/presence/:client_id",
            axum::routing::delete(remove_session_presence),
        )
        .route("/:id/pinch", post(pinch_session))
        .route("/:id/tool-approval", post(tool_approval_for_session))
}

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
}
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::{Path, Query, State};
    use axum::Json;
    use chrono::Utc;
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::plan::{PlanFile, PlanManager};
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::{Database, SessionType, WorkspaceMode};
    use krusty_core::tools::registry::ToolRegistry;

    use super::crud::{GetSessionQuery, ListSessionsQuery};
    use super::*;
    use crate::auth::{AuthenticatedUser, CurrentUser};
    use crate::types::{
        CreateSessionRequest, PinchRequest, SessionPresenceHeartbeatRequest, UpdateSessionRequest,
    };
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

    fn create_test_user(state: &AppState, user_id: &str) {
        let db = Database::new(&state.db_path).expect("database should open");
        db.conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                (user_id, format!("{user_id}@example.com"), "free"),
            )
            .expect("user should insert");
    }

    fn current_user(user_id: &str, home_dir: &std::path::Path) -> CurrentUser {
        CurrentUser(AuthenticatedUser {
            user_id: Some(user_id.to_string()),
            home_dir: Some(home_dir.to_path_buf()),
        })
    }

    #[tokio::test]
    async fn create_session_persists_user_ownership() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let result = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Owned Session".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await;
        let (_, Json(response)) = match result {
            Ok(response) => response,
            Err(_) => panic!("session creation should succeed"),
        };

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session = session_manager
            .get_session(&response.id)
            .expect("session lookup should succeed")
            .expect("session should exist");

        assert_eq!(session.user_id.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn create_session_resolves_relative_workspace_paths_within_user_root() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        std::fs::create_dir_all(&user_root).expect("user root should exist");

        let (_, Json(created)) = create_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Json(CreateSessionRequest {
                title: Some("Relative Workspace".to_string()),
                model: None,
                project_dir: Some("repo".to_string()),
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Selected),
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let expected = user_root.join("repo").to_string_lossy().to_string();
        assert_eq!(created.project_dir.as_deref(), Some(expected.as_str()));
        assert_eq!(created.working_dir.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn create_session_accepts_fresh_absolute_workspace_path_with_existing_ancestor() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let parent_dir = user_root.join("projects");
        std::fs::create_dir_all(&parent_dir).expect("parent dir should exist");

        let fresh_project_dir = parent_dir.join("fresh-repo");

        let (_, Json(created)) = create_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Json(CreateSessionRequest {
                title: Some("Fresh Workspace".to_string()),
                model: None,
                project_dir: Some(fresh_project_dir.to_string_lossy().to_string()),
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Selected),
                target_branch: None,
                session_type: Some(SessionType::Code),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let expected = fresh_project_dir.to_string_lossy().to_string();
        assert_eq!(created.project_dir.as_deref(), Some(expected.as_str()));
        assert_eq!(created.working_dir.as_deref(), Some(expected.as_str()));
        assert_eq!(created.workspace_mode, WorkspaceMode::Selected);
        assert_eq!(created.session_type, SessionType::Code);
    }

    #[tokio::test]
    async fn create_session_rejects_invalid_workspace_payloads() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let missing_project_dir = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Invalid Selected Workspace".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Selected),
                target_branch: None,
                session_type: None,
            }),
        )
        .await;

        match missing_project_dir {
            Err(AppError::BadRequest(message)) => {
                assert_eq!(
                    message,
                    "workspace modes 'selected' and 'created' require a project_dir"
                );
            }
            Ok(_) => panic!("invalid selected workspace should fail"),
            Err(_) => panic!("invalid selected workspace should fail with bad request"),
        }

        let neutral_with_project = create_session(
            State(state),
            Some(current_user("alice", std::path::Path::new("/tmp"))),
            Json(CreateSessionRequest {
                title: Some("Invalid Neutral Workspace".to_string()),
                model: None,
                project_dir: Some("/tmp/repo".to_string()),
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Neutral),
                target_branch: None,
                session_type: None,
            }),
        )
        .await;

        match neutral_with_project {
            Err(AppError::BadRequest(message)) => {
                assert_eq!(
                    message,
                    "workspace mode 'neutral' cannot include a project_dir"
                );
            }
            Ok(_) => panic!("neutral workspace with project should fail"),
            Err(_) => panic!("neutral workspace with project should fail with bad request"),
        }
    }

    #[tokio::test]
    async fn get_session_rejects_foreign_owner() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        create_test_user(&state, "bob");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user("Owned Session", None, None, Some("alice"))
            .expect("session creation should succeed");
        session_manager
            .save_message(&session_id, "user", r#"[{"type":"text","text":"hello"}]"#)
            .expect("message should save");

        let result = get_session(
            State(state),
            Some(current_user("bob", std::path::Path::new("/tmp"))),
            Path(session_id),
            Query(GetSessionQuery {
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn load_owned_session_rejects_legacy_userless_session_for_authenticated_user() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session("Legacy Session", None, None)
            .expect("session creation should succeed");
        let user = current_user("alice", std::path::Path::new("/tmp"));

        let result = super::load_owned_session(&session_manager, &session_id, Some(&user));

        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_sessions_resolves_relative_working_dir_filter_within_user_root() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        session_manager
            .create_session_for_user_with_config(
                "Scoped Session",
                None,
                Some(repo_dir.to_string_lossy().as_ref()),
                Some(repo_dir.to_string_lossy().as_ref()),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Code,
            )
            .expect("session creation should succeed");

        let Json(response) = list_sessions(
            State(state),
            Some(current_user("alice", &user_root)),
            Query(ListSessionsQuery {
                working_dir: Some("repo".to_string()),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session list should succeed"));

        assert_eq!(response.len(), 1);
        assert_eq!(
            response[0].working_dir.as_deref(),
            Some(repo_dir.to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn presence_heartbeat_tracks_active_controller_for_owned_session() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user("Owned Session", None, None, Some("alice"))
            .expect("session creation should succeed");

        let Json(response) = heartbeat_session_presence(
            State(state),
            Some(current_user("alice", std::path::Path::new("/tmp"))),
            Path(session_id),
            Json(SessionPresenceHeartbeatRequest {
                client_id: "client-1".to_string(),
                surface: "web".to_string(),
                capability: crate::presence::PresenceCapability::Controller,
                last_event_sequence: Some(12),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("presence heartbeat should succeed"));

        assert_eq!(response.active_viewers, 1);
        assert_eq!(response.active_controllers, 1);
        assert_eq!(response.clients.len(), 1);
        assert_eq!(response.clients[0].client_id, "client-1");
        assert_eq!(response.clients[0].last_event_sequence, Some(12));
    }

    #[tokio::test]
    async fn pinch_session_includes_project_context_and_ranked_files() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let workspace = state.working_dir.as_ref();
        let source_file = workspace.join("src/lib.rs");
        std::fs::create_dir_all(source_file.parent().expect("parent dir should exist"))
            .expect("src dir should exist");
        std::fs::write(
            workspace.join("AGENTS.md"),
            "# Workspace Rules\nPreserve session context.\n",
        )
        .expect("project instructions should write");
        std::fs::write(
            &source_file,
            "pub fn important() -> &'static str { \"hello\" }\n",
        )
        .expect("source file should write");

        let session_manager = match open_session_manager(&state) {
            Ok(session_manager) => session_manager,
            Err(_) => panic!("session manager should open"),
        };
        let session_id = session_manager
            .create_session_for_user(
                "Pinch Source",
                Some("claude-3-5-sonnet"),
                Some(workspace.to_string_lossy().as_ref()),
                Some("alice"),
            )
            .expect("session should create");

        let user_message =
            serde_json::json!([{ "type": "text", "text": "Continue refining the server pinch flow." }])
                .to_string();
        let assistant_message =
            serde_json::json!([{ "type": "text", "text": "I inspected the route and found missing continuation context." }])
                .to_string();
        session_manager
            .save_message(&session_id, "user", &user_message)
            .expect("user message should save");
        session_manager
            .save_message(&session_id, "assistant", &assistant_message)
            .expect("assistant message should save");

        let plan_manager =
            PlanManager::new((*state.db_path).clone()).expect("plan manager should open");
        let plan = PlanFile::from_markdown(
            r#"# Plan: Server Pinch Follow-up

Created: 2026-04-06 12:00 UTC
Session: placeholder
Working Directory: placeholder
Status: in_progress

---

## Phase 1: Continuation

- [ ] Task 1.1: Keep session continuity
"#,
        )
        .expect("plan should parse");
        plan_manager
            .save_plan_for_session(&session_id, &plan)
            .expect("plan should save");

        session_manager
            .db()
            .conn()
            .execute(
                "INSERT INTO file_activity (session_id, file_path, read_count, write_count, edit_count, last_accessed, user_referenced)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    &session_id,
                    "src/lib.rs",
                    2_i64,
                    1_i64,
                    0_i64,
                    Utc::now().to_rfc3339(),
                    1_i64,
                ),
            )
            .expect("file activity should insert");

        let Json(response) = pinch_session(
            State(state.clone()),
            Some(current_user("alice", workspace)),
            Path(session_id.clone()),
            Json(PinchRequest {
                preservation_hints: Some("Keep the route semantics intact.".to_string()),
                direction: Some("Continue the server audit.".to_string()),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("pinch should succeed"));

        let messages = session_manager
            .load_session_messages(&response.session.id)
            .expect("child messages should load");
        let (role, system_message_json) = messages.first().expect("system message should exist");

        assert_eq!(role, "system");
        assert!(system_message_json.contains("Project Instructions"));
        assert!(system_message_json.contains("[PROJECT INSTRUCTIONS -"));
        assert!(system_message_json.contains("Key Files (by importance)"));
        assert!(system_message_json.contains("src/lib.rs"));
        assert!(system_message_json.contains("Key File Contents (Pre-loaded)"));
        assert!(system_message_json.contains("pub fn important()"));
        assert!(system_message_json.contains("## Active Plan"));
        assert!(system_message_json.contains("Task 1.1: Keep session continuity"));
    }

    #[tokio::test]
    async fn pinch_session_resolves_legacy_relative_working_dir_against_user_home() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let repo_dir = user_root.join("repo");
        std::fs::create_dir_all(&repo_dir).expect("repo dir should exist");

        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let session_id = session_manager
            .create_session_for_user_with_config(
                "Legacy Relative Session",
                None,
                Some("repo"),
                Some("repo"),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Code,
            )
            .expect("session should create");
        let user_message =
            serde_json::json!([{ "type": "text", "text": "Continue from the last session." }])
                .to_string();
        let assistant_message =
            serde_json::json!([{ "type": "text", "text": "I will resume the work." }]).to_string();
        session_manager
            .save_message(&session_id, "user", &user_message)
            .expect("user message should save");
        session_manager
            .save_message(&session_id, "assistant", &assistant_message)
            .expect("assistant message should save");

        let Json(response) = pinch_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Path(session_id),
            Json(PinchRequest {
                preservation_hints: None,
                direction: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("pinch should succeed"));

        let expected = repo_dir.to_string_lossy().to_string();
        assert_eq!(
            response.session.working_dir.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            response.session.project_dir.as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn live_partial_assistant_only_surfaces_for_active_states() {
        let recovery = krusty_core::storage::SessionRecoveryState::new(
            krusty_core::storage::RecoveryStatus::Streaming,
            None,
            None,
            krusty_core::storage::PartialAssistantState {
                text: "partial".to_string(),
                thinking: "reasoning".to_string(),
                tool_calls: Vec::new(),
            },
            krusty_core::storage::RecoveryDecision::Resumable {
                latest_user_objective: "finish task".to_string(),
            },
        );

        assert!(super::state::live_partial_assistant_for_state("idle", Some(&recovery)).is_none());

        let live = super::state::live_partial_assistant_for_state("streaming", Some(&recovery))
            .expect("active state should surface live partial");
        assert_eq!(live.text, "partial");
        assert_eq!(live.thinking, "reasoning");
    }

    #[tokio::test]
    async fn session_routes_normalize_blank_model_input_to_none() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Whitespace Model".to_string()),
                model: Some("   ".to_string()),
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        assert_eq!(created.model, None);

        let Json(updated) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id.clone()),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                mode: None,
                model: Some("  gpt-5  ".to_string()),
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session update should succeed"));

        assert_eq!(updated.model.as_deref(), Some("gpt-5"));

        let Json(cleared) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                mode: None,
                model: Some("   ".to_string()),
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session update should succeed"));

        assert_eq!(cleared.model, None);
    }

    #[tokio::test]
    async fn session_routes_apply_workspace_updates() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let project_dir = state.working_dir.join("demo-app");
        std::fs::create_dir_all(&project_dir).expect("project dir should exist");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Workspace Update".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let Json(updated) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id.clone()),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: Some(project_dir.to_string_lossy().to_string()),
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Created),
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("workspace update should succeed"));

        assert_eq!(
            updated.project_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            updated.working_dir.as_deref(),
            Some(project_dir.to_string_lossy().as_ref())
        );
        assert_eq!(updated.workspace_mode, WorkspaceMode::Created);

        let Json(neutral) = update_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Neutral),
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("neutral workspace update should succeed"));

        assert_eq!(neutral.project_dir, None);
        assert_eq!(neutral.working_dir, None);
        assert_eq!(neutral.workspace_mode, WorkspaceMode::Neutral);
    }

    #[tokio::test]
    async fn session_routes_reject_invalid_workspace_payloads() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(CreateSessionRequest {
                title: Some("Workspace Validation".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let result = update_session(
            State(state),
            Some(current_user("alice", std::path::Path::new("/tmp"))),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: Some(WorkspaceMode::Created),
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await;

        match result {
            Err(AppError::BadRequest(message)) => {
                assert_eq!(
                    message,
                    "workspace modes 'selected' and 'created' require a project_dir"
                );
            }
            Ok(_) => panic!("invalid workspace update should fail"),
            Err(_) => panic!("invalid workspace update should fail with bad request"),
        }
    }

    #[tokio::test]
    async fn session_routes_reject_working_dir_updates_outside_user_root() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        let outside_root = temp_dir.join("outside");
        std::fs::create_dir_all(&user_root).expect("user root should exist");
        std::fs::create_dir_all(&outside_root).expect("outside root should exist");

        let (_, Json(created)) = create_session(
            State(state.clone()),
            Some(current_user("alice", &user_root)),
            Json(CreateSessionRequest {
                title: Some("Workspace Validation".to_string()),
                model: None,
                project_dir: None,
                working_dir: None,
                workspace_mode: None,
                target_branch: None,
                session_type: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("session creation should succeed"));

        let result = update_session(
            State(state),
            Some(current_user("alice", &user_root)),
            Path(created.id),
            Json(UpdateSessionRequest {
                title: None,
                project_dir: None,
                working_dir: Some(outside_root.to_string_lossy().to_string()),
                workspace_mode: None,
                mode: None,
                model: None,
                target_branch: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }
}
