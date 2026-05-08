//! Chat endpoint with SSE streaming via core orchestrator.

mod content;
mod interactions;
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
use krusty_core::ai::types::{ModelMessage, Role};
use krusty_core::storage::{Database, SessionType, WorkspaceMode};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;

use self::content::{build_user_content, content_blocks_include_images, validate_content_blocks};
pub(crate) use self::interactions::submit_tool_approval;
use self::interactions::{tool_approval, tool_result};
use self::session::{
    select_model_for_chat_request, setup_chat_session, ChatSessionContext, RequestedModel,
};
use self::stream::start_orchestrator_sse;
#[cfg(test)]
use self::stream::{forward_loop_event, run_orchestrator_event_bridge};
use super::session_access::{current_user_id, ensure_owned_session, request_workspace_scope};
use crate::ai_bootstrap::{persist_current_model_selection, resolve_preferred_model};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::ChatRequest;
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
        req.fast_mode,
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

// ── Helpers ──────────────────────────────────────────────────────────

/// Tools allowed in Chat sessions — conversation only, no file/bash/code tools.
/// Web search/fetch are the only tools. Research mode adds agent + report tools.
#[cfg(test)]
mod tests;
