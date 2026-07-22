//! Chat endpoint with SSE streaming via core orchestrator.

mod content;
mod interactions;
mod session;
mod stream;
mod stream_notify;
mod tools;

use std::convert::Infallible;

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, Sse},
    routing::post,
    Json, Router,
};
use krusty_core::ai::types::{ModelMessage, Role};
use krusty_core::storage::{Database, SessionType};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;
use tokio_stream::wrappers::ReceiverStream;

use self::content::{build_user_content, content_blocks_include_images, validate_content_blocks};
#[cfg(test)]
use self::interactions::deliver_steering_with_rollover;
pub(crate) use self::interactions::submit_tool_approval;
use self::interactions::{
    mako_control_error, mako_response_sse, steer, tool_approval, tool_result,
};
#[cfg(test)]
use self::session::{prepare_chat_contract_for_test, select_model_for_chat_request};
use self::session::{
    prepare_chat_route_session, refresh_chat_code_tool_surface, setup_chat_session,
    ChatSessionContext, RequestedModel,
};
use self::stream::start_orchestrator_sse;
#[cfg(test)]
use self::stream::{forward_loop_event, run_orchestrator_event_bridge};
use self::tools::{restrict_tools_to_allowlist, should_suppress_code_tools};
use super::session_access::{current_user_id, load_owned_session};
use crate::ai_bootstrap::{persist_current_model_key_selection, persist_current_model_selection};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::ChatRequest;
use crate::AppState;

const SSE_CHANNEL_BUFFER: usize = 256;
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(chat))
        .route("/steer", post(steer))
        .route("/tool-result", post(tool_result))
        .route("/tool-approval", post(tool_approval))
}
// ── Handlers ─────────────────────────────────────────────────────────

async fn chat(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<ReceiverStream<Result<Event, Infallible>>>, AppError> {
    validate_content_blocks(&req.content)?;

    let user_id = current_user_id(user.as_ref()).map(ToOwned::to_owned);
    let idempotency_key = super::mako::idempotency_key_from_headers(&headers)?;
    let requested_model =
        RequestedModel::from_request_parts(req.model.as_deref(), req.model_key.as_ref())?;
    let requested_session_type = req.session_type.unwrap_or(SessionType::Code);
    if req.session_id.is_none() && requested_session_type == SessionType::Mako {
        return Err(AppError::Conflict(
            "Create Mako sessions through POST /mako/dispatch before sending chat messages".into(),
        ));
    }
    if let Some(session_id) = req.session_id.as_deref() {
        let manager = SessionManager::new(Database::new(&state.db_path)?);
        let existing = load_owned_session(&manager, session_id, user.as_ref())?;
        if existing.session_type == SessionType::Mako
            && (req.model.is_some()
                || req.model_key.is_some()
                || req.target_branch.is_some()
                || req.mode.is_some()
                || req.permission_mode.is_some())
        {
            return Err(AppError::Conflict(
                "Mako model, branch, work mode, and permission mode are daemon-owned; send the message without mutation overrides".into(),
            ));
        }
    }
    let requires_vision = content_blocks_include_images(&req.content);
    let prepared = prepare_chat_route_session(
        &state,
        user.as_ref(),
        &req,
        requested_model,
        requested_session_type,
        requires_vision,
    )
    .await?;
    let session_id = prepared.session_id;
    let is_first_message = prepared.is_first_message;
    let pending_model_update = prepared.pending_model_update;

    let session_manager = SessionManager::new(Database::new(&state.db_path)?);
    let session = load_owned_session(&session_manager, &session_id, user.as_ref())?;
    if session.session_type == SessionType::Mako {
        if !req.content.is_empty() {
            return Err(AppError::BadRequest(
                "Mako daemon messages currently support text content only".to_string(),
            ));
        }
        let message = req.message.trim();
        if message.is_empty() {
            return Err(AppError::BadRequest(
                "A Mako message cannot be empty".to_string(),
            ));
        }
        let receiver = if state.mako_runtime.is_daemon_backed() {
            state
                .mako_runtime
                .begin_daemon_chat_turn_for_user(
                    &session_id,
                    message,
                    user_id.as_deref(),
                    is_first_message,
                    idempotency_key.as_deref(),
                )
                .await
                .map_err(mako_control_error)?
        } else {
            // Preserve the embedded runner used by focused tests. Its message
            // method persists and starts the run itself, unlike daemon IPC.
            let receiver = state
                .mako_runtime
                .subscribe_for_user(&session_id, user_id.as_deref())
                .await
                .map_err(mako_control_error)?;
            state
                .mako_runtime
                .send_message_for_user(
                    state.clone(),
                    session_id,
                    message,
                    user_id.as_deref(),
                    idempotency_key.as_deref(),
                )
                .await
                .map_err(mako_control_error)?;
            receiver
        };
        return Ok(mako_response_sse(receiver));
    }

    let mut ctx = setup_chat_session(
        &state,
        user.as_ref(),
        &session_id,
        requested_model,
        req.thinking_enabled,
        req.fast_mode,
        requires_vision,
    )
    .await?;

    if let Some(model_override) = pending_model_update {
        if model_override.is_none() {
            ctx.session_manager
                .update_session_model_selection(&session_id, None, None)?;
        } else {
            let runtime = ctx.ai_client.resolved_model();
            if let Some(metadata) = state.model_registry.get_model_by_key(&runtime.key).await {
                ctx.session_manager.update_session_model_selection(
                    &session_id,
                    Some(&runtime.key),
                    metadata.catalog_revision.as_deref(),
                )?;
                persist_current_model_key_selection(
                    &state.model_registry,
                    state.db_path.as_ref().as_path(),
                    user_id.as_deref(),
                    &runtime.key,
                )
                .await?;
            } else if let Some(model) = model_override.as_deref() {
                ctx.session_manager
                    .update_session_model(&session_id, Some(model))?;
                persist_current_model_selection(
                    &state.model_registry,
                    state.db_path.as_ref().as_path(),
                    user_id.as_deref(),
                    model,
                )
                .await?;
            }
        }
    }

    // Mako sessions always run in autonomous mode (classifier provides safety gate).
    // Resolve this before a mode refresh so the rebuilt schema uses the same
    // permission contract that execution will enforce.
    let permission_mode = if ctx.session_type == SessionType::Mako {
        PermissionMode::Autonomous
    } else {
        req.permission_mode.unwrap_or(ctx.permission_mode)
    };

    let mut work_mode = ctx.work_mode;
    if let Some(requested_mode) = req.mode {
        if ctx.session_type != SessionType::Code {
            return Err(AppError::Conflict(
                "Build and Plan modes are only available for Code conversations".into(),
            ));
        }
        if requested_mode != work_mode {
            ctx.session_manager
                .update_session_work_mode(&session_id, requested_mode)?;
            work_mode = requested_mode;
            refresh_chat_code_tool_surface(&state, &mut ctx, work_mode, permission_mode).await;
        }
    }

    if ctx.session_type == SessionType::Code
        && should_suppress_code_tools(&req.message, !req.content.is_empty())
    {
        tracing::info!(
            session_id = %session_id,
            "Suppressing coding tools for a deterministic non-tool turn"
        );
        ctx.options.tools = None;
        ctx.options.codex_parallel_tool_calls = false;
    }

    if let Some(allowed_tools) = req.allowed_tools.as_deref() {
        ctx.execution_tool_allowlist = Some(
            restrict_tools_to_allowlist(&mut ctx.options, allowed_tools)
                .map_err(AppError::BadRequest)?,
        );
    }

    let user_content = build_user_content(&req.message, &req.content)?;
    let user_content_json = serde_json::to_string(&user_content)?;

    ctx.conversation.push(ModelMessage {
        role: Role::User,
        content: user_content,
    });
    ctx.session_manager
        .save_message(&session_id, "user", &user_content_json)?;

    ctx.session_manager
        .update_session_permission_mode(&session_id, permission_mode)?;

    start_orchestrator_sse(&state, ctx, work_mode, permission_mode, is_first_message).await
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Tools allowed in Chat sessions — conversation only, no file/bash/code tools.
/// Web search/fetch are the only tools; research behavior is automatic.
#[cfg(test)]
mod tests;
