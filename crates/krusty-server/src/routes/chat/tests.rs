use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{timeout, Duration};

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::agent::{AgentCancellation, LoopEvent, LoopInput, UserHookManager};
use krusty_core::ai::models::{create_model_registry, ApiFormat, ModelMetadata};
use krusty_core::ai::providers::ProviderId;
use krusty_core::ai::types::Content;
use krusty_core::mcp::McpManager;
use krusty_core::process::ProcessRegistry;
use krusty_core::skills::SkillsManager;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::storage::{
    Database, PartialAssistantState, PendingInteractionSnapshot, RecoveryDecision,
    RecoveryNonResumableReason, RecoveryStatus, SessionRecoveryState, SessionType, WorkspaceMode,
};
use krusty_core::tools::registry::ToolRegistry;
use krusty_core::SessionManager;

use super::{
    build_user_content, chat, forward_loop_event, prepare_chat_contract_for_test,
    run_orchestrator_event_bridge, select_model_for_chat_request, tool_approval, RequestedModel,
};
use crate::auth::{AuthenticatedUser, CurrentUser};
use crate::error::AppError;
use crate::types::{ChatRequest, ContentBlock, ToolApprovalRequest};
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
async fn chat_new_session_contract_includes_session_type_workspace_model_and_fast_mode_without_ai()
{
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let project_parent = user_root.join("projects");
    std::fs::create_dir_all(&project_parent).expect("project parent should exist");
    let project_dir = project_parent.join("fresh-contract-repo");

    let contract = prepare_chat_contract_for_test(
        &state,
        Some(current_user("alice", &user_root)),
        ChatRequest {
            session_id: None,
            message: "continue in this workspace".to_string(),
            content: Vec::new(),
            project_dir: Some(project_dir.to_string_lossy().to_string()),
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Created),
            session_type: Some(SessionType::Code),
            model: Some("openai/gpt-5.5".to_string()),
            thinking_enabled: crate::types::ThinkingLevel::Off,
            fast_mode: true,
            mode: None,
            permission_mode: krusty_core::tools::registry::PermissionMode::default(),
            research_enabled: None,
        },
    )
    .await
    .unwrap_or_else(|_| panic!("chat contract should prepare without a real AI client"));

    let expected_project = project_dir.to_string_lossy().to_string();
    assert!(contract.is_first_message);
    assert_eq!(contract.session_type, SessionType::Code);
    assert_eq!(contract.workspace_mode, WorkspaceMode::Created);
    assert_eq!(contract.working_dir.to_string_lossy(), expected_project);
    assert_eq!(
        contract
            .project_dir
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
        Some(expected_project.as_str())
    );
    assert_eq!(contract.model.as_deref(), Some("openai/gpt-5.5"));
    assert!(contract.fast_mode);
    assert!(
        !contract
            .model
            .as_deref()
            .expect("model should be present")
            .contains("mini"),
        "fast mode must stay independent from mini model selection"
    );
    assert_eq!(contract.target_branch, None);
}

#[tokio::test]
async fn chat_existing_session_contract_uses_persisted_workspace_surface_and_target() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let persisted_project = user_root.join("repo");
    let ignored_project = user_root.join("ignored-repo");
    std::fs::create_dir_all(&persisted_project).expect("persisted project should exist");
    std::fs::create_dir_all(&ignored_project).expect("ignored project should exist");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Persisted Contract",
            Some("openai/gpt-5.5-mini"),
            Some(persisted_project.to_string_lossy().as_ref()),
            Some(persisted_project.to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            Some("feature/contract"),
            SessionType::Code,
        )
        .expect("session should be created");

    let contract = prepare_chat_contract_for_test(
        &state,
        Some(current_user("alice", &user_root)),
        ChatRequest {
            session_id: Some(session_id.clone()),
            message: "continue the existing work".to_string(),
            content: Vec::new(),
            project_dir: Some(ignored_project.to_string_lossy().to_string()),
            working_dir: Some(ignored_project.to_string_lossy().to_string()),
            workspace_mode: Some(WorkspaceMode::Created),
            session_type: Some(SessionType::Chat),
            model: None,
            thinking_enabled: crate::types::ThinkingLevel::Off,
            fast_mode: true,
            mode: None,
            permission_mode: krusty_core::tools::registry::PermissionMode::default(),
            research_enabled: None,
        },
    )
    .await
    .unwrap_or_else(|_| {
        panic!("existing session contract should prepare without a real AI client")
    });

    let expected_project = persisted_project.to_string_lossy().to_string();
    assert!(contract.is_first_message);
    assert_eq!(contract.session_id, session_id);
    assert_eq!(contract.session_type, SessionType::Code);
    assert_eq!(contract.workspace_mode, WorkspaceMode::Selected);
    assert_eq!(contract.working_dir.to_string_lossy(), expected_project);
    assert_eq!(
        contract
            .project_dir
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
        Some(expected_project.as_str())
    );
    assert_eq!(contract.model.as_deref(), Some("openai/gpt-5.5-mini"));
    assert_eq!(contract.target_branch.as_deref(), Some("feature/contract"));
    assert!(contract.fast_mode);
}

fn model(
    id: &str,
    provider: ProviderId,
    api_format: ApiFormat,
    supports_vision: bool,
) -> ModelMetadata {
    let mut model = ModelMetadata::new(id, id, provider).with_context(200_000, 32_768);
    model.supports_tools = true;
    model.supports_vision = supports_vision;
    model.api_format = api_format;
    model
}

#[test]
fn requested_model_prefers_request_and_trims_input() {
    let requested_model = RequestedModel::from_request(Some("  openai/gpt-5  "));

    assert_eq!(
        requested_model.effective(Some("minimax/m2")),
        Some("openai/gpt-5")
    );
    assert_eq!(requested_model.persisted(), Some(Some("openai/gpt-5")));
}

#[test]
fn requested_model_falls_back_to_session_model_when_unspecified() {
    let requested_model = RequestedModel::from_request(None);

    assert_eq!(
        requested_model.effective(Some("  anthropic/claude-opus-4.6  ")),
        Some("anthropic/claude-opus-4.6")
    );
    assert_eq!(requested_model.persisted(), None);
}

#[test]
fn requested_model_unspecified_ignores_empty_session_values() {
    let requested_model = RequestedModel::from_request(None);

    assert_eq!(requested_model.effective(Some("   ")), None);
    assert_eq!(requested_model.persisted(), None);
}

#[test]
fn requested_model_clear_does_not_fall_back_to_session_model() {
    let requested_model = RequestedModel::from_request(Some("   "));

    assert_eq!(
        requested_model.effective(Some("anthropic/claude-opus-4.6")),
        None
    );
    assert_eq!(requested_model.persisted(), Some(None));
}

#[test]
fn build_user_content_appends_message_when_only_images_are_provided() {
    let content = match build_user_content(
        "describe this",
        &[ContentBlock::Image {
            source: crate::types::ImageSource::Url {
                url: "https://example.com/image.png".to_string(),
            },
        }],
    ) {
        Ok(content) => content,
        Err(_) => panic!("image url content should build"),
    };

    assert!(matches!(content.first(), Some(Content::Image { .. })));
    assert!(matches!(
        content.last(),
        Some(Content::Text { text }) if text == "describe this"
    ));
}

#[test]
fn build_user_content_does_not_duplicate_message_when_text_block_exists() {
    let content = match build_user_content(
        "fallback message",
        &[ContentBlock::Text {
            text: "block text".to_string(),
        }],
    ) {
        Ok(content) => content,
        Err(_) => panic!("text content should build"),
    };

    assert_eq!(content.len(), 1);
    assert!(matches!(
        content.first(),
        Some(Content::Text { text }) if text == "block text"
    ));
}

#[test]
fn build_user_content_preserves_file_attachment_text_block() {
    let file_text = "Please review the attached file.\n\n--- notes.txt ---\nhello from file";
    let content = match build_user_content(
        "fallback message",
        &[ContentBlock::Text {
            text: file_text.to_string(),
        }],
    ) {
        Ok(content) => content,
        Err(_) => panic!("file attachment text block should build"),
    };

    assert_eq!(content.len(), 1);
    assert!(matches!(
        content.first(),
        Some(Content::Text { text }) if text == file_text
    ));
}

#[test]
fn build_user_content_rejects_unsupported_image_media_type() {
    let result = build_user_content(
        "describe this",
        &[ContentBlock::Image {
            source: crate::types::ImageSource::Base64 {
                media_type: "image/heic".to_string(),
                data: "ZmFrZQ==".to_string(),
            },
        }],
    );

    assert!(matches!(
        result,
        Err(AppError::BadRequest(message))
            if message.contains("image/heic") && message.contains("Convert HEIC/HEIF")
    ));
}

#[tokio::test]
async fn select_model_for_chat_request_falls_back_to_configured_vision_model() {
    let (state, _temp_dir) = create_test_state();
    {
        let mut credentials = state.credential_store.write().await;
        credentials.set(ProviderId::MiniMax, "minimax-test-key".to_string());
        credentials.set(ProviderId::OpenAI, "openai-test-key".to_string());
    }

    state
        .model_registry
        .set_models(
            ProviderId::MiniMax,
            vec![model(
                "MiniMax-M2.5",
                ProviderId::MiniMax,
                ApiFormat::Anthropic,
                false,
            )],
        )
        .await;
    state
        .model_registry
        .set_models(
            ProviderId::OpenAI,
            vec![model(
                "gpt-4.1",
                ProviderId::OpenAI,
                ApiFormat::OpenAI,
                true,
            )],
        )
        .await;

    let selected = select_model_for_chat_request(
        &state,
        RequestedModel::Unspecified,
        Some("MiniMax-M2.5"),
        true,
    )
    .await;

    match selected {
        Ok(selected) => assert_eq!(selected.as_deref(), Some("gpt-4.1")),
        Err(_) => panic!("vision fallback should resolve"),
    }
}

#[tokio::test]
async fn select_model_for_chat_request_rejects_explicit_non_vision_model() {
    let (state, _temp_dir) = create_test_state();
    {
        let mut credentials = state.credential_store.write().await;
        credentials.set(ProviderId::MiniMax, "minimax-test-key".to_string());
    }

    state
        .model_registry
        .set_models(
            ProviderId::MiniMax,
            vec![model(
                "MiniMax-M2.5",
                ProviderId::MiniMax,
                ApiFormat::Anthropic,
                false,
            )],
        )
        .await;

    let result =
        select_model_for_chat_request(&state, RequestedModel::Set("MiniMax-M2.5"), None, true)
            .await;

    assert!(matches!(
        result,
        Err(AppError::BadRequest(message))
            if message.contains("does not support image input")
    ));
}

#[tokio::test]
async fn tool_approval_rejects_foreign_owner() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    create_test_user(&state, "bob");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Owned Session", None, None, Some("alice"))
        .expect("session creation should succeed");

    let (tx, mut rx) = mpsc::unbounded_channel::<LoopInput>();
    state
        .session_inputs
        .write()
        .await
        .insert(session_id.clone(), tx);

    let result = tool_approval(
        State(state),
        Some(current_user("bob", std::path::Path::new("/tmp"))),
        Json(ToolApprovalRequest {
            session_id,
            tool_call_id: "tool-1".to_string(),
            approved: true,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::NotFound(_))));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn tool_approval_rejects_closed_session_channel() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Owned Session", None, None, Some("alice"))
        .expect("session creation should succeed");

    let (tx, rx) = mpsc::unbounded_channel::<LoopInput>();
    drop(rx);
    state
        .session_inputs
        .write()
        .await
        .insert(session_id.clone(), tx);

    let result = tool_approval(
        State(state),
        Some(current_user("alice", std::path::Path::new("/tmp"))),
        Json(ToolApprovalRequest {
            session_id,
            tool_call_id: "tool-1".to_string(),
            approved: true,
        }),
    )
    .await;

    assert!(matches!(result, Err(AppError::Conflict(_))));
}

#[tokio::test]
async fn submit_tool_approval_returns_recoverable_pending_approval_error_when_channel_missing() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Owned Session", None, None, Some("alice"))
        .expect("session creation should succeed");
    let pending_interaction = PendingInteractionSnapshot::tool_approval_from_call(
        "tool-1",
        "edit",
        &serde_json::json!({ "file_path": "src/lib.rs", "api_token": "hidden" }),
    );
    let recovery = SessionRecoveryState::new_with_pending_interactions(
        RecoveryStatus::AwaitingInput,
        Some(LoopStopReason::AwaitingInput),
        None,
        PartialAssistantState::default(),
        vec![pending_interaction],
        RecoveryDecision::NonResumable {
            reason: RecoveryNonResumableReason::AwaitingHumanInput,
        },
    );
    session_manager
        .update_recovery_state(&session_id, &recovery)
        .expect("pending approval recovery should persist");

    let result = tool_approval(
        State(state),
        Some(current_user("alice", std::path::Path::new("/tmp"))),
        Json(ToolApprovalRequest {
            session_id: session_id.clone(),
            tool_call_id: "tool-1".to_string(),
            approved: true,
        }),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::Conflict(message))
            if message.contains(&session_id)
                && message.contains("tool-1")
                && message.contains("approval channel unavailable")
                && message.contains("recoverable")
    ));
}

#[tokio::test]
async fn tool_approval_survives_sse_disconnect_until_run_finishes() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user("Owned Session", None, None, Some("alice"))
        .expect("session creation should succeed");

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<LoopInput>();
    state
        .session_inputs
        .write()
        .await
        .insert(session_id.clone(), input_tx);

    let (event_tx, event_rx) = mpsc::unbounded_channel::<LoopEvent>();
    let (sse_tx, sse_rx) = mpsc::channel(1);
    drop(sse_rx);

    let bridge = tokio::spawn(run_orchestrator_event_bridge(
        event_rx,
        sse_tx,
        session_id.clone(),
        Arc::clone(&state.session_inputs),
        None,
        None,
        Some("alice".to_string()),
        Arc::clone(&state.db_path),
    ));

    event_tx
        .send(LoopEvent::ToolApprovalRequired {
            id: "tool-1".to_string(),
            name: "edit".to_string(),
            arguments: serde_json::json!({}),
        })
        .expect("tool approval event should send");

    timeout(Duration::from_secs(1), async {
        loop {
            if state.session_inputs.read().await.contains_key(&session_id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("session input should stay registered after stream loss");

    let approval = tool_approval(
        State(state.clone()),
        Some(current_user("alice", std::path::Path::new("/tmp"))),
        Json(ToolApprovalRequest {
            session_id: session_id.clone(),
            tool_call_id: "tool-1".to_string(),
            approved: true,
        }),
    )
    .await;

    assert!(approval.is_ok());
    assert!(matches!(
        timeout(Duration::from_secs(1), input_rx.recv()).await,
        Ok(Some(LoopInput::ToolApproval {
            tool_call_id,
            approved: true,
        })) if tool_call_id == "tool-1"
    ));

    event_tx
        .send(LoopEvent::Finished {
            session_id: session_id.clone(),
            stop_reason: LoopStopReason::Completed,
        })
        .expect("finish event should send");

    bridge.await.expect("bridge should finish");
    assert!(state.session_inputs.read().await.get(&session_id).is_none());
}

#[tokio::test]
async fn chat_does_not_persist_model_override_when_setup_fails() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Existing Session",
            Some("minimax/m2"),
            None,
            None,
            krusty_core::storage::WorkspaceMode::Neutral,
            Some("alice"),
            None,
            krusty_core::storage::SessionType::Code,
        )
        .expect("session should be created");

    let result = chat(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Json(ChatRequest {
            session_id: Some(session_id.clone()),
            message: "hello".to_string(),
            content: Vec::new(),
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            session_type: None,
            model: Some("openai/gpt-5".to_string()),
            thinking_enabled: crate::types::ThinkingLevel::Off,
            fast_mode: false,
            mode: None,
            permission_mode: krusty_core::tools::registry::PermissionMode::default(),
            research_enabled: None,
        }),
    )
    .await;

    match result {
        Err(AppError::BadRequest(message)) => {
            assert_eq!(message, "No AI credentials configured");
        }
        Ok(_) => panic!("chat request should fail without configured AI credentials"),
        Err(_) => panic!("chat request should fail with bad request"),
    }

    let reloaded = session_manager
        .get_session(&session_id)
        .expect("session lookup should succeed")
        .expect("session should exist");
    assert_eq!(reloaded.model.as_deref(), Some("minimax/m2"));
}

#[tokio::test]
async fn chat_rejects_missing_model_before_creating_session() {
    let (state, temp_dir) = create_test_state();
    create_test_user(&state, "alice");
    let user_root = temp_dir.join("alice-home");
    let parent_dir = user_root.join("projects");
    std::fs::create_dir_all(&parent_dir).expect("parent dir should exist");
    let fresh_project_dir = parent_dir.join("fresh-chat-repo");

    let result = chat(
        State(state.clone()),
        Some(current_user("alice", &user_root)),
        Json(ChatRequest {
            session_id: None,
            message: "scan this workspace".to_string(),
            content: Vec::new(),
            project_dir: Some(fresh_project_dir.to_string_lossy().to_string()),
            working_dir: None,
            workspace_mode: Some(WorkspaceMode::Selected),
            session_type: Some(SessionType::Code),
            model: None,
            thinking_enabled: crate::types::ThinkingLevel::Off,
            fast_mode: false,
            mode: None,
            permission_mode: krusty_core::tools::registry::PermissionMode::default(),
            research_enabled: None,
        }),
    )
    .await;

    match result {
        Err(AppError::BadRequest(message)) => {
            assert_eq!(message, "No model selected. Choose a model and try again.");
        }
        Ok(_) => panic!("chat request should fail without a selected model"),
        Err(_) => panic!("chat request should fail with bad request"),
    }

    let expected = fresh_project_dir.to_string_lossy().to_string();
    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let sessions = session_manager
        .list_sessions_for_user(Some(expected.as_str()), Some("alice"))
        .expect("session listing should succeed");

    assert!(sessions.is_empty());
}

#[tokio::test]
async fn chat_rejects_unsupported_image_before_creating_session() {
    let (state, _temp_dir) = create_test_state();
    create_test_user(&state, "alice");

    let result = chat(
        State(state.clone()),
        Some(current_user("alice", state.working_dir.as_ref())),
        Json(ChatRequest {
            session_id: None,
            message: "what is this?".to_string(),
            content: vec![ContentBlock::Image {
                source: crate::types::ImageSource::Base64 {
                    media_type: "image/heic".to_string(),
                    data: "ZmFrZQ==".to_string(),
                },
            }],
            project_dir: None,
            working_dir: None,
            workspace_mode: None,
            session_type: Some(SessionType::Chat),
            model: None,
            thinking_enabled: crate::types::ThinkingLevel::Off,
            fast_mode: false,
            mode: None,
            permission_mode: krusty_core::tools::registry::PermissionMode::default(),
            research_enabled: None,
        }),
    )
    .await;

    assert!(matches!(
        result,
        Err(AppError::BadRequest(message))
            if message.contains("image/heic") && message.contains("Supported formats")
    ));

    let session_manager =
        SessionManager::new(Database::new(&state.db_path).expect("database should open"));
    let sessions = session_manager
        .list_sessions_for_user(None, Some("alice"))
        .expect("session listing should succeed");

    assert!(sessions.is_empty());
}

#[tokio::test]
async fn sse_critical_tool_approval_survives_full_buffer_with_lag_signal() {
    let (tx, mut rx) = mpsc::channel(1);
    let mut skipped_events = 0usize;

    assert!(
        forward_loop_event(
            &tx,
            "session-1",
            LoopEvent::TextDelta {
                delta: "first".to_string(),
            },
            &mut skipped_events,
        )
        .await
    );
    assert!(
        forward_loop_event(
            &tx,
            "session-1",
            LoopEvent::TextDelta {
                delta: "second".to_string(),
            },
            &mut skipped_events,
        )
        .await
    );
    assert_eq!(skipped_events, 1);

    let approval_tx = tx.clone();
    let approval_handle = tokio::spawn(async move {
        forward_loop_event(
            &approval_tx,
            "session-1",
            LoopEvent::ToolApprovalRequired {
                id: "tool-1".to_string(),
                name: "edit".to_string(),
                arguments: serde_json::json!({"path": "src/lib.rs"}),
            },
            &mut skipped_events,
        )
        .await
    });

    let first = rx.recv().await.expect("first event should arrive");
    let first = first.expect("sse event should be ok");
    assert!(format!("{first:?}").contains("text_delta"));

    let lagged = rx.recv().await.expect("lag signal should arrive");
    let lagged = lagged.expect("lag signal should be ok");
    let lagged = format!("{lagged:?}");
    assert!(lagged.contains("lagged"));
    assert!(lagged.contains("skipped"));

    let approval = rx.recv().await.expect("critical approval should arrive");
    let approval = approval.expect("approval event should be ok");
    let approval = format!("{approval:?}");
    assert!(approval.contains("tool_approval_required"));
    assert!(approval.contains("tool-1"));

    assert!(approval_handle.await.expect("approval task should join"));
}

#[tokio::test]
async fn forward_loop_event_surfaces_lag_before_terminal_event() {
    let (tx, mut rx) = mpsc::channel(1);
    let mut skipped_events = 0usize;

    assert!(
        forward_loop_event(
            &tx,
            "session-1",
            LoopEvent::TextDelta {
                delta: "first".to_string(),
            },
            &mut skipped_events,
        )
        .await
    );
    assert_eq!(skipped_events, 0);

    assert!(
        forward_loop_event(
            &tx,
            "session-1",
            LoopEvent::TextDelta {
                delta: "second".to_string(),
            },
            &mut skipped_events,
        )
        .await
    );
    assert_eq!(skipped_events, 1);

    let first = rx.recv().await.expect("first event should arrive");
    let first = first.expect("sse event should be ok");
    let first = format!("{first:?}");
    assert!(first.contains("text_delta"));

    let mut finish_skipped_events = skipped_events;
    let tx_clone = tx.clone();
    let finish_handle = tokio::spawn(async move {
        let delivered = forward_loop_event(
            &tx_clone,
            "session-1",
            LoopEvent::Finished {
                session_id: "session-1".to_string(),
                stop_reason: LoopStopReason::Completed,
            },
            &mut finish_skipped_events,
        )
        .await;
        (delivered, finish_skipped_events)
    });

    let lagged = rx.recv().await.expect("lagged event should arrive");
    let lagged = lagged.expect("lagged event should be ok");
    let lagged = format!("{lagged:?}");
    assert!(lagged.contains("lagged"));
    assert!(lagged.contains("skipped"));

    let finish = rx.recv().await.expect("finish event should arrive");
    let finish = finish.expect("finish event should be ok");
    let finish = format!("{finish:?}");
    assert!(finish.contains("finish"));

    let (delivered, skipped_events) = finish_handle.await.expect("finish task should join");
    assert!(delivered);
    assert_eq!(skipped_events, 0);

    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
}
