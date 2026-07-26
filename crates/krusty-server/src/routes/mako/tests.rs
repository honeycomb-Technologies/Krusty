use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use tokio::sync::{Mutex, RwLock};

use krusty_core::agent::{
    loop_events::LoopStopReason, AgentCancellation, DelegatedRunStage, LoopEvent, UserHookManager,
};
use krusty_core::ai::models::{create_model_registry, ApiFormat, ModelAuthScope, ModelMetadata};
use krusty_core::ai::providers::ProviderId;
use krusty_core::mcp::McpManager;
use krusty_core::paths;
use krusty_core::process::ProcessRegistry;
use krusty_core::skills::SkillsManager;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::storage::reports::CreateReportInput;
use krusty_core::storage::{
    bootstrap_mako_home, AutonomousTaskStore, Database, DelegatedRunRole, DelegatedRunScope,
    DelegatedRunStartInput, DelegatedRunStore, MakoProfileDocumentKind, MakoProfileOwner,
    MakoProfileStore, MakoRunPriority, MakoRuntimeStateStatus, MakoRuntimeStateStore, MemoryStore,
    MemoryType, Preferences, ReportStore, RuntimeTraceEvent, RuntimeTraceStore, SessionType,
    WorkspaceMode, CURRENT_SNAPSHOT_TITLE,
};
use krusty_core::tools::registry::ToolRegistry;
use krusty_core::SessionManager;

use super::attention::{attention, AttentionQuery};
use super::current::current;
use super::home::{
    build_mako_bootstrap_response_from_dir, build_mako_channels_response_from_dir,
    build_mako_crew_response_from_dir_and_sessions, build_mako_home_response_from_dir,
    update_crew_document, update_home_document, DocumentWriteRequest,
};
use super::mako_home_dir_for_user;
use super::sessions::{
    dispatch, list_sessions, main_session, map_runtime_trace_event, recover_daemon,
    schedule_session, session_status, set_priority, DispatchRequest, PriorityRequest,
    ScheduleRequest,
};
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

    let mut credential_store = CredentialStore::default();
    credential_store.set(ProviderId::OpenAI, "test-openai-key".to_string());

    (
        AppState {
            server_port: 3000,
            db_path: Arc::new(db_path),
            working_dir: Arc::new(working_dir.clone()),
            ai_client: None,
            tool_registry: Arc::new(ToolRegistry::new()),
            process_registry: Arc::new(ProcessRegistry::new()),
            model_registry: create_model_registry(),
            credential_store: Arc::new(RwLock::new(credential_store)),
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
    Preferences::for_user(
        Database::new(&state.db_path).expect("database should open"),
        user_id,
    )
    .set_current_model("gpt-5.5")
    .expect("test model preference should persist");
}

fn current_user(user_id: &str, home_dir: &std::path::Path) -> CurrentUser {
    CurrentUser(AuthenticatedUser {
        user_id: Some(user_id.to_string()),
        home_dir: Some(home_dir.to_path_buf()),
    })
}

fn app_error_description(error: AppError) -> String {
    match error {
        AppError::NotFound(message) => format!("not found: {message}"),
        AppError::BadRequest(message) => format!("bad request: {message}"),
        AppError::Conflict(message) => format!("conflict: {message}"),
        AppError::ServiceUnavailable(message) => format!("service unavailable: {message}"),
        AppError::BadGateway(message) => format!("bad gateway: {message}"),
        AppError::Internal(message) => format!("internal: {message}"),
    }
}

async fn configure_test_model(state: &AppState) {
    let mut model = ModelMetadata::new("gpt-5.5", "GPT-5.5 Test", ProviderId::OpenAI)
        .with_transport(ApiFormat::OpenAIResponses);
    model.auth_scope = Some(ModelAuthScope::ApiKey);
    state.model_registry.upsert_model(model).await;
}

#[tokio::test]
async fn main_session_is_reused_and_isolated_by_owner() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    create_test_user(&state, "bob");

    let Json(first) = main_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .expect("alice companion should create");
    let Json(second) = main_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .expect("alice companion should reuse");
    let Json(other) = main_session(
        State(state.clone()),
        Some(current_user("bob", state.working_dir.as_ref())),
    )
    .await
    .expect("bob companion should create");

    assert!(first.created);
    assert!(!second.created);
    assert!(other.created);
    assert_eq!(first.session_id, second.session_id);
    assert_ne!(first.session_id, other.session_id);

    let manager = SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    assert!(manager
        .verify_session_ownership(&first.session_id, Some("alice"))
        .expect("ownership check should succeed"));
    assert!(!manager
        .verify_session_ownership(&first.session_id, Some("bob"))
        .expect("foreign ownership check should succeed"));
}

#[tokio::test]
async fn dispatch_normalizes_model_before_persisting_session() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    configure_test_model(&state).await;

    let (_, Json(response)) = dispatch(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "Investigate issue".to_string(),
            project_dir: None,
            model: Some("  gpt-5.5  ".to_string()),
            model_key: None,
            start_at: None,
            priority: None,
            crew_slug: None,
        }),
    )
    .await
    .unwrap_or_else(|error| panic!("dispatch should succeed: {}", app_error_description(error)));

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session = session_manager
        .get_session(&response.session_id)
        .expect("session lookup should succeed")
        .expect("session should exist");
    assert_eq!(session.model.as_deref(), Some("gpt-5.5"));
    let model_key = session
        .model_key
        .as_ref()
        .expect("dispatch should freeze an exact executable key");
    assert_eq!(model_key.provider, ProviderId::OpenAI);
    assert_eq!(model_key.model_id, "gpt-5.5");
    assert_eq!(model_key.auth_scope, Some(ModelAuthScope::ApiKey));
    assert_eq!(model_key.api_format, ApiFormat::OpenAIResponses);
}

#[test]
fn mako_home_response_surfaces_documents_and_sorted_crew() {
    let temp_dir =
        std::env::temp_dir().join(format!("krusty-mako-home-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let crew_builder = temp_dir.join("crew").join("builder");
    let crew_reviewer = temp_dir.join("crew").join("reviewer");
    std::fs::create_dir_all(&crew_builder).expect("builder dir should exist");
    std::fs::create_dir_all(&crew_reviewer).expect("reviewer dir should exist");

    std::fs::write(
        temp_dir.join(krusty_core::paths::MAKO_SOUL_FILE),
        "Always Swimming.",
    )
    .expect("soul should write");
    std::fs::write(temp_dir.join("CHANNELS.md"), "Signal line").expect("channels should write");
    std::fs::write(crew_reviewer.join("IDENTITY.md"), "Reviewer").expect("reviewer identity");
    std::fs::write(crew_builder.join("SOUL.md"), "Builder soul").expect("builder soul");

    let response = build_mako_home_response_from_dir(&temp_dir);

    assert_eq!(
        response
            .soul
            .as_ref()
            .map(|document| document.file_name.as_str()),
        Some("MAKO_SOUL.md")
    );
    assert_eq!(
        response
            .channels
            .as_ref()
            .map(|document| document.file_name.as_str()),
        Some("CHANNELS.md")
    );
    assert_eq!(response.crew_count, 2);
    assert_eq!(response.crew[0].slug, "builder");
    assert_eq!(response.crew[1].slug, "reviewer");
}

#[test]
fn mako_bootstrap_response_creates_default_home_and_crew() {
    let temp_dir =
        std::env::temp_dir().join(format!("krusty-mako-bootstrap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");

    let response = build_mako_bootstrap_response_from_dir(&temp_dir)
        .unwrap_or_else(|_| panic!("bootstrap should work"));

    assert!(response.ok);
    assert!(response
        .created_files
        .iter()
        .any(|path| path == paths::MAKO_SOUL_FILE));
    assert!(response
        .created_files
        .iter()
        .any(|path| path == "crew/reviewer/SOUL.md"));
    assert_eq!(response.home.crew_count, 3);
    assert_eq!(
        response
            .home
            .soul
            .as_ref()
            .map(|document| document.file_name.as_str()),
        Some(paths::MAKO_SOUL_FILE)
    );
}

#[tokio::test]
async fn mako_channels_response_surfaces_runtime_delivery_state() {
    let (state, temp_dir) = create_test_state();
    std::fs::write(
        temp_dir.join("CHANNELS.md"),
        "# Mako Channels\n- [x] iPhone push: urgent approvals only",
    )
    .expect("channels should write");

    let response = build_mako_channels_response_from_dir(&state, &temp_dir, 0);
    assert!(response.items.iter().any(|item| item.id == "main-thread"));

    let push = response
        .items
        .iter()
        .find(|item| item.id == "iphone-push")
        .expect("push channel should exist");
    assert_eq!(push.status, "attention");
    assert_eq!(push.kind, "mobile_push");
}

#[tokio::test]
async fn update_home_document_writes_to_user_scoped_database_profile() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_home = temp_dir.join("alice-home");
    std::fs::create_dir_all(&user_home).expect("user home should exist");

    let Json(response) = update_home_document(
        State(state.clone()),
        Some(current_user("alice", &user_home)),
        Path("soul".to_string()),
        Json(DocumentWriteRequest {
            content: "Stay watchful.".to_string(),
            expected_revision: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("document update should succeed"));

    let owner = MakoProfileOwner::user("alice").expect("owner should be valid");
    let stored =
        MakoProfileStore::new(Database::new(&state.db_path).expect("database should open"))
            .load(&owner)
            .expect("profile should load")
            .expect("profile should exist");
    assert_eq!(
        stored
            .document(MakoProfileDocumentKind::Soul)
            .map(|document| document.content.as_str()),
        Some("Stay watchful.")
    );
    assert_eq!(
        response
            .soul
            .as_ref()
            .map(|document| document.file_name.as_str()),
        Some(paths::MAKO_SOUL_FILE)
    );
}

#[tokio::test]
async fn update_crew_document_rejects_invalid_slug() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_home = temp_dir.join("alice-home");
    std::fs::create_dir_all(&user_home).expect("user home should exist");

    let result = update_crew_document(
        State(state),
        Some(current_user("alice", &user_home)),
        Path(("../oops".to_string(), "soul".to_string())),
        Json(DocumentWriteRequest {
            content: "Nope".to_string(),
            expected_revision: None,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::BadRequest(message)) if message == "invalid crew slug"));
}

#[test]
fn mako_crew_response_merges_home_profiles_with_runtime_state() {
    let temp_dir = std::env::temp_dir().join(format!("krusty-mako-crew-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    bootstrap_mako_home(&temp_dir).expect("bootstrap should work");
    let db_path = temp_dir.join("krusty.db");
    let db = Database::new(&db_path).expect("db should open");
    db.conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            ("alice", "alice@example.com", "free"),
        )
        .expect("user should insert");
    let session_manager = SessionManager::new(Database::new(&db_path).expect("db should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Crew runtime",
            None,
            Some(temp_dir.to_string_lossy().as_ref()),
            Some(temp_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("session should create");
    let sessions = session_manager
        .list_sessions_for_user_by_type(None, Some("alice"), SessionType::Mako)
        .expect("sessions should load");
    let task_store = AutonomousTaskStore::new(Database::new(&db_path).expect("db should open"));
    let delegated_store = krusty_core::storage::DelegatedRunStore::new(
        Database::new(&db_path).expect("db should open"),
    );
    let runtime_states =
        MakoRuntimeStateStore::new(Database::new(&db_path).expect("db should open"))
            .list_states_for_sessions(
                &sessions
                    .iter()
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>(),
            )
            .expect("runtime states should load");
    let task_id = task_store
        .create_task(&session_id, "Review", "Review current run", &[])
        .expect("task should create");
    task_store
        .claim_task(&task_id, "reviewer")
        .expect("task should claim");

    let response = build_mako_crew_response_from_dir_and_sessions(
        &temp_dir,
        &sessions,
        &runtime_states,
        &task_store,
        &delegated_store,
    )
    .unwrap_or_else(|_| panic!("crew response should build"));

    let reviewer = response
        .members
        .iter()
        .find(|member| member.slug == "reviewer")
        .expect("reviewer should exist");
    assert!(reviewer.known_to_home);
    assert_eq!(reviewer.status, "running");
    assert_eq!(reviewer.active_task_count, 1);
    assert!(reviewer.identity.is_some());
    assert!(reviewer.soul.is_some());
}

#[test]
fn user_scoped_mako_home_dir_prefers_current_user_home() {
    let temp_dir =
        std::env::temp_dir().join(format!("krusty-mako-user-home-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should exist");
    let user = current_user("alice", &temp_dir);

    assert_eq!(
        mako_home_dir_for_user(Some(&user)),
        paths::mako_dir_for_home(&temp_dir)
    );
}

#[tokio::test]
async fn dispatch_resolves_relative_project_dir_against_user_home() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    configure_test_model(&state).await;
    let user_root = temp_dir.join("alice-home");
    std::fs::create_dir_all(&user_root).expect("user root should exist");

    let (_, Json(response)) = dispatch(
        State(state.clone()),
        Some(current_user("alice", &user_root)),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "Investigate issue".to_string(),
            project_dir: Some("repo".to_string()),
            model: None,
            model_key: None,
            start_at: None,
            priority: None,
            crew_slug: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("dispatch should succeed"));

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session = session_manager
        .get_session(&response.session_id)
        .expect("session lookup should succeed")
        .expect("session should exist");
    let expected = user_root.join("repo").to_string_lossy().to_string();
    assert_eq!(session.project_dir.as_deref(), Some(expected.as_str()));
    assert_eq!(session.working_dir.as_deref(), Some(expected.as_str()));
}

#[tokio::test]
async fn dispatch_rejects_blank_task() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let result = dispatch(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "   ".to_string(),
            project_dir: None,
            model: None,
            model_key: None,
            start_at: None,
            priority: None,
            crew_slug: None,
        }),
    )
    .await;

    match result {
        Err(AppError::BadRequest(message)) => assert_eq!(message, "task must not be empty"),
        Ok(_) => panic!("blank dispatch should fail"),
        Err(_) => panic!("blank dispatch should fail with bad request"),
    }
}

#[tokio::test]
async fn dispatch_rejects_project_dir_outside_user_root() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let outside_root = temp_dir.join("outside");
    std::fs::create_dir_all(&user_root).expect("user root should exist");
    std::fs::create_dir_all(&outside_root).expect("outside root should exist");

    let result = dispatch(
        State(state),
        Some(current_user("alice", &user_root)),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "Investigate issue".to_string(),
            project_dir: Some(outside_root.to_string_lossy().to_string()),
            model: None,
            model_key: None,
            start_at: None,
            priority: None,
            crew_slug: None,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[tokio::test]
async fn dispatch_can_schedule_future_run() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    configure_test_model(&state).await;
    let wake_at = chrono::Utc::now() + chrono::Duration::minutes(30);

    let (_, Json(response)) = dispatch(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "Check CI later".to_string(),
            project_dir: None,
            model: None,
            model_key: None,
            start_at: Some(wake_at.to_rfc3339()),
            priority: None,
            crew_slug: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("scheduled dispatch should succeed"));

    assert_eq!(response.status, "scheduled");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    let runtime = runtime_store
        .get_state(&response.session_id)
        .expect("runtime lookup should succeed")
        .expect("runtime should exist");
    assert_eq!(runtime.status, MakoRuntimeStateStatus::Sleeping);
    assert_eq!(runtime.sleep_reason.as_deref(), Some("scheduled"));
    assert_eq!(
        runtime.last_wake_reason.as_deref(),
        Some("scheduled_dispatch")
    );
    assert!(runtime.next_wake_at.is_some());

    let Json(summary) = current(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(summary.status.scheduled_count, 1);
    assert_eq!(summary.status.sleeping_count, 0);
}

#[tokio::test]
async fn dispatch_persists_requested_priority() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    configure_test_model(&state).await;

    let (_, Json(response)) = dispatch(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "Escalate production fix".to_string(),
            project_dir: None,
            model: None,
            model_key: None,
            start_at: None,
            priority: Some(MakoRunPriority::High),
            crew_slug: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("dispatch should succeed"));

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    let runtime = runtime_store
        .get_state(&response.session_id)
        .expect("runtime lookup should succeed")
        .expect("runtime should exist");
    assert_eq!(runtime.priority, MakoRunPriority::High);
}

#[tokio::test]
async fn schedule_session_can_reschedule_existing_run() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    configure_test_model(&state).await;

    let (_, Json(response)) = dispatch(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        HeaderMap::new(),
        Json(DispatchRequest {
            task: "Investigate issue".to_string(),
            project_dir: None,
            model: None,
            model_key: None,
            start_at: None,
            priority: None,
            crew_slug: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("dispatch should succeed"));

    let wake_at = chrono::Utc::now() + chrono::Duration::hours(2);
    let Json(_) = schedule_session(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(response.session_id.clone()),
        HeaderMap::new(),
        Json(ScheduleRequest {
            start_at: wake_at.to_rfc3339(),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("schedule should succeed"));

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    let runtime = runtime_store
        .get_state(&response.session_id)
        .expect("runtime lookup should succeed")
        .expect("runtime should exist");
    let expected_wake_at = wake_at.to_rfc3339();
    assert_eq!(runtime.status, MakoRuntimeStateStatus::Sleeping);
    assert_eq!(runtime.sleep_reason.as_deref(), Some("scheduled"));
    assert_eq!(runtime.last_wake_reason.as_deref(), Some("manual_schedule"));
    assert_eq!(
        runtime.next_wake_at.as_deref(),
        Some(expected_wake_at.as_str())
    );
}

#[tokio::test]
async fn session_status_rejects_non_mako_session() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Code Session",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Code,
        )
        .expect("session should create");

    let result = session_status(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(session_id.clone()),
    )
    .await;

    match result {
        Err(AppError::BadRequest(message)) => {
            assert_eq!(
                message,
                format!("Session {} is not a Mako session", session_id)
            )
        }
        Ok(_) => panic!("code session should not load through mako status"),
        Err(_) => panic!("code session should fail with bad request"),
    }
}

#[tokio::test]
async fn list_sessions_only_returns_mako_sessions_with_runtime_state() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let mako_session_id = session_manager
        .create_session_for_user_with_config(
            "Mako Session",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("mako session should create");
    session_manager
        .create_session_for_user_with_config(
            "Code Session",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Code,
        )
        .expect("code session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &mako_session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some("2026-01-01T00:00:00Z"),
            Some("waiting"),
            None,
            None,
            Some("sleep"),
            MakoRunPriority::Normal,
        )
        .expect("runtime state should persist");

    let Json(sessions) = list_sessions(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("list sessions should succeed"));

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, mako_session_id);
    assert_eq!(
        sessions[0].runtime.as_ref().map(|runtime| runtime.status),
        Some(MakoRuntimeStateStatus::Sleeping)
    );
}

#[tokio::test]
async fn current_summarizes_waiting_and_sleeping_runs() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let waiting_session_id = session_manager
        .create_session_for_user_with_config(
            "Waiting Run",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("waiting session should create");
    let sleeping_session_id = session_manager
        .create_session_for_user_with_config(
            "Sleeping Run",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("sleeping session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &waiting_session_id,
            MakoRuntimeStateStatus::AwaitingInput,
            None,
            Some("approval"),
            None,
            None,
            Some("user"),
            MakoRunPriority::Normal,
        )
        .expect("waiting state should persist");
    runtime_store
        .set_state(
            &sleeping_session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some("2026-01-01T00:00:00Z"),
            Some("waiting"),
            None,
            None,
            Some("sleep"),
            MakoRunPriority::High,
        )
        .expect("sleeping state should persist");

    let Json(summary) = current(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(summary.status.home_status, "blocked");
    assert_eq!(summary.status.waiting_count, 1);
    assert_eq!(summary.status.sleeping_count, 1);
    assert_eq!(summary.status.high_priority_count, 1);
    assert_eq!(summary.status.pending_approvals_count, 0);
    assert_eq!(
        summary.status.next_wake_at.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(summary.diagnostics.daemon.active_runtime_count, 0);
    assert_eq!(summary.diagnostics.daemon.scheduled_wake_count, 0);
    assert_eq!(summary.diagnostics.daemon.recoverable_session_count, 1);
    assert_eq!(summary.runs.len(), 2);
}

#[tokio::test]
async fn current_surfaces_pending_tool_approvals() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Approval Run",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("approval session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &session_id,
            MakoRuntimeStateStatus::AwaitingInput,
            None,
            Some("approval"),
            None,
            Some("durable-run-1"),
            Some("user"),
            MakoRunPriority::High,
        )
        .expect("waiting state should persist");

    let now = chrono::Utc::now().to_rfc3339();
    let durable_db = Database::new(&state.db_path).expect("database should open");
    durable_db
        .conn()
        .execute_batch(&format!(
            "INSERT INTO mako_controllers (
                id, scope_key, user_id, session_id, status, timezone,
                max_concurrent_runs, created_at, updated_at
             ) VALUES (
                'controller-approval', 'session:{session_id}', 'alice', '{session_id}',
                'active', 'UTC', 1, '{now}', '{now}'
             );
             INSERT INTO mako_runs (
                id, controller_id, session_id, kind, objective, config_json, status,
                priority, available_at, attempt_count, max_attempts, created_at, updated_at
             ) VALUES (
                'durable-run-1', 'controller-approval', '{session_id}', 'dispatch', 'work',
                '{{}}', 'running',
                0, '{now}', 1, 3, '{now}', '{now}'
             );"
        ))
        .expect("durable approval run should insert");
    durable_db
        .conn()
        .execute(
            "INSERT INTO mako_controller_events (
                controller_id, sequence, event_type, run_id, payload_json, created_at
             ) VALUES ('controller-approval', 1, 'agentic_event', 'durable-run-1', ?1, ?2)",
            (
                serde_json::json!({
                    "type": "tool_approval_required",
                    "id": "tool-1",
                    "name": "bash",
                    "arguments": {
                        "command": "git push",
                        "cwd": "/workspace"
                    }
                })
                .to_string(),
                now.as_str(),
            ),
        )
        .expect("durable approval event should persist");

    let trace_db = Database::new(&state.db_path).expect("database should open");
    let trace_store = RuntimeTraceStore::new(&trace_db);
    let approval_event = RuntimeTraceEvent::from_loop_event(
        "trace-run-deliberately-different",
        1,
        0,
        &LoopEvent::ToolApprovalRequired {
            id: "tool-1".to_string(),
            name: "bash".to_string(),
            arguments: serde_json::json!({
                "command": "git push",
                "cwd": "/workspace"
            }),
        },
    );
    trace_store
        .append_event(&session_id, &approval_event)
        .expect("approval event should persist");

    let Json(summary) = current(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(summary.status.pending_approvals_count, 1);
    assert_eq!(summary.approvals.len(), 1);
    assert_eq!(summary.approvals[0].session_id, session_id);
    assert_eq!(summary.approvals[0].run_id, "durable-run-1");
    assert_eq!(summary.approvals[0].tool_call_id, "tool-1");
    assert_eq!(summary.approvals[0].tool_name, "bash");
    assert_eq!(summary.approvals[0].priority, MakoRunPriority::High);
    assert_eq!(summary.diagnostics.attention_run_count, 1);
    assert_eq!(summary.diagnostics.queue_pressure, "attention");
    assert_eq!(summary.diagnostics.health_state, "attention");
    assert_eq!(
        summary.runs[0]
            .diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.kind.as_str()),
        Some("awaiting_approval")
    );
}

#[tokio::test]
async fn current_surfaces_overdue_wake_diagnostics() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Overdue Wake",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("session should create");

    let overdue_wake_at = (chrono::Utc::now() - chrono::Duration::minutes(20)).to_rfc3339();
    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some(overdue_wake_at.as_str()),
            Some("scheduled"),
            None,
            None,
            Some("scheduled_dispatch"),
            MakoRunPriority::Normal,
        )
        .expect("runtime state should persist");

    let Json(summary) = current(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(summary.diagnostics.degraded_count, 1);
    assert_eq!(summary.diagnostics.stalled_count, 1);
    assert_eq!(summary.diagnostics.overdue_wake_count, 1);
    assert_eq!(summary.diagnostics.open_run_count, 1);
    assert_eq!(summary.diagnostics.health_state, "degraded");
    assert_eq!(summary.diagnostics.queue_pressure, "calm");
    assert_eq!(
        summary.runs[0]
            .diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.kind.as_str()),
        Some("overdue_wake")
    );
    assert_eq!(
        summary.runs[0]
            .diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.severity.as_str()),
        Some("critical")
    );
}

#[tokio::test]
async fn attention_surfaces_scheduled_start_and_delegated_completion_updates() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Scheduled Release Watch",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &session_id,
            MakoRuntimeStateStatus::Running,
            None,
            None,
            None,
            Some("run-1"),
            Some("scheduled_dispatch"),
            MakoRunPriority::Normal,
        )
        .expect("runtime state should persist");

    let trace_db = Database::new(&state.db_path).expect("database should open");
    let trace_store = RuntimeTraceStore::new(&trace_db);
    let started_event = RuntimeTraceEvent::from_loop_event(
        "run-1",
        1,
        0,
        &LoopEvent::TickInjected { tick_number: 1 },
    );
    trace_store
        .append_event(&session_id, &started_event)
        .expect("scheduled run should write trace");

    let delegated_store =
        DelegatedRunStore::new(Database::new(&state.db_path).expect("database should open"));
    delegated_store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "delegated-1".to_string(),
            parent_session_id: session_id.clone(),
            parent_tool_call_id: Some("tool-crew-1".to_string()),
            role: DelegatedRunRole::Build,
            stage: DelegatedRunStage::Created,
            provider: None,
            model: None,
            resumable: false,
            resumed_from_run_id: None,
            target_scope: vec![DelegatedRunScope {
                label: "release notes".to_string(),
                path: "docs/release.md".to_string(),
                kind: "file".to_string(),
            }],
        })
        .expect("delegated run should create");
    delegated_store
        .finalize_run(
            "delegated-1",
            DelegatedRunStage::Complete,
            &serde_json::json!({
                "human_review": "Release notes review completed."
            }),
            Some("Release notes review completed."),
            false,
        )
        .expect("delegated run should finalize");

    let Json(response) = attention(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Query(AttentionQuery {
            thread_session_id: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("attention should succeed"));

    assert!(response
        .items
        .iter()
        .any(|item| item.kind == "scheduled_run_started"));
    assert!(response
        .items
        .iter()
        .any(|item| item.kind == "delegated_task_completed"));
}

#[tokio::test]
async fn attention_uses_scheduled_completion_kind_for_scheduled_run_finishes() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Nightly Summary",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &session_id,
            MakoRuntimeStateStatus::Idle,
            None,
            None,
            None,
            None,
            Some("manual_schedule"),
            MakoRunPriority::Normal,
        )
        .expect("runtime state should persist");

    let trace_db = Database::new(&state.db_path).expect("database should open");
    let trace_store = RuntimeTraceStore::new(&trace_db);
    let started_event = RuntimeTraceEvent::from_loop_event(
        "run-2",
        1,
        0,
        &LoopEvent::TickInjected { tick_number: 1 },
    );
    trace_store
        .append_event(&session_id, &started_event)
        .expect("scheduled run should write start trace");
    let finished_event = RuntimeTraceEvent::from_loop_event(
        "run-2",
        2,
        1,
        &LoopEvent::Finished {
            session_id: session_id.clone(),
            stop_reason: LoopStopReason::Completed,
        },
    );
    trace_store
        .append_event(&session_id, &finished_event)
        .expect("scheduled run should write finish trace");

    let Json(response) = attention(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Query(AttentionQuery {
            thread_session_id: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("attention should succeed"));

    assert!(response
        .items
        .iter()
        .any(|item| item.kind == "scheduled_run_completed"));
    assert!(!response
        .items
        .iter()
        .any(|item| item.kind == "run_completed"));
}

#[tokio::test]
async fn attention_does_not_treat_future_scheduled_sleep_as_started() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Tomorrow Morning Check",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some(&(chrono::Utc::now() + chrono::Duration::minutes(20)).to_rfc3339()),
            Some("scheduled"),
            None,
            None,
            Some("scheduled_dispatch"),
            MakoRunPriority::Normal,
        )
        .expect("runtime state should persist");

    let Json(response) = attention(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Query(AttentionQuery {
            thread_session_id: None,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("attention should succeed"));

    assert!(!response
        .items
        .iter()
        .any(|item| item.kind == "scheduled_run_started"));
}

#[tokio::test]
async fn current_summarizes_knowledge_snapshot_health() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let project_dir = user_root.join("repo");
    std::fs::create_dir_all(&project_dir).expect("project dir should exist");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Knowledge Run",
            None,
            Some(project_dir.to_string_lossy().as_ref()),
            Some(project_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("session should create");

    let Json(initial_summary) = current(
        State(state.clone()),
        Some(current_user("alice", &user_root)),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(initial_summary.diagnostics.knowledge.scope_count, 1);
    assert_eq!(
        initial_summary.diagnostics.knowledge.missing_snapshot_count,
        1
    );
    assert_eq!(
        initial_summary.diagnostics.knowledge.stale_snapshot_count,
        0
    );

    let memory_store =
        MemoryStore::new(Database::new(&state.db_path).expect("database should open"));
    let snapshot = memory_store
        .save(
            MemoryType::Project,
            CURRENT_SNAPSHOT_TITLE,
            "Initial knowledge snapshot",
            Some(project_dir.to_string_lossy().as_ref()),
            Some("alice"),
        )
        .expect("snapshot should persist");

    Database::new(&state.db_path)
        .expect("database should open")
        .conn()
        .execute(
            "UPDATE agent_memories SET updated_at = ?1 WHERE id = ?2",
            ("2025-01-01T00:00:00Z", snapshot.id.as_str()),
        )
        .expect("snapshot should backdate");

    let report_store =
        ReportStore::new(Database::new(&state.db_path).expect("database should open"));
    report_store
        .create_report(CreateReportInput {
            title: "Knowledge refresh",
            session_id: session_id.as_str(),
            project_dir: Some(project_dir.to_string_lossy().as_ref()),
            report_root: None,
            content: "Updated project findings",
            summary: "Updated project findings",
            tags: &[],
            sources: &[],
        })
        .expect("report should persist");

    let Json(updated_summary) = current(
        State(state.clone()),
        Some(current_user("alice", &user_root)),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(updated_summary.diagnostics.knowledge.scope_count, 1);
    assert_eq!(
        updated_summary.diagnostics.knowledge.missing_snapshot_count,
        0
    );
    assert_eq!(
        updated_summary.diagnostics.knowledge.stale_snapshot_count,
        1
    );
}

#[tokio::test]
async fn recover_daemon_only_recovers_owned_sessions() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    create_test_user(&state, "bob");
    configure_test_model(&state).await;

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let alice_session_id = session_manager
        .create_session_for_user_with_config(
            "Alice Sleeping",
            Some("gpt-5.5"),
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("alice session should create");
    let bob_session_id = session_manager
        .create_session_for_user_with_config(
            "Bob Sleeping",
            Some("gpt-5.5"),
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("bob"),
            None,
            SessionType::Mako,
        )
        .expect("bob session should create");

    let wake_at = (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &alice_session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some(wake_at.as_str()),
            Some("scheduled"),
            None,
            None,
            Some("dispatch"),
            MakoRunPriority::Normal,
        )
        .expect("alice runtime state should persist");
    runtime_store
        .set_state(
            &bob_session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some(wake_at.as_str()),
            Some("scheduled"),
            None,
            None,
            Some("dispatch"),
            MakoRunPriority::Normal,
        )
        .expect("bob runtime state should persist");

    let Json(response) = recover_daemon(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        HeaderMap::new(),
    )
    .await
    .unwrap_or_else(|_| panic!("recover should succeed"));

    assert!(response.ok);
    assert_eq!(response.recovered_count, 1);

    let Json(alice_summary) = current(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("alice current should succeed"));
    assert_eq!(alice_summary.diagnostics.daemon.scheduled_wake_count, 1);
    assert_eq!(
        alice_summary.diagnostics.daemon.recoverable_session_count,
        1
    );

    let Json(bob_summary) = current(
        State(state.clone()),
        Some(current_user("bob", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("bob current should succeed"));
    assert_eq!(bob_summary.diagnostics.daemon.scheduled_wake_count, 0);
    assert_eq!(bob_summary.diagnostics.daemon.recoverable_session_count, 1);
}

#[tokio::test]
async fn set_priority_updates_runtime_state_and_current_ordering() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let first_session_id = session_manager
        .create_session_for_user_with_config(
            "First Run",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("first session should create");
    let second_session_id = session_manager
        .create_session_for_user_with_config(
            "Second Run",
            None,
            Some(state.working_dir.to_string_lossy().as_ref()),
            Some(state.working_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("second session should create");

    let runtime_store =
        MakoRuntimeStateStore::new(Database::new(&state.db_path).expect("database should open"));
    runtime_store
        .set_state(
            &first_session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some("2026-01-01T00:00:00Z"),
            Some("scheduled"),
            None,
            None,
            Some("dispatch"),
            MakoRunPriority::Normal,
        )
        .expect("first runtime state should persist");
    runtime_store
        .set_state(
            &second_session_id,
            MakoRuntimeStateStatus::Sleeping,
            Some("2026-01-01T01:00:00Z"),
            Some("scheduled"),
            None,
            None,
            Some("dispatch"),
            MakoRunPriority::Low,
        )
        .expect("second runtime state should persist");

    let Json(_) = set_priority(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Path(second_session_id.clone()),
        HeaderMap::new(),
        Json(PriorityRequest {
            priority: MakoRunPriority::High,
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("priority update should succeed"));

    let runtime = runtime_store
        .get_state(&second_session_id)
        .expect("runtime lookup should succeed")
        .expect("runtime should exist");
    assert_eq!(runtime.priority, MakoRunPriority::High);

    let Json(summary) = current(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
    )
    .await
    .unwrap_or_else(|_| panic!("current should succeed"));

    assert_eq!(summary.status.high_priority_count, 1);
    assert_eq!(
        summary.runs.first().map(|run| run.session_id.as_str()),
        Some(second_session_id.as_str())
    );
}

#[tokio::test]
async fn session_status_includes_resolved_cadence() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let project_dir = user_root.join("repo");
    std::fs::create_dir_all(project_dir.join(".krusty")).expect("project settings dir");
    std::fs::write(
        project_dir.join(".krusty").join("settings.json"),
        r#"{ "mako": { "tick_interval_secs": 15, "max_ticks": 50 } }"#,
    )
    .expect("project settings should write");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Configured Run",
            None,
            Some(project_dir.to_string_lossy().as_ref()),
            Some(project_dir.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .expect("configured session should create");

    let Json(status) = session_status(
        State(state.clone()),
        Some(current_user("alice", &user_root)),
        Path(session_id),
    )
    .await
    .unwrap_or_else(|_| panic!("session status should succeed"));

    assert_eq!(status.cadence.tick_interval_secs, 15);
    assert_eq!(status.cadence.max_ticks, 50);
}

#[test]
fn map_runtime_trace_event_skips_malformed_payload() {
    let event = RuntimeTraceEvent {
        run_id: "run-1".to_string(),
        sequence: 7,
        turn: 0,
        event_type: "user_message".to_string(),
        call_kind: None,
        operation: None,
        payload: serde_json::json!({ "level": "info" }),
        failure_category: None,
        stop_reason: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
    };

    assert!(map_runtime_trace_event(event).is_none());
}
