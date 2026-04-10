//! Chat endpoint with SSE streaming via core orchestrator.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::post,
    Json, Router,
};
use futures::stream::Stream;
use serde_json::json;
use tokio::sync::{mpsc, Mutex, OwnedMutexGuard};
use tokio_stream::wrappers::ReceiverStream;

use krusty_core::agent::coordinator_prompt::system_prompt_for_session;
use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::agent::plan_handler::parse_plan_confirm_choice;
use krusty_core::agent::{
    AgenticOrchestrator, LoopEvent, LoopInput, OrchestratorConfig, OrchestratorServices,
};
use krusty_core::ai::client::{
    AiClient, AnthropicAdaptiveEffort, CallOptions, CodexReasoningEffort,
};
use krusty_core::ai::providers::ProviderId;
use krusty_core::ai::types::{AiTool, Content, ImageContent, ModelMessage, Role, ThinkingConfig};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{
    Database, MakoRuntimeStateStore, ProjectSettings, SessionType, WorkMode, WorkspaceMode,
};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;

use super::session_access::{
    current_user_id, ensure_owned_session, load_owned_session, request_workspace_scope,
};
use crate::apns::{ApnsEventType, ApnsPayload};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::notifications::{fire_apns, fire_push, session_title};
use crate::push::{PushEventType, PushPayload};
use crate::types::{
    AgenticEvent, ChatRequest, ContentBlock, ThinkingLevel, ToolApprovalRequest, ToolResultRequest,
};
use crate::utils::messages::parse_stored_model_messages;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::{
    normalize_resolved_requested_workspace, resolve_optional_workspace_path,
    resolve_session_working_dir, WorkspaceNormalizationPolicy,
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

struct ChatSessionContext {
    ai_client: Arc<AiClient>,
    options: CallOptions,
    conversation: Vec<ModelMessage>,
    session_id: String,
    session_manager: SessionManager,
    working_dir: PathBuf,
    project_dir: Option<PathBuf>,
    work_mode: WorkMode,
    session_type: SessionType,
    mako_crew_slug: Option<String>,
    user_id: Option<String>,
    guard: OwnedMutexGuard<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestedModel<'a> {
    Unspecified,
    Clear,
    Set(&'a str),
}

impl<'a> RequestedModel<'a> {
    fn from_request(value: Option<&'a str>) -> Self {
        match value {
            Some(raw) => trimmed_nonempty(Some(raw))
                .map(Self::Set)
                .unwrap_or(Self::Clear),
            None => Self::Unspecified,
        }
    }

    fn effective(self, session_model: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::Unspecified => trimmed_nonempty(session_model),
            Self::Clear => None,
            Self::Set(model) => Some(model),
        }
    }

    fn persisted(self) -> Option<Option<&'a str>> {
        match self {
            Self::Unspecified => None,
            Self::Clear => Some(None),
            Self::Set(model) => Some(Some(model)),
        }
    }
}

/// Build user message content from content blocks (images) and text message.
fn build_user_content(message: &str, content_blocks: &[ContentBlock]) -> Vec<Content> {
    let mut contents = Vec::with_capacity(content_blocks.len() + usize::from(!message.is_empty()));
    let mut has_text_block = false;

    for block in content_blocks {
        match block {
            ContentBlock::Text { text } => {
                tracing::debug!("Content block: Text ({} chars)", text.len());
                has_text_block = true;
                contents.push(Content::Text { text: text.clone() });
            }
            ContentBlock::Image { source } => {
                let image = match source {
                    crate::types::ImageSource::Base64 { media_type, data } => {
                        tracing::debug!(
                            "Content block: Image (base64, media_type={}, data_len={})",
                            media_type,
                            data.len()
                        );
                        ImageContent {
                            base64: Some(data.clone()),
                            url: None,
                            media_type: Some(media_type.clone()),
                        }
                    }
                    crate::types::ImageSource::Url { url } => {
                        tracing::debug!("Content block: Image (url={})", url);
                        ImageContent {
                            base64: None,
                            url: Some(url.clone()),
                            media_type: None,
                        }
                    }
                };
                contents.push(Content::Image {
                    image,
                    detail: None,
                });
            }
        }
    }

    if !message.is_empty() && !has_text_block {
        contents.push(Content::Text {
            text: message.to_string(),
        });
    }

    if contents.is_empty() {
        contents.push(Content::Text {
            text: message.to_string(),
        });
    }

    contents
}

async fn setup_chat_session(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
    requested_model: RequestedModel<'_>,
    thinking_level: ThinkingLevel,
    research_enabled: bool,
) -> Result<ChatSessionContext, AppError> {
    let user_id = current_user_id(user).map(ToOwned::to_owned);
    let workspace_scope = request_workspace_scope(state, user);

    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);
    let session = load_owned_session(&session_manager, session_id, user)?;

    let effective_model = requested_model.effective(session.model.as_deref());
    let ai_client = state
        .resolve_ai_client(effective_model)
        .await
        .ok_or_else(|| AppError::BadRequest("No AI credentials configured".to_string()))?;

    let working_dir = resolve_session_working_dir(
        session.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let project_dir = resolve_optional_workspace_path(
        session.project_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?
    .map(PathBuf::from);

    let session_lock = {
        let mut locks = state.session_locks.write().await;
        if locks.len() > SESSION_LOCK_MAX_ENTRIES {
            locks.retain(|_, (lock, created_at)| {
                created_at.elapsed() < SESSION_LOCK_MAX_AGE || Arc::strong_count(lock) > 1
            });
        }
        let (lock, _) = locks
            .entry(session_id.to_string())
            .or_insert_with(|| (Arc::new(Mutex::new(())), Instant::now()));
        lock.clone()
    };
    let guard = Arc::clone(&session_lock)
        .try_lock_owned()
        .map_err(|_| AppError::Conflict(format!("Session {} is busy", session_id)))?;

    let raw_messages = session_manager.load_session_messages(session_id)?;
    let conversation = parse_stored_model_messages(session_id, raw_messages, "chat conversation");

    tracing::info!(
        session_type = ?session.session_type,
        session_id = %session_id,
        "Filtering tools for session type"
    );
    let ai_tools = filter_tools_for_session_type(
        state.tool_registry.get_ai_tools().await,
        session.session_type,
        research_enabled,
    );
    let mut options = CallOptions {
        tools: if ai_tools.is_empty() { None } else { Some(ai_tools) },
        session_id: Some(session_id.to_string()),
        codex_parallel_tool_calls: true,
        system_prompt: match session.session_type {
            SessionType::Chat => Some(
                "You are Krusty, a friendly conversational assistant. This is a chat session. \
                 You do NOT have access to any tools, files, or code. Do not mention or list tools. \
                 If the user needs coding help, suggest they switch to Code mode. \
                 Be helpful, natural, and conversational.".to_string()
            ),
            SessionType::Mako => system_prompt_for_session(SessionType::Mako),
            SessionType::Code => None, // uses default Krusty coding assistant prompt
        },
        ..Default::default()
    };
    if thinking_level.is_enabled() {
        apply_thinking_config(&ai_client, thinking_level, &mut options);
    }

    let effective_work_mode = PlanManager::new((*state.db_path).clone())
        .ok()
        .and_then(|pm| pm.get_lifecycle_state(session_id, session.work_mode).ok())
        .map(|state| state.effective_work_mode)
        .unwrap_or(session.work_mode);
    let mako_runtime = if session.session_type == SessionType::Mako {
        MakoRuntimeStateStore::new(Database::new(&state.db_path)?).get_state(session_id)?
    } else {
        None
    };

    Ok(ChatSessionContext {
        ai_client,
        options,
        conversation,
        session_id: session_id.to_string(),
        session_manager,
        working_dir,
        project_dir,
        work_mode: effective_work_mode,
        session_type: session.session_type,
        mako_crew_slug: mako_runtime.and_then(|runtime| runtime.crew_slug),
        user_id,
        guard,
    })
}

// ── Handlers ─────────────────────────────────────────────────────────

async fn chat(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let user_id = current_user_id(user.as_ref()).map(ToOwned::to_owned);
    let workspace_scope = request_workspace_scope(&state, user.as_ref());
    let requested_model = RequestedModel::from_request(req.model.as_deref());
    let requested_session_type = req.session_type.unwrap_or(SessionType::Code);

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
            let id = sm.create_session_for_user_with_config(
                &title,
                requested_model.effective(None),
                workspace.working_dir.as_deref(),
                workspace.project_dir.as_deref(),
                workspace.workspace_mode,
                user_id.as_deref(),
                None,
                requested_session_type,
            )?;
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
    )
    .await?;

    if let Some(model_override) = pending_model_update {
        ctx.session_manager
            .update_session_model(&session_id, model_override)?;
    }

    let mut work_mode = ctx.work_mode;
    if let Some(requested_mode) = req.mode {
        if requested_mode != work_mode {
            ctx.session_manager
                .update_session_work_mode(&session_id, requested_mode)?;
            work_mode = requested_mode;
        }
    }

    let user_content = build_user_content(&req.message, &req.content);
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
                matches!(c, Content::ToolResult { tool_use_id, .. } if tool_use_id == &req.tool_call_id)
            })
        {
            for c in &mut last_msg.content {
                if let Content::ToolResult {
                    tool_use_id,
                    output,
                    ..
                } = c
                {
                    if tool_use_id == &req.tool_call_id {
                        *output = serde_json::Value::String(req.result.clone());
                        break;
                    }
                }
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

// ── Orchestrator → SSE bridge ────────────────────────────────────────

fn loop_event_requires_delivery(event: &LoopEvent) -> bool {
    matches!(
        event,
        LoopEvent::AwaitingInput { .. }
            | LoopEvent::ToolApprovalRequired { .. }
            | LoopEvent::PlanComplete { .. }
            | LoopEvent::AgentSleeping { .. }
            | LoopEvent::UserMessage { .. }
            | LoopEvent::ClassifierDecision { .. }
            | LoopEvent::Finished { .. }
            | LoopEvent::Error { .. }
    )
}

fn event_to_sse(event: &AgenticEvent) -> Option<Event> {
    Event::default().json_data(event).ok()
}

async fn forward_loop_event(
    sse_tx: &mpsc::Sender<Result<Event, Infallible>>,
    session_id: &str,
    loop_event: LoopEvent,
    skipped_events: &mut usize,
) -> bool {
    let requires_delivery = loop_event_requires_delivery(&loop_event);

    if *skipped_events > 0 {
        let lagged_event = AgenticEvent::Lagged {
            skipped: *skipped_events,
        };

        if let Some(sse_event) = event_to_sse(&lagged_event) {
            if requires_delivery {
                if sse_tx.send(Ok(sse_event)).await.is_err() {
                    return false;
                }
                *skipped_events = 0;
            } else {
                match sse_tx.try_send(Ok(sse_event)) {
                    Ok(()) => *skipped_events = 0,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        *skipped_events = skipped_events.saturating_add(1);
                        tracing::warn!(
                            session_id,
                            skipped = *skipped_events,
                            "Dropping SSE event because client queue is full"
                        );
                        return true;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return false,
                }
            }
        } else {
            *skipped_events = 0;
        }
    }

    let agentic_event: AgenticEvent = loop_event.into();
    let Some(sse_event) = event_to_sse(&agentic_event) else {
        return true;
    };

    if requires_delivery {
        sse_tx.send(Ok(sse_event)).await.is_ok()
    } else {
        match sse_tx.try_send(Ok(sse_event)) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                *skipped_events = skipped_events.saturating_add(1);
                tracing::warn!(
                    session_id,
                    skipped = *skipped_events,
                    "Dropping SSE event because client queue is full"
                );
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

async fn start_orchestrator_sse(
    state: &AppState,
    ctx: ChatSessionContext,
    work_mode: WorkMode,
    permission_mode: PermissionMode,
    generate_title: bool,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let (sse_tx, sse_rx) = mpsc::channel::<Result<Event, Infallible>>(SSE_CHANNEL_BUFFER);

    let services = OrchestratorServices {
        ai_client: ctx.ai_client,
        tool_registry: Arc::clone(&state.tool_registry),
        process_registry: Arc::clone(&state.process_registry),
        db_path: (*state.db_path).clone(),
        skills_manager: Arc::clone(&state.skills_manager),
    };
    let mako_settings = ProjectSettings::load_mako_settings(ctx.project_dir.as_deref());

    let config = OrchestratorConfig {
        session_id: ctx.session_id.clone(),
        working_dir: ctx.working_dir,
        project_dir: ctx.project_dir,
        mako_crew_slug: ctx.mako_crew_slug.clone(),
        session_type: ctx.session_type,
        permission_mode,
        user_id: ctx.user_id.clone(),
        initial_work_mode: work_mode,
        generate_title,
        ..Default::default()
    };

    let (mut event_rx, input_tx) = if ctx.session_type == SessionType::Mako {
        use krusty_core::agent::tick_engine::{TickEngine, TickEngineConfig};
        let tick_config = TickEngineConfig {
            tick_interval: std::time::Duration::from_secs(mako_settings.tick_interval_secs),
            max_ticks: mako_settings.max_ticks,
            enabled: true,
        };
        TickEngine::run(services, config, tick_config, ctx.conversation, ctx.options)
    } else {
        let orchestrator = AgenticOrchestrator::new(services, config);
        orchestrator.run(ctx.conversation, ctx.options)
    };

    // Store input channel for tool approvals
    let session_id = ctx.session_id;
    {
        let mut inputs = state.session_inputs.write().await;
        inputs.insert(session_id.clone(), input_tx);
    }

    let session_inputs = Arc::clone(&state.session_inputs);
    let active_agent_streams = Arc::clone(&state.active_agent_streams);
    let push_service = state.push_service.clone();
    let apns_service = state.apns_service.clone();
    let user_id = ctx.user_id;
    let db_path = Arc::clone(&state.db_path);
    let guard = ctx.guard;

    tokio::spawn(async move {
        active_agent_streams.fetch_add(1, Ordering::Relaxed);
        let _guard = guard;
        let mut awaiting_input = false;
        let mut had_error = false;
        let mut stop_reason: Option<LoopStopReason> = None;
        let mut skipped_events = 0usize;

        while let Some(loop_event) = event_rx.recv().await {
            if let LoopEvent::Finished {
                stop_reason: ref reason,
                ..
            } = loop_event
            {
                stop_reason = Some(reason.clone());
            }
            let is_finished = matches!(loop_event, LoopEvent::Finished { .. });

            if matches!(loop_event, LoopEvent::AwaitingInput { .. }) {
                awaiting_input = true;
                fire_push(
                    &push_service,
                    user_id.as_deref(),
                    PushPayload {
                        title: "Krusty".into(),
                        body: "Krusty needs your input".into(),
                        session_id: Some(session_id.clone()),
                        tag: None,
                    },
                    PushEventType::AwaitingInput,
                );
                fire_apns(
                    &apns_service,
                    user_id.as_deref(),
                    ApnsPayload {
                        title: "Krusty".into(),
                        body: "Krusty needs your input".into(),
                        session_id: Some(session_id.clone()),
                        category: Some("TOOL_APPROVAL".into()),
                        data: Some(json!({
                            "type": "awaiting_input",
                            "sessionId": session_id.clone(),
                        })),
                    },
                    ApnsEventType::AwaitingInput,
                );
            }

            // APNs: tool approval required (not triggered by Web Push currently)
            if let LoopEvent::ToolApprovalRequired {
                ref id, ref name, ..
            } = loop_event
            {
                fire_apns(
                    &apns_service,
                    user_id.as_deref(),
                    ApnsPayload {
                        title: "Permission Required".into(),
                        body: format!("\"{name}\" is requesting permission to execute."),
                        session_id: Some(session_id.clone()),
                        category: Some("TOOL_APPROVAL".into()),
                        data: Some(serde_json::json!({
                            "requestId": id,
                            "sessionId": session_id.clone(),
                            "toolName": name,
                            "type": "tool_approval",
                        })),
                    },
                    ApnsEventType::ToolApproval,
                );
            }

            if matches!(loop_event, LoopEvent::Error { .. }) {
                had_error = true;
            }

            if !forward_loop_event(&sse_tx, &session_id, loop_event, &mut skipped_events).await {
                break;
            }

            if is_finished {
                break;
            }
        }

        // Fire push notification based on how the loop ended
        if !awaiting_input {
            if had_error {
                fire_push(
                    &push_service,
                    user_id.as_deref(),
                    PushPayload {
                        title: "Krusty".into(),
                        body: "Session encountered an error".into(),
                        session_id: Some(session_id.clone()),
                        tag: None,
                    },
                    PushEventType::Error,
                );
                fire_apns(
                    &apns_service,
                    user_id.as_deref(),
                    ApnsPayload {
                        title: "Krusty".into(),
                        body: "Session encountered an error".into(),
                        session_id: Some(session_id.clone()),
                        category: None,
                        data: Some(json!({
                            "type": "error",
                            "sessionId": session_id.clone(),
                        })),
                    },
                    ApnsEventType::Error,
                );
            } else if stop_reason == Some(LoopStopReason::Sleeping) {
                tracing::info!(
                    session_id = %session_id,
                    "Session entered sleeping state; skipping completion push"
                );
            } else {
                let title = session_title(&db_path, &session_id);
                fire_push(
                    &push_service,
                    user_id.as_deref(),
                    PushPayload {
                        title: "Krusty".into(),
                        body: format!("{title} is complete"),
                        session_id: Some(session_id.clone()),
                        tag: Some(format!("session-{session_id}")),
                    },
                    PushEventType::Completion,
                );
                fire_apns(
                    &apns_service,
                    user_id.as_deref(),
                    ApnsPayload {
                        title: format!("{title} — Complete"),
                        body: "Response finished".into(),
                        session_id: Some(session_id.clone()),
                        category: Some("STREAM_COMPLETE".into()),
                        data: Some(serde_json::json!({
                            "type": "stream_complete",
                            "sessionId": session_id.clone(),
                        })),
                    },
                    ApnsEventType::Completion,
                );
            }
        }

        // Clean up session input channel
        let mut inputs = session_inputs.write().await;
        inputs.remove(&session_id);
        active_agent_streams.fetch_sub(1, Ordering::Relaxed);
    });

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Tools allowed in Chat sessions — conversation only, no file/bash/code tools.
/// Web search/fetch are the only tools. Research mode adds agent + report tools.
const CHAT_ALLOWED_TOOLS: &[&str] = &["web_search", "web_fetch"];

/// Additional tools unlocked for Chat sessions when research mode is enabled.
const CHAT_RESEARCH_TOOLS: &[&str] = &["agent", "create_report", "list_reports", "read_report"];

/// Tools exclusive to Mako sessions -- excluded from Code sessions.
const MAKO_ONLY_TOOLS: &[&str] = &[
    "send_user_message",
    "sleep",
    "create_task",
    "update_task",
    "list_tasks",
    "create_report",
    "list_reports",
    "read_report",
];

/// Filter tools based on the session type.
///
/// - **Code**: all registered tools except Mako-only tools.
/// - **Chat**: only a minimal subset (no file/bash tools). When
///   `research_enabled` is true, the agent and report tools are included.
/// - **Mako**: all registered tools (Code tools + Mako extensions), executed
///   through the autonomous wake-driven runtime.
fn filter_tools_for_session_type(
    tools: Vec<AiTool>,
    session_type: SessionType,
    research_enabled: bool,
) -> Vec<AiTool> {
    let before = tools.len();
    let result = filter_tools_inner(tools, session_type, research_enabled);
    tracing::info!(
        session_type = ?session_type,
        before_count = before,
        after_count = result.len(),
        tool_names = ?result.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        "Tool filter applied"
    );
    result
}

fn filter_tools_inner(
    tools: Vec<AiTool>,
    session_type: SessionType,
    research_enabled: bool,
) -> Vec<AiTool> {
    match session_type {
        SessionType::Code => tools
            .into_iter()
            .filter(|t| !MAKO_ONLY_TOOLS.contains(&t.name.as_str()))
            .collect(),
        SessionType::Chat => tools
            .into_iter()
            .filter(|t| {
                CHAT_ALLOWED_TOOLS.contains(&t.name.as_str())
                    || (research_enabled && CHAT_RESEARCH_TOOLS.contains(&t.name.as_str()))
            })
            .collect(),
        SessionType::Mako => {
            // Mako gets everything: Code tools plus Mako-specific tools.
            // All tools are registered globally; here we just pass them through.
            tools
        }
    }
}

fn apply_thinking_config(
    ai_client: &AiClient,
    thinking_level: ThinkingLevel,
    options: &mut CallOptions,
) {
    if !thinking_level.is_enabled() {
        return;
    }

    let cfg = ai_client.config();
    let model_lower = cfg.model.to_ascii_lowercase();
    let is_codex = cfg.provider_id == ProviderId::OpenAI && model_lower.contains("codex");
    let is_anthropic_opus_4_6 = cfg.provider_id == ProviderId::Anthropic
        && (model_lower.contains("opus-4-6")
            || model_lower.contains("opus-4.6")
            || model_lower.contains("opus 4.6"));

    options.thinking = Some(ThinkingConfig::default());

    if is_codex {
        options.codex_reasoning_effort = Some(match thinking_level {
            ThinkingLevel::Off => return,
            ThinkingLevel::Low => CodexReasoningEffort::Low,
            ThinkingLevel::Medium => CodexReasoningEffort::Medium,
            ThinkingLevel::High => CodexReasoningEffort::High,
            ThinkingLevel::XHigh => CodexReasoningEffort::XHigh,
        });
    } else if is_anthropic_opus_4_6 {
        options.anthropic_adaptive_effort = Some(match thinking_level {
            ThinkingLevel::Off => return,
            ThinkingLevel::Low => AnthropicAdaptiveEffort::Low,
            ThinkingLevel::Medium => AnthropicAdaptiveEffort::Medium,
            ThinkingLevel::High => AnthropicAdaptiveEffort::High,
            ThinkingLevel::XHigh => AnthropicAdaptiveEffort::Max,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::State;
    use axum::Json;
    use tokio::sync::mpsc::error::TryRecvError;
    use tokio::sync::{mpsc, Mutex, RwLock};

    use krusty_core::agent::loop_events::LoopStopReason;
    use krusty_core::agent::{AgentCancellation, LoopEvent, LoopInput, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::ai::types::Content;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::{Database, SessionType, WorkspaceMode};
    use krusty_core::tools::registry::ToolRegistry;
    use krusty_core::SessionManager;

    use super::{build_user_content, chat, forward_loop_event, tool_approval, RequestedModel};
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
        let content = build_user_content(
            "describe this",
            &[ContentBlock::Image {
                source: crate::types::ImageSource::Url {
                    url: "https://example.com/image.png".to_string(),
                },
            }],
        );

        assert!(matches!(content.first(), Some(Content::Image { .. })));
        assert!(matches!(
            content.last(),
            Some(Content::Text { text }) if text == "describe this"
        ));
    }

    #[test]
    fn build_user_content_does_not_duplicate_message_when_text_block_exists() {
        let content = build_user_content(
            "fallback message",
            &[ContentBlock::Text {
                text: "block text".to_string(),
            }],
        );

        assert_eq!(content.len(), 1);
        assert!(matches!(
            content.first(),
            Some(Content::Text { text }) if text == "block text"
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
    async fn chat_creates_code_session_in_fresh_workspace_before_ai_setup() {
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
                assert_eq!(message, "No AI credentials configured");
            }
            Ok(_) => panic!("chat request should fail without configured AI credentials"),
            Err(_) => panic!("chat request should fail with bad request"),
        }

        let expected = fresh_project_dir.to_string_lossy().to_string();
        let session_manager =
            SessionManager::new(Database::new(&state.db_path).expect("database should open"));
        let sessions = session_manager
            .list_sessions_for_user(Some(expected.as_str()), Some("alice"))
            .expect("session listing should succeed");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_type, SessionType::Code);
        assert_eq!(sessions[0].workspace_mode, WorkspaceMode::Selected);
        assert_eq!(sessions[0].project_dir.as_deref(), Some(expected.as_str()));
        assert_eq!(sessions[0].working_dir.as_deref(), Some(expected.as_str()));
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
