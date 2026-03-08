//! Chat endpoint with SSE streaming via core orchestrator.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
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

use krusty_core::agent::plan_handler::parse_plan_confirm_choice;
use krusty_core::agent::{
    AgenticOrchestrator, LoopEvent, LoopInput, OrchestratorConfig, OrchestratorServices,
};
use krusty_core::ai::client::{
    AiClient, AnthropicAdaptiveEffort, CallOptions, CodexReasoningEffort,
};
use krusty_core::ai::providers::ProviderId;
use krusty_core::ai::types::{Content, ImageContent, ModelMessage, Role, ThinkingConfig};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{Database, WorkMode};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::push::{PushEventType, PushPayload, PushService};
use crate::types::{
    AgenticEvent, ChatRequest, ContentBlock, ThinkingLevel, ToolApprovalRequest, ToolResultRequest,
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
    work_mode: WorkMode,
    user_id: Option<String>,
    guard: OwnedMutexGuard<()>,
}

/// Build user message content from content blocks (images) and text message.
fn build_user_content(message: &str, content_blocks: &[ContentBlock]) -> Vec<Content> {
    let mut contents: Vec<Content> = Vec::new();

    for block in content_blocks {
        match block {
            ContentBlock::Text { text } => {
                tracing::debug!("Content block: Text ({} chars)", text.len());
            }
            ContentBlock::Image { source } => match source {
                crate::types::ImageSource::Base64 { media_type, data } => {
                    tracing::debug!(
                        "Content block: Image (base64, media_type={}, data_len={})",
                        media_type,
                        data.len()
                    );
                }
                crate::types::ImageSource::Url { url } => {
                    tracing::debug!("Content block: Image (url={})", url);
                }
            },
        }
    }

    for block in content_blocks {
        match block {
            ContentBlock::Text { text } => {
                contents.push(Content::Text { text: text.clone() });
            }
            ContentBlock::Image { source } => {
                let image_content = match source {
                    crate::types::ImageSource::Base64 { media_type, data } => {
                        Some(Content::Image {
                            image: ImageContent {
                                base64: Some(data.clone()),
                                url: None,
                                media_type: Some(media_type.clone()),
                            },
                            detail: None,
                        })
                    }
                    crate::types::ImageSource::Url { url } => Some(Content::Image {
                        image: ImageContent {
                            base64: None,
                            url: Some(url.clone()),
                            media_type: None,
                        },
                        detail: None,
                    }),
                };
                if let Some(img) = image_content {
                    contents.push(img);
                }
            }
        }
    }

    if contents.is_empty() || !message.is_empty() {
        let has_text = contents.iter().any(|c| matches!(c, Content::Text { .. }));
        if !message.is_empty() && !has_text {
            contents.push(Content::Text {
                text: message.to_string(),
            });
        }
    }

    if contents.is_empty() {
        contents.push(Content::Text {
            text: message.to_string(),
        });
    }

    contents
}

fn resolve_model_override<'a>(
    requested_model: Option<&'a str>,
    session_model: Option<&'a str>,
) -> Option<&'a str> {
    requested_model
        .and_then(|model| {
            let trimmed = model.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
        .or_else(|| {
            session_model.and_then(|model| {
                let trimmed = model.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            })
        })
}

async fn setup_chat_session(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
    model_override: Option<&str>,
    thinking_level: ThinkingLevel,
) -> Result<ChatSessionContext, AppError> {
    let user_id = user.and_then(|u| u.0.user_id.clone());
    let user_home_dir = user.and_then(|u| u.0.home_dir.clone());
    let default_working_dir = user_home_dir
        .clone()
        .unwrap_or_else(|| (*state.working_dir).clone());

    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);

    if !session_manager.verify_session_ownership(session_id, user_id.as_deref())? {
        return Err(AppError::NotFound(format!(
            "Session {} not found",
            session_id
        )));
    }

    let session = session_manager
        .get_session(session_id)?
        .ok_or_else(|| AppError::NotFound(format!("Session {} not found", session_id)))?;

    let effective_model = resolve_model_override(model_override, session.model.as_deref());
    let ai_client = state
        .resolve_ai_client(effective_model)
        .await
        .ok_or_else(|| AppError::BadRequest("No AI credentials configured".to_string()))?;

    let working_dir = session
        .working_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or(default_working_dir);

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
    let conversation: Vec<ModelMessage> = raw_messages
        .into_iter()
        .filter_map(|(role_str, content_json)| {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            serde_json::from_str(&content_json)
                .ok()
                .map(|content| ModelMessage { role, content })
        })
        .collect();

    let ai_tools = state.tool_registry.get_ai_tools().await;
    let mut options = CallOptions {
        tools: Some(ai_tools),
        session_id: Some(session_id.to_string()),
        codex_parallel_tool_calls: true,
        ..Default::default()
    };
    if thinking_level.is_enabled() {
        apply_thinking_config(&ai_client, thinking_level, &mut options);
    }

    Ok(ChatSessionContext {
        ai_client,
        options,
        conversation,
        session_id: session_id.to_string(),
        session_manager,
        working_dir,
        work_mode: session.work_mode,
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
    let user_id = user.as_ref().and_then(|u| u.0.user_id.clone());
    let default_working_dir = user
        .as_ref()
        .and_then(|u| u.0.home_dir.clone())
        .unwrap_or_else(|| (*state.working_dir).clone());
    let model_override = resolve_model_override(req.model.as_deref(), None);

    let (session_id, is_first_message) = match req.session_id {
        Some(id) => {
            let db = Database::new(&state.db_path)?;
            let sm = SessionManager::new(db);
            if !sm.verify_session_ownership(&id, user_id.as_deref())? {
                return Err(AppError::NotFound(format!("Session {} not found", id)));
            }
            if let Some(ref model) = req.model {
                let normalized = if model.is_empty() {
                    None
                } else {
                    Some(model.as_str())
                };
                sm.update_session_model(&id, normalized)?;
            }
            let messages = sm.load_session_messages(&id)?;
            (id, messages.is_empty())
        }
        None => {
            let db = Database::new(&state.db_path)?;
            let sm = SessionManager::new(db);
            let title = SessionManager::generate_title_from_content(&req.message);
            let working_dir_str = default_working_dir.to_string_lossy().to_string();
            let id = sm.create_session_for_user(
                &title,
                model_override,
                Some(working_dir_str.as_str()),
                user_id.as_deref(),
            )?;
            (id, true)
        }
    };

    let mut ctx = setup_chat_session(
        &state,
        user.as_ref(),
        &session_id,
        model_override,
        req.thinking_enabled,
    )
    .await?;

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

    start_orchestrator_sse(
        &state,
        ctx,
        work_mode,
        req.permission_mode,
        is_first_message,
    )
    .await
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
        None,
        ThinkingLevel::Off,
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
    Json(req): Json<ToolApprovalRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let inputs = state.session_inputs.read().await;
    let sender = inputs
        .get(&req.session_id)
        .ok_or_else(|| AppError::NotFound("No active session".into()))?;
    let _ = sender.send(LoopInput::ToolApproval {
        tool_call_id: req.tool_call_id,
        approved: req.approved,
    });
    Ok(Json(json!({"status": "ok"})))
}

// ── Orchestrator → SSE bridge ────────────────────────────────────────

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

    let config = OrchestratorConfig {
        session_id: ctx.session_id.clone(),
        working_dir: ctx.working_dir,
        permission_mode,
        user_id: ctx.user_id.clone(),
        initial_work_mode: work_mode,
        generate_title,
        ..Default::default()
    };

    let orchestrator = AgenticOrchestrator::new(services, config);
    let (mut event_rx, input_tx) = orchestrator.run(ctx.conversation, ctx.options);

    // Store input channel for tool approvals
    let session_id = ctx.session_id;
    {
        let mut inputs = state.session_inputs.write().await;
        inputs.insert(session_id.clone(), input_tx);
    }

    let session_inputs = Arc::clone(&state.session_inputs);
    let push_service = state.push_service.clone();
    let user_id = ctx.user_id;
    let db_path = Arc::clone(&state.db_path);
    let guard = ctx.guard;

    tokio::spawn(async move {
        let _guard = guard;
        let mut awaiting_input = false;
        let mut had_error = false;

        while let Some(loop_event) = event_rx.recv().await {
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
            }

            if matches!(loop_event, LoopEvent::Error { .. }) {
                had_error = true;
            }

            let agentic_event: AgenticEvent = loop_event.into();
            if let Ok(sse_event) = Event::default().json_data(&agentic_event) {
                let _ = sse_tx.send(Ok(sse_event)).await;
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
            }
        }

        // Clean up session input channel
        let mut inputs = session_inputs.write().await;
        inputs.remove(&session_id);
    });

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ── Helpers ──────────────────────────────────────────────────────────

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
        && (model_lower.contains("opus-4-6") || model_lower.contains("opus 4.6"));

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
            ThinkingLevel::High | ThinkingLevel::XHigh => AnthropicAdaptiveEffort::High,
        });
    }
}

fn session_title(db_path: &Path, session_id: &str) -> String {
    match Database::new(db_path) {
        Ok(db) => {
            let session_manager = SessionManager::new(db);
            match session_manager.get_session(session_id) {
                Ok(Some(session)) => session.title,
                Ok(None) => "Session".to_string(),
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        "Failed to load session title: {}", e
                    );
                    "Session".to_string()
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to open database while loading session title: {}", e);
            "Session".to_string()
        }
    }
}

fn fire_push(
    push_service: &Option<Arc<PushService>>,
    user_id: Option<&str>,
    payload: PushPayload,
    event_type: PushEventType,
) {
    if let Some(svc) = push_service.clone() {
        let uid = user_id.map(String::from);
        tokio::spawn(async move {
            let stats = svc.notify_user(uid.as_deref(), payload, event_type).await;
            tracing::info!(
                event_type = event_type.as_str(),
                attempted = stats.attempted,
                sent = stats.sent,
                stale_removed = stats.stale_removed,
                failed = stats.failed,
                "Push event dispatched"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_model_override;

    #[test]
    fn resolve_model_override_prefers_request_and_trims_input() {
        assert_eq!(
            resolve_model_override(Some("  openai/gpt-5  "), Some("minimax/m2")),
            Some("openai/gpt-5")
        );
    }

    #[test]
    fn resolve_model_override_falls_back_to_session_model() {
        assert_eq!(
            resolve_model_override(None, Some("  anthropic/claude-opus-4.6  ")),
            Some("anthropic/claude-opus-4.6")
        );
    }

    #[test]
    fn resolve_model_override_ignores_empty_values() {
        assert_eq!(resolve_model_override(Some("   "), Some("   ")), None);
    }
}
