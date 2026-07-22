use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use tokio::sync::{Mutex, RwLock};

use krusty_core::agent::loop_events::{LoopEvent, LoopStopReason};
use krusty_core::agent::{
    effective_context_window_for_runtime, AgentCancellation, LoopInput, UserHookManager,
};
use krusty_core::ai::models::{
    create_model_registry, ApiFormat, ModelCatalogSource, ModelKey, ModelMetadata,
};
use krusty_core::ai::providers::ProviderId;
use krusty_core::mcp::McpManager;
use krusty_core::plan::{PlanFile, PlanManager};
use krusty_core::process::ProcessRegistry;
use krusty_core::skills::SkillsManager;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::storage::{
    Database, PartialAssistantState, PendingInteractionSnapshot, PendingPlanTaskSnapshot,
    RecoveryDecision, RecoveryNonResumableReason, RecoveryStatus, RecoveryToolCall,
    RuntimeTraceEvent, RuntimeTraceStore, SessionRecoveryState, SessionType, WorkspaceMode,
};
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
async fn generic_session_routes_reject_daemon_owned_mako_create_update_and_pinch() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let create = create_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Json(CreateSessionRequest {
            title: Some("Orphan Mako".into()),
            model: Some("test:model".into()),
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: Some(SessionType::Mako),
            permission_mode: None,
        }),
    )
    .await;
    assert!(matches!(create, Err(AppError::Conflict(_))));

    let manager = SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    assert!(manager
        .list_sessions_for_user_by_type(None, Some("alice"), SessionType::Mako)
        .expect("sessions should list")
        .is_empty());
    let session_id = manager
        .create_session_for_user_with_config(
            "Daemon Mako",
            Some("test:model"),
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("test Mako session should create");

    let update = update_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(session_id.clone()),
        Json(UpdateSessionRequest {
            title: Some("Bypassed title".into()),
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            mode: None,
            model: None,
            model_key: None,
            target_branch: None,
            permission_mode: None,
        }),
    )
    .await;
    assert!(matches!(update, Err(AppError::Conflict(_))));

    let pinch = pinch_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(session_id.clone()),
        Json(PinchRequest {
            preservation_hints: None,
            direction: None,
        }),
    )
    .await;
    assert!(matches!(pinch, Err(AppError::Conflict(_))));
    assert_eq!(
        manager
            .get_session(&session_id)
            .expect("session should load")
            .expect("session should exist")
            .title,
        "Daemon Mako"
    );
}

#[tokio::test]
async fn session_create_persists_full_continuation_contract() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let project_parent = user_root.join("projects");
    std::fs::create_dir_all(&project_parent).expect("project parent should exist");
    let project_dir = project_parent.join("continuation-contract");

    let (_, Json(created)) = create_session(
        State(state.clone()),
        Some(current_user("alice", &user_root)),
        Json(CreateSessionRequest {
            title: Some("Continuation Contract".to_string()),
            model: Some("openai/gpt-5.5".to_string()),
            model_key: None,
            project_dir: Some(project_dir.to_string_lossy().to_string()),
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Created),
            target_branch: Some("feature/continue".to_string()),
            session_type: Some(SessionType::Code),
            permission_mode: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("session creation should succeed"));

    let expected_project = project_dir.to_string_lossy().to_string();
    assert_eq!(created.session_type, SessionType::Code);
    assert_eq!(created.workspace_mode, WorkspaceMode::Created);
    assert_eq!(
        created.working_dir.as_deref(),
        Some(expected_project.as_str())
    );
    assert_eq!(
        created.project_dir.as_deref(),
        Some(expected_project.as_str())
    );
    assert_eq!(created.target_branch.as_deref(), Some("feature/continue"));
    assert_eq!(created.model.as_deref(), Some("openai/gpt-5.5"));

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let persisted = session_manager
        .get_session(&created.id)
        .expect("session lookup should succeed")
        .expect("session should exist");
    assert_eq!(persisted.session_type, SessionType::Code);
    assert_eq!(persisted.workspace_mode, WorkspaceMode::Created);
    assert_eq!(
        persisted.working_dir.as_deref(),
        Some(expected_project.as_str())
    );
    assert_eq!(
        persisted.project_dir.as_deref(),
        Some(expected_project.as_str())
    );
    assert_eq!(persisted.target_branch.as_deref(), Some("feature/continue"));
    assert_eq!(persisted.model.as_deref(), Some("openai/gpt-5.5"));
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
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            project_dir: Some("repo".to_string()),
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Selected),
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            project_dir: Some(fresh_project_dir.to_string_lossy().to_string()),
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Selected),
            target_branch: None,
            session_type: Some(SessionType::Code),
            permission_mode: None,
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
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Selected),
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            project_dir: Some("/tmp/repo".to_string()),
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Neutral),
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
async fn session_cancel_signals_the_owned_active_run() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Active Session", None, None, Some("alice"))
        .expect("session creation should succeed");
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    state
        .session_inputs
        .write()
        .await
        .insert(session_id.clone(), input_tx);

    let Json(response) = cancel_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(session_id),
    )
    .await
    .unwrap_or_else(|_| panic!("owned cancellation should succeed"));

    assert!(response.ok);
    assert!(matches!(input_rx.recv().await, Some(LoopInput::Cancel)));
}

#[tokio::test]
async fn session_cancel_rejects_a_foreign_owner_without_signalling() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    create_test_user(&state, "bob");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Alice Session", None, None, Some("alice"))
        .expect("session creation should succeed");
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    state
        .session_inputs
        .write()
        .await
        .insert(session_id.clone(), input_tx);

    let result = cancel_session(
        State(state.clone()),
        Some(current_user("bob", state.working_dir.as_ref())),
        Path(session_id),
    )
    .await;

    assert!(matches!(result, Err(AppError::NotFound(_))));
    assert!(input_rx.try_recv().is_err());
}

#[tokio::test]
async fn session_cancel_is_idempotent_when_the_run_is_inactive() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Idle Session", None, None, Some("alice"))
        .expect("session creation should succeed");

    let Json(response) = cancel_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(session_id),
    )
    .await
    .unwrap_or_else(|_| panic!("idle cancellation should be idempotent"));

    assert!(response.ok);
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
async fn load_owned_session_rejects_authenticated_session_for_local_actor() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Alice Session", None, None, Some("alice"))
        .expect("session creation should succeed");

    let result = super::load_owned_session(&session_manager, &session_id, None);

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
async fn session_state_exposes_recovery_live_partial_and_trace_sequence() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Recoverable Session",
            Some("openai/gpt-5.5"),
            None,
            None,
            WorkspaceMode::Neutral,
            Some("alice"),
            None,
            SessionType::Code,
        )
        .expect("session creation should succeed");
    session_manager
        .set_agent_state(&session_id, "streaming")
        .expect("agent state should update");

    let recovery = SessionRecoveryState::new(
        RecoveryStatus::Streaming,
        Some(LoopStopReason::StreamIdleTimeout),
        None,
        PartialAssistantState {
            text: "partial answer".to_string(),
            thinking: "working notes".to_string(),
            tool_calls: vec![RecoveryToolCall::summary("tool-1", "edit")],
        },
        RecoveryDecision::Resumable {
            latest_user_objective: "finish the contract harness".to_string(),
        },
    );
    session_manager
        .update_recovery_state(&session_id, &recovery)
        .expect("recovery state should persist");

    let trace_event = RuntimeTraceEvent::from_loop_event(
        "run-1",
        42,
        1,
        &LoopEvent::TextDelta {
            delta: "partial answer".to_string(),
        },
    );
    RuntimeTraceStore::new(session_manager.db())
        .append_event(&session_id, &trace_event)
        .expect("runtime trace should persist");

    let Json(response) = get_session_state(
        State(state),
        Some(current_user("alice", std::path::Path::new("/tmp"))),
        Path(session_id.clone()),
    )
    .await
    .unwrap_or_else(|_| panic!("session state should load"));

    assert_eq!(response.id, session_id);
    assert_eq!(response.agent_state, "streaming");
    assert_eq!(response.recovery.as_ref(), Some(&recovery));
    assert_eq!(
        response
            .live_partial_assistant
            .as_ref()
            .map(|partial| partial.text.as_str()),
        Some("partial answer")
    );
    assert_eq!(response.last_event_sequence, Some(42));
}

fn seed_trace_snapshot(state: &AppState, session_id: &str) -> Vec<RuntimeTraceEvent> {
    let db = Database::new(&state.db_path).expect("database should open");
    let store = RuntimeTraceStore::new(&db);
    let events = vec![
        LoopEvent::ThinkingDelta {
            thinking: "inspect".to_string(),
        },
        LoopEvent::TextDelta {
            delta: "answer".to_string(),
        },
        LoopEvent::TurnComplete {
            turn: 1,
            has_more: false,
        },
        LoopEvent::Finished {
            session_id: session_id.to_string(),
            stop_reason: LoopStopReason::Completed,
        },
        LoopEvent::TitleGenerated {
            title: "Snapshot complete".to_string(),
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, event)| {
        RuntimeTraceEvent::from_loop_event(
            "snapshot-run",
            i64::try_from(index + 1).expect("small sequence"),
            1,
            &event,
        )
    })
    .collect::<Vec<_>>();

    for event in &events {
        store
            .append_event(session_id, event)
            .expect("trace event should persist");
    }
    events
}

#[tokio::test]
async fn session_trace_latest_limit_uses_one_coherent_snapshot() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user(
            "Trace Snapshot",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some("alice"),
        )
        .expect("session should create");
    seed_trace_snapshot(&state, &session_id);

    let Json(response) = get_session_trace(
        State(state),
        Some(current_user("alice", &temp_dir)),
        Path(session_id),
        Query(state::GetSessionTraceQuery {
            limit: Some(2),
            after_sequence: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("trace endpoint should succeed"));

    assert_eq!(response.summary.total_events, 5);
    assert_eq!(
        response.summary.last_stop_reason,
        Some(LoopStopReason::Completed)
    );
    assert_eq!(response.latest_sequence, Some(5));
    assert_eq!(
        response
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
}

#[tokio::test]
async fn session_trace_after_sequence_applies_limit_without_changing_snapshot_metadata() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user(
            "Incremental Trace Snapshot",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some("alice"),
        )
        .expect("session should create");
    seed_trace_snapshot(&state, &session_id);

    let Json(response) = get_session_trace(
        State(state),
        Some(current_user("alice", &temp_dir)),
        Path(session_id),
        Query(state::GetSessionTraceQuery {
            limit: Some(2),
            after_sequence: Some(2),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("trace endpoint should succeed"));

    assert_eq!(response.summary.total_events, 5);
    assert_eq!(
        response.summary.last_stop_reason,
        Some(LoopStopReason::Completed)
    );
    assert_eq!(response.latest_sequence, Some(5));
    assert_eq!(
        response
            .events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
}

#[tokio::test]
async fn get_session_state_exposes_awaiting_input_details_after_reload() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Awaiting Input Session",
            Some("openai/gpt-5.5"),
            None,
            None,
            WorkspaceMode::Neutral,
            Some("alice"),
            None,
            SessionType::Code,
        )
        .expect("session creation should succeed");
    session_manager
        .set_agent_state(&session_id, "idle")
        .expect("agent state should simulate a fresh server process");

    let pending_interactions = vec![
        PendingInteractionSnapshot::ask_user_from_call(
            "ask-1",
            &serde_json::json!({
                "questions": [{
                    "header": "Choose deploy target",
                    "question": "Which environment should Krusty continue against?",
                    "options": [{ "label": "staging", "description": "Safe validation" }],
                    "multi_select": false
                }]
            }),
        ),
        PendingInteractionSnapshot::tool_approval_from_call(
            "tool-1",
            "edit",
            &serde_json::json!({
                "file_path": "src/lib.rs",
                "api_token": "super-secret-token",
                "content": "raw file content should not be replayed to clients"
            }),
        ),
        PendingInteractionSnapshot::plan_confirm(
            "plan-1",
            "Ship reload-safe prompts",
            2,
            vec![PendingPlanTaskSnapshot {
                description: "Expose the server contract".to_string(),
                completed: false,
            }],
        ),
    ];
    let recovery = SessionRecoveryState::new_with_pending_interactions(
        RecoveryStatus::AwaitingInput,
        Some(LoopStopReason::AwaitingInput),
        None,
        PartialAssistantState::default(),
        pending_interactions.clone(),
        RecoveryDecision::NonResumable {
            reason: RecoveryNonResumableReason::AwaitingHumanInput,
        },
    );
    session_manager
        .update_recovery_state(&session_id, &recovery)
        .expect("recovery state should persist");

    let Json(response) = get_session_state(
        State(state),
        Some(current_user("alice", std::path::Path::new("/tmp"))),
        Path(session_id.clone()),
    )
    .await
    .unwrap_or_else(|_| panic!("session state should load"));

    assert_eq!(response.id, session_id);
    assert_eq!(response.agent_state, "idle");
    assert_eq!(response.recovery.as_ref(), Some(&recovery));
    assert_eq!(response.pending_interactions, pending_interactions);

    let PendingInteractionSnapshot::ToolApproval { tool_call } = &response.pending_interactions[1]
    else {
        panic!("expected a tool approval pending interaction");
    };
    assert_eq!(tool_call.arguments.value["file_path"], "src/lib.rs");
    assert_eq!(tool_call.arguments.value["api_token"], "[REDACTED]");
    assert!(tool_call
        .arguments
        .redacted_paths
        .contains(&"$.api_token".to_string()));
}

#[tokio::test]
async fn pinch_model_contract_resolves_persisted_exact_key_and_runtime_context() {
    let (state, _temp_dir) = create_test_state();
    let narrow = ModelMetadata::new("shared-pinch", "Narrow", ProviderId::MiniMax)
        .with_context(8_192, 2_048)
        .with_transport(ApiFormat::Anthropic);
    let exact = ModelMetadata::new("shared-pinch", "Exact", ProviderId::OpenRouter)
        .with_context(500_000, 32_768)
        .with_transport(ApiFormat::OpenAI);
    let exact_key = exact.key();
    state
        .model_registry
        .set_models(ProviderId::MiniMax, vec![narrow])
        .await;
    state
        .model_registry
        .set_models(ProviderId::OpenRouter, vec![exact])
        .await;
    state
        .credential_store
        .write()
        .await
        .set(ProviderId::OpenRouter, "openrouter-test-key".to_string());

    let manager =
        open_session_manager(&state).unwrap_or_else(|_| panic!("session manager should open"));
    let session_id = manager
        .create_session("Exact pinch", Some("shared-pinch"), None)
        .expect("session should create");
    manager
        .update_session_model_selection(&session_id, Some(&exact_key), Some("pinch-runtime"))
        .expect("exact selection should persist");

    let source = manager
        .get_session(&session_id)
        .expect("session should load")
        .expect("session should exist");
    let persisted_key = source.model_key.as_ref().expect("exact key should load");
    let client = state
        .resolve_ai_client_for_key_for_user(persisted_key, source.user_id.as_deref())
        .await
        .expect("exact pinch client should resolve");
    let effective_window = effective_context_window_for_runtime(
        client.config().uses_chatgpt_codex_format(),
        client.resolved_model().capabilities.context_window,
    );

    assert_eq!(client.resolved_model().key, exact_key);
    assert_eq!(client.provider_id(), ProviderId::OpenRouter);
    assert_eq!(effective_window, 500_000);
}

#[tokio::test]
async fn pinch_session_includes_project_context_and_ranked_files() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let exact_metadata = ModelMetadata::new(
        "claude-3-5-sonnet",
        "Claude 3.5 Sonnet",
        ProviderId::Anthropic,
    )
    .with_context(200_000, 8_192)
    .with_transport(ApiFormat::Anthropic);
    let exact_key = exact_metadata.key();
    state
        .model_registry
        .set_models_with_catalog(
            ProviderId::Anthropic,
            vec![exact_metadata],
            Some(ModelCatalogSource::Curated),
            Some("pinch-catalog".to_string()),
        )
        .await;

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
    session_manager
        .update_session_model_selection(&session_id, Some(&exact_key), Some("pinch-catalog"))
        .expect("exact pinch model should persist");

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

    assert_eq!(response.session.id, session_id);
    assert_eq!(response.session.model_key, Some(exact_key));
    assert_eq!(
        response.session.model_catalog_revision.as_deref(),
        Some("pinch-catalog")
    );
    assert!(response.checkpoint_id.is_some());
    assert!(response.replaced_messages.unwrap_or(0) > 0);

    let messages = session_manager
        .load_session_messages(&response.session.id)
        .expect("compacted messages should load");
    let combined = messages
        .iter()
        .map(|(_, content)| content.as_str())
        .collect::<String>();

    assert!(combined.contains("Conversation Compacted"));
    assert!(combined.contains("Continue the server audit."));
    assert!(combined.contains("src/lib.rs"));
    assert!(
        !combined.contains("Task 1.1: Keep session continuity"),
        "canonical active plan state must be reinjected at request time, not duplicated in history"
    );
    assert!(plan_manager
        .get_active_plan(&session_id)
        .expect("active plan lookup")
        .is_some());
}

#[tokio::test]
async fn pinch_session_rejects_an_active_session_writer() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let workspace = state.working_dir.as_ref();
    let session_manager = match open_session_manager(&state) {
        Ok(manager) => manager,
        Err(_) => panic!("session manager"),
    };
    let session_id = session_manager
        .create_session_for_user(
            "Busy Pinch",
            None,
            Some(workspace.to_string_lossy().as_ref()),
            Some("alice"),
        )
        .expect("session");
    let _guard = state
        .try_lock_session(&session_id)
        .await
        .expect("first session writer lock");

    let result = pinch_session(
        State(state.clone()),
        Some(current_user("alice", workspace)),
        Path(session_id.clone()),
        Json(PinchRequest {
            preservation_hints: None,
            direction: None,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::Conflict(message)) if message.contains("busy")));
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
        Path(session_id.clone()),
        Json(PinchRequest {
            preservation_hints: None,
            direction: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("pinch should succeed"));

    assert_eq!(response.session.id, session_id);

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
async fn create_session_persists_exact_model_key_and_catalog_revision() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let mut metadata = ModelMetadata::new("shared-model", "Shared via Grok", ProviderId::Grok);
    metadata.api_format = ApiFormat::OpenAIResponses;
    let key = ModelKey::from_metadata(&metadata);
    state
        .model_registry
        .set_models_with_catalog(
            ProviderId::Grok,
            vec![metadata],
            Some(ModelCatalogSource::LiveDynamic),
            Some("catalog-exact-test".to_string()),
        )
        .await;

    let (_, Json(created)) = create_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Json(CreateSessionRequest {
            title: Some("Exact model".to_string()),
            model: Some("shared-model".to_string()),
            model_key: Some(key.clone()),
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: None,
            permission_mode: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("exact session creation should succeed"));

    assert_eq!(created.model.as_deref(), Some("shared-model"));
    assert_eq!(created.model_key, Some(key.clone()));
    assert_eq!(
        created.model_catalog_revision.as_deref(),
        Some("catalog-exact-test")
    );
    let persisted = open_session_manager(&state)
        .unwrap_or_else(|_| panic!("session manager should open"))
        .get_session(&created.id)
        .expect("session should load")
        .expect("session should exist");
    assert_eq!(persisted.model_key, Some(key));
    assert_eq!(
        persisted.model_catalog_revision.as_deref(),
        Some("catalog-exact-test")
    );
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
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            target_branch: None,
            permission_mode: None,
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
            model_key: None,
            target_branch: None,
            permission_mode: None,
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
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            target_branch: None,
            permission_mode: None,
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
            model_key: None,
            target_branch: None,
            permission_mode: None,
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
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            target_branch: None,
            permission_mode: None,
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
            model_key: None,
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            target_branch: None,
            session_type: None,
            permission_mode: None,
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
            model_key: None,
            target_branch: None,
            permission_mode: None,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}
