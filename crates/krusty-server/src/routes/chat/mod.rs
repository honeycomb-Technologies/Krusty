//! Chat endpoint with SSE streaming via core orchestrator.

mod content;
mod session;
mod stream;
mod stream_notify;
mod tools;

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::post,
    Json, Router,
};
use futures::stream::Stream;
use serde_json::json;

use krusty_core::agent::plan_handler::parse_plan_confirm_choice;
use krusty_core::agent::LoopInput;
use krusty_core::ai::types::{Content, ModelMessage, Role};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{Database, SessionType, WorkMode, WorkspaceMode};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;

use self::content::{build_user_content, content_blocks_include_images, validate_content_blocks};
use self::session::{
    select_model_for_chat_request, setup_chat_session, ChatSessionContext, RequestedModel,
};
use self::stream::start_orchestrator_sse;
#[cfg(test)]
use self::stream::{forward_loop_event, run_orchestrator_event_bridge};
use self::tools::apply_thinking_config;
use super::session_access::{current_user_id, ensure_owned_session, request_workspace_scope};
use crate::ai_bootstrap::{persist_current_model_selection, resolve_preferred_model};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{ChatRequest, ThinkingLevel, ToolApprovalRequest, ToolResultRequest};
use crate::utils::workspace::{
    normalize_resolved_requested_workspace, WorkspaceNormalizationPolicy,
};
use crate::AppState;

const SSE_CHANNEL_BUFFER: usize = 256;
const SESSION_LOCK_MAX_ENTRIES: usize = 1000;
const SESSION_LOCK_MAX_AGE: Duration = Duration::from_secs(3600);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(chat))
        .route("/tool-result", post(tool_result))
        .route("/tool-approval", post(tool_approval))
}
// ── Handlers ─────────────────────────────────────────────────────────

async fn chat(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    validate_content_blocks(&req.content)?;

    let user_id = current_user_id(user.as_ref()).map(ToOwned::to_owned);
    let workspace_scope = request_workspace_scope(&state, user.as_ref());
    let requested_model = RequestedModel::from_request(req.model.as_deref());
    let requested_session_type = req.session_type.unwrap_or(SessionType::Code);
    let requires_vision = content_blocks_include_images(&req.content);

    let (session_id, is_first_message, pending_model_update) = match req.session_id {
        Some(id) => {
            let db = Database::new(&state.db_path)?;
            let sm = SessionManager::new(db);
            ensure_owned_session(&sm, &id, user.as_ref())?;
            let messages = sm.load_session_messages(&id)?;
            (id, messages.is_empty(), requested_model.persisted())
        }
        None => {
            let db = Database::new(&state.db_path)?;
            let sm = SessionManager::new(db);
            let title = SessionManager::generate_title_from_content(&req.message);
            let default_mode_without_paths = if requested_session_type == SessionType::Chat {
                WorkspaceMode::Neutral
            } else {
                WorkspaceMode::Selected
            };
            let default_workspace = workspace_scope.base_dir.to_string_lossy().to_string();
            let workspace = normalize_resolved_requested_workspace(
                req.working_dir.as_deref(),
                req.project_dir.as_deref(),
                req.workspace_mode,
                WorkspaceNormalizationPolicy {
                    default_mode_without_paths,
                    selected_fallback_dir: Some(default_workspace.as_str()),
                },
                &workspace_scope.base_dir,
                &workspace_scope.allowed_root,
            )?;
            let preferred_model =
                resolve_preferred_model(state.db_path.as_ref().as_path(), user_id.as_deref());
            let initial_model = select_model_for_chat_request(
                &state,
                requested_model,
                preferred_model
                    .as_deref()
                    .filter(|_| matches!(requested_model, RequestedModel::Unspecified)),
                requires_vision,
            )
            .await?;
            if initial_model.is_none() && preferred_model.is_none() {
                return Err(AppError::BadRequest(
                    "No model selected. Choose a model and try again.".to_string(),
                ));
            }
            let id = sm.create_session_for_user_with_config(
                &title,
                initial_model.as_deref(),
                workspace.working_dir.as_deref(),
                workspace.project_dir.as_deref(),
                workspace.workspace_mode,
                user_id.as_deref(),
                None,
                requested_session_type,
            )?;
            let should_persist_current_model = match requested_model {
                RequestedModel::Set(_) => true,
                RequestedModel::Unspecified => {
                    preferred_model.as_deref() == initial_model.as_deref()
                }
                RequestedModel::Clear => false,
            };
            if should_persist_current_model {
                if let Some(model) = initial_model.as_deref() {
                    persist_current_model_selection(
                        &state.model_registry,
                        state.db_path.as_ref().as_path(),
                        user_id.as_deref(),
                        model,
                    )
                    .await?;
                }
            }
            (id, true, None)
        }
    };

    let mut ctx = setup_chat_session(
        &state,
        user.as_ref(),
        &session_id,
        requested_model,
        req.thinking_enabled,
        req.research_enabled.unwrap_or(false),
        requires_vision,
    )
    .await?;

    if let Some(model_override) = pending_model_update {
        ctx.session_manager
            .update_session_model(&session_id, model_override)?;
        if let Some(model) = model_override {
            persist_current_model_selection(
                &state.model_registry,
                state.db_path.as_ref().as_path(),
                user_id.as_deref(),
                model,
            )
            .await?;
        }
    }

    let mut work_mode = ctx.work_mode;
    if let Some(requested_mode) = req.mode {
        if requested_mode != work_mode {
            ctx.session_manager
                .update_session_work_mode(&session_id, requested_mode)?;
            work_mode = requested_mode;
        }
    }

    let user_content = build_user_content(&req.message, &req.content)?;
    let user_content_json = serde_json::to_string(&user_content)?;

    ctx.conversation.push(ModelMessage {
        role: Role::User,
        content: user_content,
    });
    ctx.session_manager
        .save_message(&session_id, "user", &user_content_json)?;

    // Mako sessions always run in autonomous mode (classifier provides safety gate)
    let permission_mode = if ctx.session_type == SessionType::Mako {
        PermissionMode::Autonomous
    } else {
        req.permission_mode
    };

    start_orchestrator_sse(&state, ctx, work_mode, permission_mode, is_first_message).await
}

async fn tool_result(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ToolResultRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let mut ctx = setup_chat_session(
        &state,
        user.as_ref(),
        &req.session_id,
        RequestedModel::Unspecified,
        ThinkingLevel::Off,
        false,
        false,
    )
    .await?;

    // Plan confirmation is an internal orchestrator event, not a real tool call.
    // Don't add a ToolResult — instead add a user message to resume the conversation.
    if req.tool_call_id.starts_with("plan-confirm-") {
        let choice = parse_plan_confirm_choice(&req.result);
        let work_mode = if choice.as_deref() == Some("execute") {
            ctx.session_manager
                .update_session_work_mode(&req.session_id, WorkMode::Build)?;
            // Add a user message instructing the AI to begin execution
            let user_content = vec![Content::Text {
                text:
                    "The plan has been approved. Begin executing the plan, starting with Task 1.1."
                        .to_string(),
            }];
            let user_json = serde_json::to_string(&user_content)?;
            ctx.conversation.push(ModelMessage {
                role: Role::User,
                content: user_content,
            });
            ctx.session_manager
                .save_message(&req.session_id, "user", &user_json)?;
            WorkMode::Build
        } else {
            // Abandon
            if let Ok(plan_manager) = PlanManager::new((*state.db_path).clone()) {
                let _ = plan_manager.abandon_plan(&req.session_id);
            }
            let user_content = vec![Content::Text {
                text: "The plan has been abandoned. What would you like to do instead?".to_string(),
            }];
            let user_json = serde_json::to_string(&user_content)?;
            ctx.conversation.push(ModelMessage {
                role: Role::User,
                content: user_content,
            });
            ctx.session_manager
                .save_message(&req.session_id, "user", &user_json)?;
            ctx.work_mode
        };
        return start_orchestrator_sse(&state, ctx, work_mode, PermissionMode::Autonomous, false)
            .await;
    }

    let has_thinking = ctx.conversation.iter().any(|msg| {
        msg.content
            .iter()
            .any(|c| matches!(c, Content::Thinking { .. }))
    });
    if has_thinking {
        apply_thinking_config(&ctx.ai_client, ThinkingLevel::High, &mut ctx.options);
    }

    // Merge or append tool result into conversation
    let merged = if let Some(last_msg) = ctx.conversation.last_mut() {
        if last_msg.role == Role::User
            && last_msg.content.iter().any(|c| {
                matches!(c, Content::ToolResult { tool_use_id, .. } if tool_use_id == req.tool_call_id.as_str())
            })
        {
            if let Some(output) = last_msg.content.iter_mut().find_map(|content| match content {
                Content::ToolResult {
                    tool_use_id, output, ..
                } if tool_use_id == req.tool_call_id.as_str() => Some(output),
                _ => None,
            }) {
                *output = serde_json::Value::String(req.result.clone());
            }
            let updated_json = serde_json::to_string(&last_msg.content)?;
            ctx.session_manager
                .update_last_message(&req.session_id, "user", &updated_json)?;
            true
        } else {
            false
        }
    } else {
        false
    };

    if !merged {
        let tool_result_content = vec![Content::ToolResult {
            tool_use_id: req.tool_call_id.clone(),
            output: serde_json::Value::String(req.result.clone()),
            is_error: None,
        }];
        let tool_result_json = serde_json::to_string(&tool_result_content)?;
        ctx.conversation.push(ModelMessage {
            role: Role::User,
            content: tool_result_content,
        });
        ctx.session_manager
            .save_message(&req.session_id, "user", &tool_result_json)?;
    }

    let work_mode = ctx.work_mode;
    start_orchestrator_sse(&state, ctx, work_mode, PermissionMode::Autonomous, false).await
}

async fn tool_approval(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ToolApprovalRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    submit_tool_approval(&state, user.as_ref(), req).await
}

pub(crate) async fn submit_tool_approval(
    state: &AppState,
    user: Option<&CurrentUser>,
    req: ToolApprovalRequest,
) -> Result<Json<serde_json::Value>, AppError> {
    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    ensure_owned_session(&session_manager, &req.session_id, user)?;

    let inputs = state.session_inputs.read().await;
    let sender = inputs
        .get(&req.session_id)
        .ok_or_else(|| AppError::NotFound("No active session".into()))?;
    sender
        .send(LoopInput::ToolApproval {
            tool_call_id: req.tool_call_id,
            approved: req.approved,
        })
        .map_err(|_| {
            AppError::Conflict(format!(
                "Session {} is no longer accepting tool approvals",
                req.session_id
            ))
        })?;
    Ok(Json(json!({"status": "ok"})))
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Tools allowed in Chat sessions — conversation only, no file/bash/code tools.
/// Web search/fetch are the only tools. Research mode adds agent + report tools.
#[cfg(test)]
mod tests {
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
    use krusty_core::storage::{Database, SessionType, WorkspaceMode};
    use krusty_core::tools::registry::ToolRegistry;
    use krusty_core::SessionManager;

    use super::{
        build_user_content, chat, forward_loop_event, run_orchestrator_event_bridge,
        select_model_for_chat_request, tool_approval, RequestedModel,
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
}
