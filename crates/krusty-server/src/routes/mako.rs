//! Mako dispatch and session management endpoints

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::PathBuf;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use krusty_core::storage::{
    AutonomousTask, AutonomousTaskStore, Database, MakoRunPriority, MakoRuntimeState,
    MakoRuntimeStateStatus, MakoRuntimeStateStore, ProjectSettings, RuntimeTraceEvent,
    RuntimeTraceStore, SessionType, TaskStatus, WorkspaceMode,
};
use krusty_core::SessionManager;

use super::session_access::{
    current_user_id, ensure_owned_session_of_type, load_agent_state_or_idle,
    load_owned_session_of_type, request_workspace_scope,
};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::AgenticEvent;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::resolve_optional_workspace_path;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dispatch", post(dispatch))
        .route("/current", get(current))
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id/status", get(session_status))
        .route("/sessions/:id/events", get(observe_events))
        .route("/sessions/:id/message", post(send_message))
        .route("/sessions/:id/schedule", post(schedule_session))
        .route("/sessions/:id/priority", post(set_priority))
        .route("/sessions/:id/pause", post(pause_session))
        .route("/sessions/:id/resume", post(resume_session))
        .route("/sessions/:id", delete(cancel_session))
}

#[derive(Debug, Deserialize)]
struct DispatchRequest {
    task: String,
    project_dir: Option<String>,
    model: Option<String>,
    start_at: Option<String>,
    priority: Option<MakoRunPriority>,
}

#[derive(Debug, Deserialize)]
struct MessageRequest {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ScheduleRequest {
    start_at: String,
}

#[derive(Debug, Deserialize)]
struct PriorityRequest {
    priority: MakoRunPriority,
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
    cadence: MakoCadenceSummary,
}

#[derive(Debug, Serialize)]
struct MakoCurrentRunSummary {
    session_id: String,
    title: String,
    updated_at: String,
    project_dir: Option<String>,
    agent_state: String,
    runtime: Option<MakoRuntimeState>,
    pending_tasks: usize,
    in_progress_tasks: usize,
    completed_tasks: usize,
    failed_tasks: usize,
    blocked_tasks: usize,
    cadence: MakoCadenceSummary,
}

#[derive(Debug, Serialize)]
struct MakoPendingApprovalSummary {
    session_id: String,
    session_title: String,
    project_dir: Option<String>,
    tool_call_id: String,
    tool_name: String,
    arguments: Value,
    requested_at: String,
    priority: MakoRunPriority,
}

#[derive(Debug, Serialize)]
struct MakoStatusSummary {
    home_status: String,
    total_count: usize,
    running_count: usize,
    sleeping_count: usize,
    scheduled_count: usize,
    high_priority_count: usize,
    paused_count: usize,
    waiting_count: usize,
    failed_count: usize,
    idle_count: usize,
    pending_approvals_count: usize,
    next_wake_at: Option<String>,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct MakoCadenceSummary {
    tick_interval_secs: u64,
    max_ticks: usize,
}

#[derive(Debug, Serialize)]
struct MakoCurrentResponse {
    status: MakoStatusSummary,
    runs: Vec<MakoCurrentRunSummary>,
    approvals: Vec<MakoPendingApprovalSummary>,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Debug, Default, Clone, Copy)]
struct TaskCounts {
    pending: usize,
    in_progress: usize,
    completed: usize,
    failed: usize,
    blocked: usize,
}

async fn dispatch(
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
    MakoRuntimeStateStore::new(Database::new(&state.db_path)?)
        .set_priority(&session_id, priority)?;

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

async fn list_sessions(
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

async fn current(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoCurrentResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let trace_db = Database::new(&state.db_path)?;
    let trace_store = RuntimeTraceStore::new(&trace_db);
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let sessions =
        session_manager.list_sessions_for_user_by_type(None, user_id, SessionType::Mako)?;
    let runtime_states = runtime_store.list_states_for_sessions(
        &sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>(),
    )?;

    let mut runs = Vec::with_capacity(sessions.len());
    let mut running_count = 0usize;
    let mut sleeping_count = 0usize;
    let mut scheduled_count = 0usize;
    let mut high_priority_count = 0usize;
    let mut paused_count = 0usize;
    let mut waiting_count = 0usize;
    let mut failed_count = 0usize;
    let mut idle_count = 0usize;
    let mut next_wake_at: Option<String> = None;
    let mut approvals = Vec::new();

    for session in sessions {
        let agent_state = load_agent_state_or_idle(&session_manager, &session.id)?.state;
        let runtime = runtime_states.get(&session.id).cloned();
        let task_counts = summarize_tasks(&task_store.list_tasks(&session.id)?);
        let cadence = load_mako_cadence(
            session.project_dir.as_deref(),
            session.working_dir.as_deref(),
            &workspace_scope.base_dir,
            &workspace_scope.allowed_root,
        );
        let priority = runtime
            .as_ref()
            .map(|state| state.priority)
            .unwrap_or(MakoRunPriority::Normal);
        if priority == MakoRunPriority::High {
            high_priority_count += 1;
        }

        let run_state = classify_run_state(runtime.as_ref(), agent_state.as_str());
        match run_state {
            RunState::Running => running_count += 1,
            RunState::Scheduled => {
                scheduled_count += 1;
                if let Some(runtime) = runtime.as_ref() {
                    next_wake_at = earlier_timestamp(next_wake_at, runtime.next_wake_at.clone());
                }
            }
            RunState::Sleeping => {
                sleeping_count += 1;
                if let Some(runtime) = runtime.as_ref() {
                    next_wake_at = earlier_timestamp(next_wake_at, runtime.next_wake_at.clone());
                }
            }
            RunState::Paused => paused_count += 1,
            RunState::Waiting => {
                waiting_count += 1;
                approvals.extend(load_pending_approvals(
                    &trace_store,
                    &session.id,
                    &session.title,
                    session.project_dir.as_deref(),
                    priority,
                )?);
            }
            RunState::Failed => failed_count += 1,
            RunState::Idle => idle_count += 1,
        }

        runs.push(MakoCurrentRunSummary {
            session_id: session.id,
            title: session.title,
            updated_at: session.updated_at.to_rfc3339(),
            project_dir: session.project_dir,
            agent_state,
            runtime,
            pending_tasks: task_counts.pending,
            in_progress_tasks: task_counts.in_progress,
            completed_tasks: task_counts.completed,
            failed_tasks: task_counts.failed,
            blocked_tasks: task_counts.blocked,
            cadence,
        });
    }

    runs.sort_by(compare_run_summaries);
    approvals.sort_by(compare_pending_approvals);

    Ok(Json(MakoCurrentResponse {
        status: MakoStatusSummary {
            home_status: overall_home_status(
                running_count,
                sleeping_count,
                scheduled_count,
                paused_count,
                waiting_count,
                failed_count,
            )
            .to_string(),
            total_count: runs.len(),
            running_count,
            sleeping_count,
            scheduled_count,
            high_priority_count,
            paused_count,
            waiting_count,
            failed_count,
            idle_count,
            pending_approvals_count: approvals.len(),
            next_wake_at,
        },
        runs,
        approvals,
    }))
}

async fn session_status(
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
    let cadence = load_mako_cadence(
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

async fn observe_events(
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

async fn send_message(
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

async fn pause_session(
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

async fn schedule_session(
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

async fn set_priority(
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

async fn resume_session(
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

async fn cancel_session(
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

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
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

fn map_runtime_trace_event(event: RuntimeTraceEvent) -> Option<AgenticEvent> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
    Running,
    Scheduled,
    Sleeping,
    Paused,
    Waiting,
    Failed,
    Idle,
}

fn summarize_tasks(tasks: &[AutonomousTask]) -> TaskCounts {
    let completed_ids: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Completed)
        .map(|task| task.id.as_str())
        .collect();
    let mut counts = TaskCounts::default();

    for task in tasks {
        match task.status {
            TaskStatus::Pending => {
                counts.pending += 1;
                if task
                    .blocked_by
                    .iter()
                    .any(|dependency| !completed_ids.contains(dependency.as_str()))
                {
                    counts.blocked += 1;
                }
            }
            TaskStatus::InProgress => counts.in_progress += 1,
            TaskStatus::Completed => counts.completed += 1,
            TaskStatus::Failed => counts.failed += 1,
        }
    }

    counts
}

fn load_mako_cadence(
    project_dir: Option<&str>,
    working_dir: Option<&str>,
    workspace_base: &std::path::Path,
    allowed_root: &std::path::Path,
) -> MakoCadenceSummary {
    let resolved_project_dir =
        resolve_optional_workspace_path(project_dir.or(working_dir), workspace_base, allowed_root)
            .ok()
            .flatten()
            .map(PathBuf::from);
    let settings = ProjectSettings::load_mako_settings(resolved_project_dir.as_deref());

    MakoCadenceSummary {
        tick_interval_secs: settings.tick_interval_secs,
        max_ticks: settings.max_ticks,
    }
}

fn classify_run_state(runtime: Option<&MakoRuntimeState>, agent_state: &str) -> RunState {
    match runtime {
        Some(runtime)
            if runtime.status == MakoRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled") =>
        {
            RunState::Scheduled
        }
        Some(runtime) => match runtime.status {
            MakoRuntimeStateStatus::Running => RunState::Running,
            MakoRuntimeStateStatus::Sleeping => RunState::Sleeping,
            MakoRuntimeStateStatus::Paused => RunState::Paused,
            MakoRuntimeStateStatus::AwaitingInput => RunState::Waiting,
            MakoRuntimeStateStatus::Error => RunState::Failed,
            MakoRuntimeStateStatus::Cancelled | MakoRuntimeStateStatus::Idle => match agent_state {
                "streaming" | "tool_executing" => RunState::Running,
                "awaiting_input" => RunState::Waiting,
                "error" => RunState::Failed,
                _ => RunState::Idle,
            },
        },
        None => match agent_state {
            "streaming" | "tool_executing" => RunState::Running,
            "awaiting_input" => RunState::Waiting,
            "error" => RunState::Failed,
            _ => RunState::Idle,
        },
    }
}

fn overall_home_status(
    running_count: usize,
    sleeping_count: usize,
    scheduled_count: usize,
    paused_count: usize,
    waiting_count: usize,
    failed_count: usize,
) -> &'static str {
    if running_count > 0 {
        "awake"
    } else if waiting_count > 0 || failed_count > 0 {
        "blocked"
    } else if paused_count > 0 {
        "paused"
    } else if sleeping_count > 0 || scheduled_count > 0 {
        "sleeping"
    } else {
        "idle"
    }
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

fn earlier_timestamp(current: Option<String>, candidate: Option<String>) -> Option<String> {
    match (current, candidate) {
        (None, candidate) => candidate,
        (current, None) => current,
        (Some(current), Some(candidate)) => {
            if candidate < current {
                Some(candidate)
            } else {
                Some(current)
            }
        }
    }
}

fn load_pending_approvals(
    trace_store: &RuntimeTraceStore<'_>,
    session_id: &str,
    session_title: &str,
    project_dir: Option<&str>,
    priority: MakoRunPriority,
) -> Result<Vec<MakoPendingApprovalSummary>, AppError> {
    let mut pending = BTreeMap::new();

    for event in trace_store.list_events(session_id, Some(200))? {
        match event.event_type.as_str() {
            "tool_approval_required" => {
                let Some(tool_call_id) = event.payload.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(tool_name) = event.payload.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments = event
                    .payload
                    .get("arguments")
                    .cloned()
                    .unwrap_or(Value::Null);

                pending.insert(
                    tool_call_id.to_string(),
                    MakoPendingApprovalSummary {
                        session_id: session_id.to_string(),
                        session_title: session_title.to_string(),
                        project_dir: project_dir.map(str::to_string),
                        tool_call_id: tool_call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        arguments,
                        requested_at: event.created_at,
                        priority,
                    },
                );
            }
            "tool_approved" | "tool_denied" | "tool_result" => {
                if let Some(tool_call_id) = event.payload.get("id").and_then(Value::as_str) {
                    pending.remove(tool_call_id);
                }
            }
            _ => {}
        }
    }

    Ok(pending.into_values().collect())
}

fn compare_pending_approvals(
    left: &MakoPendingApprovalSummary,
    right: &MakoPendingApprovalSummary,
) -> std::cmp::Ordering {
    let priority_order = priority_rank(right.priority).cmp(&priority_rank(left.priority));
    if priority_order != std::cmp::Ordering::Equal {
        return priority_order;
    }

    let requested_order = left.requested_at.cmp(&right.requested_at);
    if requested_order != std::cmp::Ordering::Equal {
        return requested_order;
    }

    left.session_title
        .cmp(&right.session_title)
        .then_with(|| left.tool_name.cmp(&right.tool_name))
}

fn compare_run_summaries(
    left: &MakoCurrentRunSummary,
    right: &MakoCurrentRunSummary,
) -> std::cmp::Ordering {
    let left_priority = left
        .runtime
        .as_ref()
        .map(|runtime| runtime.priority)
        .unwrap_or(MakoRunPriority::Normal);
    let right_priority = right
        .runtime
        .as_ref()
        .map(|runtime| runtime.priority)
        .unwrap_or(MakoRunPriority::Normal);
    let priority_order = priority_rank(right_priority).cmp(&priority_rank(left_priority));
    if priority_order != std::cmp::Ordering::Equal {
        return priority_order;
    }

    let left_scheduled = left
        .runtime
        .as_ref()
        .map(|runtime| {
            runtime.status == MakoRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled")
        })
        .unwrap_or(false);
    let right_scheduled = right
        .runtime
        .as_ref()
        .map(|runtime| {
            runtime.status == MakoRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled")
        })
        .unwrap_or(false);

    if left_scheduled && right_scheduled {
        let wake_order = left
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.next_wake_at.as_ref())
            .cmp(
                &right
                    .runtime
                    .as_ref()
                    .and_then(|runtime| runtime.next_wake_at.as_ref()),
            );
        if wake_order != std::cmp::Ordering::Equal {
            return wake_order;
        }
    }

    right.updated_at.cmp(&left.updated_at)
}

fn priority_rank(priority: MakoRunPriority) -> u8 {
    match priority {
        MakoRunPriority::High => 2,
        MakoRunPriority::Normal => 1,
        MakoRunPriority::Low => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::extract::Path;
    use axum::extract::State;
    use axum::Json;
    use tokio::sync::{Mutex, RwLock};

    use krusty_core::agent::{AgentCancellation, LoopEvent, UserHookManager};
    use krusty_core::ai::models::create_model_registry;
    use krusty_core::mcp::McpManager;
    use krusty_core::process::ProcessRegistry;
    use krusty_core::skills::SkillsManager;
    use krusty_core::storage::credentials::CredentialStore;
    use krusty_core::storage::{
        Database, MakoRunPriority, MakoRuntimeStateStatus, MakoRuntimeStateStore,
        RuntimeTraceEvent, RuntimeTraceStore, SessionType, WorkspaceMode,
    };
    use krusty_core::tools::registry::ToolRegistry;
    use krusty_core::SessionManager;

    use super::{
        current, dispatch, list_sessions, map_runtime_trace_event, schedule_session,
        session_status, set_priority, DispatchRequest, PriorityRequest, ScheduleRequest,
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
    async fn dispatch_normalizes_model_before_persisting_session() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");

        let (_, Json(response)) = dispatch(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(DispatchRequest {
                task: "Investigate issue".to_string(),
                project_dir: None,
                model: Some("  openai/gpt-5  ".to_string()),
                start_at: None,
                priority: None,
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
        assert_eq!(session.model.as_deref(), Some("openai/gpt-5"));
    }

    #[tokio::test]
    async fn dispatch_resolves_relative_project_dir_against_user_home() {
        let (state, temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let user_root = temp_dir.join("alice-home");
        std::fs::create_dir_all(&user_root).expect("user root should exist");

        let (_, Json(response)) = dispatch(
            State(state.clone()),
            Some(current_user("alice", &user_root)),
            Json(DispatchRequest {
                task: "Investigate issue".to_string(),
                project_dir: Some("repo".to_string()),
                model: None,
                start_at: None,
                priority: None,
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
            Json(DispatchRequest {
                task: "   ".to_string(),
                project_dir: None,
                model: None,
                start_at: None,
                priority: None,
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
            Json(DispatchRequest {
                task: "Investigate issue".to_string(),
                project_dir: Some(outside_root.to_string_lossy().to_string()),
                model: None,
                start_at: None,
                priority: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn dispatch_can_schedule_future_run() {
        let (state, _temp_dir) = create_test_state();
        create_test_user(&state, "alice");
        let wake_at = chrono::Utc::now() + chrono::Duration::minutes(30);

        let (_, Json(response)) = dispatch(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(DispatchRequest {
                task: "Check CI later".to_string(),
                project_dir: None,
                model: None,
                start_at: Some(wake_at.to_rfc3339()),
                priority: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("scheduled dispatch should succeed"));

        assert_eq!(response.status, "scheduled");

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
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

        let (_, Json(response)) = dispatch(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(DispatchRequest {
                task: "Escalate production fix".to_string(),
                project_dir: None,
                model: None,
                start_at: None,
                priority: Some(MakoRunPriority::High),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("dispatch should succeed"));

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
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

        let (_, Json(response)) = dispatch(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Json(DispatchRequest {
                task: "Investigate issue".to_string(),
                project_dir: None,
                model: None,
                start_at: None,
                priority: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("dispatch should succeed"));

        let wake_at = chrono::Utc::now() + chrono::Duration::hours(2);
        let Json(_) = schedule_session(
            State(state.clone()),
            Some(current_user("alice", state.working_dir.as_ref())),
            Path(response.session_id.clone()),
            Json(ScheduleRequest {
                start_at: wake_at.to_rfc3339(),
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("schedule should succeed"));

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
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

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
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

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
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

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
        runtime_store
            .set_state(
                &session_id,
                MakoRuntimeStateStatus::AwaitingInput,
                None,
                Some("approval"),
                None,
                None,
                Some("user"),
                MakoRunPriority::High,
            )
            .expect("waiting state should persist");

        let trace_db = Database::new(&state.db_path).expect("database should open");
        let trace_store = RuntimeTraceStore::new(&trace_db);
        let approval_event = RuntimeTraceEvent::from_loop_event(
            "run-1",
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
        assert_eq!(summary.approvals[0].tool_call_id, "tool-1");
        assert_eq!(summary.approvals[0].tool_name, "bash");
        assert_eq!(summary.approvals[0].priority, MakoRunPriority::High);
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

        let runtime_store = MakoRuntimeStateStore::new(
            Database::new(&state.db_path).expect("database should open"),
        );
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
            payload: serde_json::json!({ "level": "info" }),
            failure_category: None,
            stop_reason: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        assert!(map_runtime_trace_event(event).is_none());
    }
}
