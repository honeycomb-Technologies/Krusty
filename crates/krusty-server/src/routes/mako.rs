//! Mako dispatch and session management endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{AutonomousTaskStore, Database, SessionType, WorkspaceMode};
use krusty_core::SessionManager;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::SessionResponse;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dispatch", post(dispatch))
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id/status", get(session_status))
        .route("/sessions/:id/pause", post(pause_session))
        .route("/sessions/:id/resume", post(resume_session))
        .route("/sessions/:id", delete(cancel_session))
}

#[derive(Debug, Deserialize)]
struct DispatchRequest {
    task: String,
    project_dir: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct DispatchResponse {
    session_id: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct MakoSessionStatus {
    session_id: String,
    session_type: SessionType,
    title: String,
    tasks: Vec<krusty_core::storage::AutonomousTask>,
    agent_state: String,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

/// Submit a task to Mako.
///
/// Creates a Mako session, saves the task as the first user message,
/// and returns the session_id. The client connects via `/api/chat` SSE
/// to start execution.
async fn dispatch(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<DispatchRequest>,
) -> Result<(StatusCode, Json<DispatchResponse>), AppError> {
    let session_manager = open_session_manager(&state)?;

    let working_dir = req
        .project_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .or_else(|| state.working_dir.to_str());

    let session_id = session_manager.create_session_for_user_with_config(
        &req.task,
        req.model.as_deref(),
        working_dir,
        working_dir,
        WorkspaceMode::Selected,
        current_user_id(user.as_ref()),
        None,
        SessionType::Mako,
    )?;

    // Save the task as the first user message so the chat SSE stream picks it up
    let content_json = serde_json::json!([{ "type": "text", "text": req.task }]).to_string();
    session_manager.save_message(&session_id, "user", &content_json)?;

    Ok((
        StatusCode::CREATED,
        Json(DispatchResponse {
            session_id,
            status: "started".to_string(),
        }),
    ))
}

/// List all active Mako sessions.
async fn list_sessions(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<Vec<SessionResponse>>, AppError> {
    let session_manager = open_session_manager(&state)?;

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let all_sessions = session_manager.list_sessions_for_user(None, user_id)?;
    let mako_sessions: Vec<SessionResponse> = all_sessions
        .into_iter()
        .filter(|s| s.session_type == SessionType::Mako)
        .map(Into::into)
        .collect();

    Ok(Json(mako_sessions))
}

/// Get Mako session status including tasks and agent state.
async fn session_status(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<MakoSessionStatus>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_mako_session(&session_manager, &id, user.as_ref())?;

    let agent_state = session_manager
        .get_agent_state(&id)
        .map(|s| s.state)
        .unwrap_or_else(|| "idle".to_string());

    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let tasks = task_store.list_tasks(&id)?;

    Ok(Json(MakoSessionStatus {
        session_id: id,
        session_type: SessionType::Mako,
        title: session.title,
        tasks,
        agent_state,
    }))
}

/// Pause a Mako session (stub -- updates state but does not yet control the tick engine).
async fn pause_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;

    Ok(Json(OkResponse { ok: true }))
}

/// Resume a paused Mako session (stub).
async fn resume_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;

    Ok(Json(OkResponse { ok: true }))
}

/// Cancel a Mako session.
async fn cancel_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;

    session_manager.delete_session(&id)?;

    let mut locks = state.session_locks.write().await;
    locks.remove(&id);

    Ok(StatusCode::NO_CONTENT)
}

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
}

fn current_user_id(user: Option<&CurrentUser>) -> Option<&str> {
    user.and_then(|u| u.0.user_id.as_deref())
}

fn load_owned_mako_session(
    session_manager: &SessionManager,
    session_id: &str,
    user: Option<&CurrentUser>,
) -> Result<krusty_core::storage::SessionInfo, AppError> {
    let session = session_manager
        .get_session(session_id)?
        .ok_or_else(|| AppError::NotFound(format!("Mako session {} not found", session_id)))?;

    if session.session_type != SessionType::Mako {
        return Err(AppError::BadRequest(format!(
            "Session {} is not a Mako session",
            session_id
        )));
    }

    let user_id = current_user_id(user);
    if !session_manager.verify_session_ownership(session_id, user_id)? {
        return Err(AppError::NotFound(format!(
            "Mako session {} not found",
            session_id
        )));
    }

    Ok(session)
}

fn ensure_owned_mako_session(
    session_manager: &SessionManager,
    session_id: &str,
    user: Option<&CurrentUser>,
) -> Result<(), AppError> {
    load_owned_mako_session(session_manager, session_id, user)?;
    Ok(())
}
