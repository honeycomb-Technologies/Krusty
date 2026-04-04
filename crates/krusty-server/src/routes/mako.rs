//! Mako dispatch and session management endpoints

use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use krusty_core::storage::{
    AutonomousTask, AutonomousTaskStore, Database, MakoRuntimeState, MakoRuntimeStateStore,
    SessionInfo, SessionType, WorkspaceMode,
};
use krusty_core::SessionManager;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::AgenticEvent;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dispatch", post(dispatch))
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id/status", get(session_status))
        .route("/sessions/:id/events", get(observe_events))
        .route("/sessions/:id/message", post(send_message))
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

#[derive(Debug, Deserialize)]
struct MessageRequest {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ObserveEventsQuery {
    replay_limit: Option<usize>,
    after_sequence: Option<i64>,
}

const DEFAULT_MAKO_REPLAY_LIMIT: usize = 50;
const MAX_MAKO_REPLAY_LIMIT: usize = 200;
const MAKO_EVENT_STREAM_BUFFER: usize = 256;

#[derive(Debug, Serialize)]
struct DispatchResponse {
    session_id: String,
    status: String,
}

#[derive(Debug, Serialize)]
struct MakoSessionSummary {
    session_id: String,
    title: String,
    updated_at: String,
    project_dir: Option<String>,
    agent_state: String,
    runtime: Option<MakoRuntimeState>,
}

#[derive(Debug, Serialize)]
struct MakoSessionStatus {
    session_id: String,
    session_type: SessionType,
    title: String,
    tasks: Vec<AutonomousTask>,
    agent_state: String,
    runtime: Option<MakoRuntimeState>,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

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

    let content_json = serde_json::json!([{ "type": "text", "text": req.task }]).to_string();
    session_manager.save_message(&session_id, "user", &content_json)?;
    state
        .mako_runtime
        .start_or_restart_session(state.clone(), session_id.clone(), "dispatch")
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(DispatchResponse {
            session_id,
            status: "started".to_string(),
        }),
    ))
}

async fn list_sessions(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<Vec<MakoSessionSummary>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let all_sessions = session_manager.list_sessions_for_user(None, user_id)?;
    let mut mako_sessions = Vec::new();

    for session in all_sessions {
        if session.session_type != SessionType::Mako {
            continue;
        }

        let agent_state = session_manager
            .get_agent_state(&session.id)
            .map(|s| s.state)
            .unwrap_or_else(|| "idle".to_string());
        let runtime = runtime_store.get_state(&session.id)?;

        mako_sessions.push(MakoSessionSummary {
            session_id: session.id,
            title: session.title,
            updated_at: session.updated_at.to_rfc3339(),
            project_dir: session.project_dir,
            agent_state,
            runtime,
        });
    }

    Ok(Json(mako_sessions))
}

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
    let runtime = MakoRuntimeStateStore::new(Database::new(&state.db_path)?).get_state(&id)?;

    Ok(Json(MakoSessionStatus {
        session_id: id,
        session_type: SessionType::Mako,
        title: session.title,
        tasks,
        agent_state,
        runtime,
    }))
}

async fn observe_events(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ObserveEventsQuery>,
) -> Result<Sse<ReceiverStream<std::result::Result<Event, Infallible>>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;

    let mut receiver = state.mako_runtime.subscribe(&id).await;
    let replay_events = load_mako_replay_events(&session_manager, &id, &query)?;
    let (tx, rx) =
        mpsc::channel::<std::result::Result<Event, Infallible>>(MAKO_EVENT_STREAM_BUFFER);

    tokio::spawn(async move {
        for event in replay_events {
            let Ok(sse_event) = Event::default().json_data(event) else {
                continue;
            };
            if tx.send(Ok(sse_event)).await.is_err() {
                return;
            }
        }

        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let Ok(sse_event) = Event::default().json_data(event) else {
                        continue;
                    };
                    if tx.send(Ok(sse_event)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

async fn send_message(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;

    let message = req.message.trim();
    if message.is_empty() {
        return Err(AppError::BadRequest(
            "message must not be empty".to_string(),
        ));
    }

    let content_json = serde_json::json!([{ "type": "text", "text": message }]).to_string();
    session_manager.save_message(&id, "user", &content_json)?;
    state
        .mako_runtime
        .start_or_restart_session(state.clone(), id.clone(), "user_message")
        .await?;

    Ok(Json(OkResponse { ok: true }))
}

async fn pause_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;
    state.mako_runtime.pause_session(&state, &id).await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn resume_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;
    state
        .mako_runtime
        .start_or_restart_session(state.clone(), id.clone(), "resume")
        .await?;
    Ok(Json(OkResponse { ok: true }))
}

async fn cancel_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_mako_session(&session_manager, &id, user.as_ref())?;

    state.mako_runtime.stop_active_run(&state, &id).await;
    state.mako_runtime.forget_session(&id).await;
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
) -> Result<SessionInfo, AppError> {
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

fn load_mako_replay_events(
    session_manager: &SessionManager,
    session_id: &str,
    query: &ObserveEventsQuery,
) -> Result<Vec<AgenticEvent>, AppError> {
    let limit = query
        .replay_limit
        .unwrap_or(DEFAULT_MAKO_REPLAY_LIMIT)
        .min(MAX_MAKO_REPLAY_LIMIT);

    let trace_events = match query.after_sequence {
        Some(after_sequence) => session_manager.load_runtime_trace_events_after(
            session_id,
            after_sequence,
            Some(limit),
        )?,
        None if limit == 0 => Vec::new(),
        None => session_manager.load_runtime_trace_events(session_id, Some(limit))?,
    };

    Ok(trace_events
        .into_iter()
        .filter_map(AgenticEvent::from_runtime_trace)
        .collect())
}
