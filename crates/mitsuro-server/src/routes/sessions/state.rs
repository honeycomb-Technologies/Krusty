use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;

use mitsuro_core::storage::{Database, DelegatedRunStore, RuntimeTraceEvent, RuntimeTraceSummary};
use mitsuro_core::workflow::WorkflowManager;

use super::{
    ensure_owned_session, load_agent_state_or_idle, load_owned_session, open_session_manager,
};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{SessionStateResponse, SessionTraceResponse};
use crate::AppState;

/// Query params for retrieving a session trace.
#[derive(Debug, Deserialize)]
pub(super) struct GetSessionTraceQuery {
    /// Maximum number of trace events to return.
    pub limit: Option<usize>,
    /// Return only events strictly after this persisted sequence.
    pub after_sequence: Option<i64>,
}

/// Get session agent state
///
/// Returns the current agent execution state (idle, streaming, tool_executing, etc.)
/// Used by frontend to determine if session has active processing.
pub(super) async fn get_session_state(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<SessionStateResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &id, user.as_ref())?;
    let workflow = WorkflowManager::new(state.db_path.as_ref().clone())
        .map_err(|error| AppError::Internal(error.to_string()))?
        .get_snapshot(&id)
        .map_err(|error| AppError::Internal(error.to_string()))?;

    let agent_state = load_agent_state_or_idle(&session_manager, &id)?;
    let recovery = session_manager.load_recovery_state(&id)?;
    let pending_interactions = recovery
        .as_ref()
        .map(|recovery| recovery.pending_interactions.clone())
        .unwrap_or_default();
    let live_partial_assistant =
        live_partial_assistant_for_state(&agent_state.state, recovery.as_ref());
    let last_event_sequence = session_manager.load_runtime_trace_latest_sequence(&id)?;
    let delegated_tools = state
        .delegated_state
        .read()
        .await
        .get(&id)
        .cloned()
        .unwrap_or_default();
    let recent_delegated_runs = DelegatedRunStore::new(Database::new(&state.db_path)?)
        .list_runs_for_session(&id, 20)?
        .into_iter()
        .map(Into::into)
        .collect();

    Ok(Json(SessionStateResponse {
        id,
        agent_state: agent_state.state,
        started_at: agent_state.started_at,
        last_event_at: agent_state.last_event_at,
        mode: session.work_mode,
        permission_mode: session.permission_mode,
        workflow,
        recovery,
        pending_interactions,
        live_partial_assistant,
        delegated_tools,
        recent_delegated_runs,
        last_event_sequence,
    }))
}

/// Get compact runtime trace summary and recent events for a session.
pub(super) async fn get_session_trace(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<GetSessionTraceQuery>,
) -> Result<Json<SessionTraceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    const DEFAULT_TRACE_LIMIT: usize = 200;
    const MAX_TRACE_LIMIT: usize = 1_000;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_TRACE_LIMIT)
        .min(MAX_TRACE_LIMIT);
    let snapshot = session_manager.load_runtime_trace_events(&id, None)?;

    Ok(Json(trace_response_from_snapshot(
        id,
        snapshot,
        query.after_sequence,
        limit,
    )))
}

/// Build every response field from one exact persisted event snapshot.
///
/// Trace writes are intentionally asynchronous. Performing independent reads
/// for the summary, watermark, and event window allowed a batch commit between
/// those reads and produced internally contradictory responses. The retained
/// trace is bounded, so one ordered read is both cheap and a coherent snapshot.
fn trace_response_from_snapshot(
    id: String,
    snapshot: Vec<RuntimeTraceEvent>,
    after_sequence: Option<i64>,
    limit: usize,
) -> SessionTraceResponse {
    let summary = RuntimeTraceSummary::from_events(&snapshot);
    let latest_sequence = snapshot.last().map(|event| event.sequence);
    let latest_window_start = snapshot.len().saturating_sub(limit);
    let events = match after_sequence {
        Some(after_sequence) => snapshot
            .into_iter()
            .filter(|event| event.sequence > after_sequence)
            .take(limit)
            .collect(),
        None => snapshot.into_iter().skip(latest_window_start).collect(),
    };

    SessionTraceResponse {
        id,
        summary,
        events,
        latest_sequence,
    }
}

pub(super) fn live_partial_assistant_for_state(
    agent_state: &str,
    recovery: Option<&mitsuro_core::storage::SessionRecoveryState>,
) -> Option<mitsuro_core::storage::PartialAssistantState> {
    if matches!(
        agent_state,
        "streaming" | "tool_executing" | "awaiting_input"
    ) {
        return recovery.map(|recovery| recovery.partial_assistant.clone());
    }
    None
}
