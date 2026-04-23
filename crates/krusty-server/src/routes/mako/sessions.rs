use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use krusty_core::storage::{
    AutonomousTask, AutonomousTaskStore, Database, MakoRunPriority, MakoRuntimeState,
    MakoRuntimeStateStore, RuntimeTraceEvent, SessionType, WorkspaceMode,
};
use krusty_core::SessionManager;

use super::super::session_access::{
    current_user_id, ensure_owned_session_of_type, load_agent_state_or_idle,
    load_owned_session_of_type, request_workspace_scope,
};
use super::{current, open_session_manager, OkResponse};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::AgenticEvent;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::resolve_optional_workspace_path;
use crate::AppState;

const DEFAULT_MAKO_REPLAY_LIMIT: usize = 50;
const MAX_MAKO_REPLAY_LIMIT: usize = 200;
const MAKO_EVENT_STREAM_BUFFER: usize = 256;

#[derive(Debug, Deserialize)]
pub(super) struct DispatchRequest {
    pub(super) task: String,
    pub(super) project_dir: Option<String>,
    pub(super) model: Option<String>,
    pub(super) start_at: Option<String>,
    pub(super) priority: Option<MakoRunPriority>,
    pub(super) crew_slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MessageRequest {
    pub(super) message: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScheduleRequest {
    pub(super) start_at: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PriorityRequest {
    pub(super) priority: MakoRunPriority,
}

#[derive(Debug, Deserialize)]
pub(super) struct CrewRequest {
    pub(super) crew_slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ObserveEventsQuery {
    pub(super) replay_limit: Option<usize>,
    pub(super) after_sequence: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct DispatchResponse {
    pub(super) session_id: String,
    pub(super) status: String,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoSessionSummary {
    pub(super) session_id: String,
    pub(super) title: String,
    pub(super) updated_at: String,
    pub(super) project_dir: Option<String>,
    pub(super) agent_state: String,
    pub(super) runtime: Option<MakoRuntimeState>,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoSessionStatus {
    pub(super) session_id: String,
    pub(super) session_type: SessionType,
    pub(super) title: String,
    pub(super) tasks: Vec<AutonomousTask>,
    pub(super) agent_state: String,
    pub(super) runtime: Option<MakoRuntimeState>,
    pub(super) cadence: current::MakoCadenceSummary,
}

#[derive(Debug, Serialize)]
pub(super) struct RecoverDaemonResponse {
    pub(super) ok: bool,
    pub(super) recovered_count: usize,
}

pub(super) async fn dispatch(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<DispatchRequest>,
) -> Result<(StatusCode, Json<DispatchResponse>), AppError> {
    let session_manager = open_session_manager(&state)?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());
    let task = req.task.trim();
    if task.is_empty() {
        return Err(AppError::BadRequest("task must not be empty".to_string()));
    }

    let working_dir = resolve_optional_workspace_path(
        req.project_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?
    .unwrap_or_else(|| workspace_scope.base_dir.to_string_lossy().into_owned());
    let start_at = parse_requested_wake_at(req.start_at.as_deref())?;
    let model = trimmed_nonempty(req.model.as_deref());
    let priority = req.priority.unwrap_or(MakoRunPriority::Normal);
    let crew_slug = req
        .crew_slug
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if let Some(crew_slug) = crew_slug.as_deref() {
        if !krusty_core::storage::is_valid_crew_slug(crew_slug) {
            return Err(AppError::BadRequest("invalid crew slug".to_string()));
        }
    }

    let session_id = session_manager.create_session_for_user_with_config(
        task,
        model,
        Some(working_dir.as_str()),
        Some(working_dir.as_str()),
        WorkspaceMode::Selected,
        current_user_id(user.as_ref()),
        None,
        SessionType::Mako,
    )?;
    let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
    runtime_store.set_priority(&session_id, priority)?;
    runtime_store.set_crew_slug(&session_id, crew_slug.as_deref())?;

    let content_json = serde_json::json!([{ "type": "text", "text": task }]).to_string();
    session_manager.save_message(&session_id, "user", &content_json)?;
    let status = if let Some(wake_at) = start_at {
        state
            .mako_runtime
            .schedule_session(
                &state,
                session_id.clone(),
                wake_at,
                "scheduled_dispatch",
                "scheduled",
            )
            .await?;
        "scheduled"
    } else {
        state
            .mako_runtime
            .start_or_restart_session(state.clone(), session_id.clone(), "dispatch")
            .await?;
        "started"
    };

    Ok((
        StatusCode::CREATED,
        Json(DispatchResponse {
            session_id,
            status: status.to_string(),
        }),
    ))
}

pub(super) async fn list_sessions(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<Vec<MakoSessionSummary>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let all_sessions =
        session_manager.list_sessions_for_user_by_type(None, user_id, SessionType::Mako)?;
    let runtime_states = runtime_store.list_states_for_sessions(
        &all_sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut mako_sessions = Vec::new();

    for session in all_sessions {
        let agent_state = load_agent_state_or_idle(&session_manager, &session.id)?.state;
        let runtime = runtime_states.get(&session.id).cloned();

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

pub(super) async fn recover_daemon(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<RecoverDaemonResponse>, AppError> {
    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let recoverable_states = {
        let session_manager = open_session_manager(&state)?;
        let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
        let mut states = Vec::new();
        for runtime_state in runtime_store.list_recoverable_states()? {
            let Some(session) = session_manager.get_session(&runtime_state.session_id)? else {
                continue;
            };
            if session.session_type != SessionType::Mako {
                continue;
            }
            if session.user_id.as_deref() != user_id {
                continue;
            }
            states.push(runtime_state);
        }
        states
    };
    let mut recovered_count = 0usize;

    for runtime_state in recoverable_states {
        state
            .mako_runtime
            .recover_persisted_state(state.clone(), &runtime_state, "manual_recover")
            .await?;
        recovered_count += 1;
    }

    Ok(Json(RecoverDaemonResponse {
        ok: true,
        recovered_count,
    }))
}

pub(super) async fn session_status(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<MakoSessionStatus>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());
    let session = load_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;

    let agent_state = load_agent_state_or_idle(&session_manager, &id)?.state;

    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let tasks = task_store.list_tasks(&id)?;
    let runtime = MakoRuntimeStateStore::new(Database::new(&state.db_path)?).get_state(&id)?;
    let cadence = current::load_mako_cadence(
        session.project_dir.as_deref(),
        session.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    );

    Ok(Json(MakoSessionStatus {
        session_id: id,
        session_type: SessionType::Mako,
        title: session.title,
        tasks,
        agent_state,
        runtime,
        cadence,
    }))
}

pub(super) async fn observe_events(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ObserveEventsQuery>,
) -> Result<Sse<ReceiverStream<std::result::Result<Event, Infallible>>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;

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

pub(super) async fn send_message(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;

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

pub(super) async fn pause_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;
    state.mako_runtime.pause_session(&state, &id).await?;
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn schedule_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<ScheduleRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;
    let wake_at = parse_requested_wake_at(Some(req.start_at.as_str()))?
        .ok_or_else(|| AppError::BadRequest("start_at must be provided".to_string()))?;
    state
        .mako_runtime
        .schedule_session(&state, id, wake_at, "manual_schedule", "scheduled")
        .await?;
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn set_priority(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<PriorityRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;
    let store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
    store.set_priority(&id, req.priority)?;
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn set_crew(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<CrewRequest>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;

    let crew_slug = req
        .crew_slug
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if let Some(crew_slug) = crew_slug.as_deref() {
        if !krusty_core::storage::is_valid_crew_slug(crew_slug) {
            return Err(AppError::BadRequest("invalid crew slug".to_string()));
        }
    }

    let store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
    store.set_crew_slug(&id, crew_slug.as_deref())?;
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn resume_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;
    state
        .mako_runtime
        .start_or_restart_session(state.clone(), id.clone(), "resume")
        .await?;
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn cancel_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session_of_type(
        &session_manager,
        &id,
        SessionType::Mako,
        "Mako",
        user.as_ref(),
    )?;

    state.mako_runtime.stop_active_run(&state, &id).await;
    state.mako_runtime.forget_session(&id).await;
    session_manager.delete_session(&id)?;

    let mut locks = state.session_locks.write().await;
    locks.remove(&id);

    Ok(StatusCode::NO_CONTENT)
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
        .filter_map(map_runtime_trace_event)
        .collect())
}

pub(super) fn map_runtime_trace_event(event: RuntimeTraceEvent) -> Option<AgenticEvent> {
    let sequence = event.sequence;
    let event_type = event.event_type.clone();
    let mapped = AgenticEvent::from_runtime_trace(event);
    if mapped.is_none() {
        tracing::warn!(
            sequence,
            event_type,
            "Skipping persisted runtime trace event that could not be replayed"
        );
    }
    mapped
}

fn parse_requested_wake_at(
    value: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let Some(raw) = trimmed_nonempty(value) else {
        return Ok(None);
    };

    let wake_at = chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| date.with_timezone(&chrono::Utc))
        .map_err(|_| {
            AppError::BadRequest("start_at must be a valid RFC3339 timestamp".to_string())
        })?;
    if wake_at <= chrono::Utc::now() {
        return Err(AppError::BadRequest(
            "start_at must be in the future".to_string(),
        ));
    }

    Ok(Some(wake_at))
}
