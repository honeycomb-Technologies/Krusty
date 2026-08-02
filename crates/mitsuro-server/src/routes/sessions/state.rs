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

const RECENT_DELEGATED_ARTIFACT_LIMIT: usize = 20;
const DELEGATED_HYDRATION_SUMMARY_LIMIT: usize = 10_000;

fn core_delegated_stage_is_terminal(stage: krusty_core::agent::DelegatedRunStage) -> bool {
    matches!(
        stage,
        krusty_core::agent::DelegatedRunStage::Complete
            | krusty_core::agent::DelegatedRunStage::Degraded
            | krusty_core::agent::DelegatedRunStage::Failed
            | krusty_core::agent::DelegatedRunStage::Cancelled
    )
}

/// Query params for retrieving a session trace.
#[derive(Debug, Deserialize)]
pub(super) struct GetSessionTraceQuery {
    /// Maximum number of trace events to return.
    pub limit: Option<usize>,
    /// Return only events strictly after this persisted sequence.
    pub after_sequence: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GetSessionStateQuery {
    /// Include the compact newest-run index used during full transcript
    /// hydration. Polling callers omit it to keep the hot state endpoint small.
    #[serde(default)]
    pub include_delegated_history: bool,
}

/// Get session agent state
///
/// Returns the current agent execution state (idle, streaming, tool_executing, etc.)
/// Used by frontend to determine if session has active processing.
pub(super) async fn get_session_state(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<GetSessionStateQuery>,
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
    let delegated_store = DelegatedRunStore::new(Database::new(&state.db_path)?);
    let recent_records =
        delegated_store.list_runs_for_session(&id, RECENT_DELEGATED_ARTIFACT_LIMIT)?;
    let run_summaries = if query.include_delegated_history {
        delegated_store.list_run_summaries_for_session(&id, DELEGATED_HYDRATION_SUMMARY_LIMIT)?
    } else {
        Vec::new()
    };
    let summaries_by_tool = run_summaries
        .iter()
        .map(|summary| (summary.parent_tool_call_id.as_str(), summary))
        .collect::<std::collections::HashMap<_, _>>();

    // The process-local map is only a live optimization. A durable terminal
    // row (or a newer continuation for the same tool call) always evicts a
    // stale Running projection from reconnect state.
    let mut delegated_tools = delegated_tools
        .into_iter()
        .filter(|live| {
            if let Some(summary) = summaries_by_tool.get(live.tool_call_id.as_str()) {
                return summary.delegated_run_id == live.delegated_run_id
                    && !core_delegated_stage_is_terminal(summary.stage);
            }

            match delegated_store.get_run(&live.delegated_run_id) {
                Ok(Some(record)) => {
                    record.parent_session_id == id
                        && record.parent_tool_call_id.as_deref() == Some(live.tool_call_id.as_str())
                        && !core_delegated_stage_is_terminal(record.stage)
                }
                Ok(None) => true,
                Err(error) => {
                    tracing::warn!(
                        delegated_run_id = %live.delegated_run_id,
                        %error,
                        "Could not reconcile live delegated snapshot with durable state"
                    );
                    true
                }
            }
        })
        .collect::<Vec<_>>();
    for record in &recent_records {
        let Some(snapshot) =
            crate::types::DelegatedToolStateResponse::from_active_durable_snapshot(record)
        else {
            continue;
        };
        if summaries_by_tool
            .get(snapshot.tool_call_id.as_str())
            .is_some_and(|summary| summary.delegated_run_id != snapshot.delegated_run_id)
        {
            continue;
        }
        if !delegated_tools.iter().any(|live| {
            live.delegated_run_id == snapshot.delegated_run_id
                && live.tool_call_id == snapshot.tool_call_id
        }) {
            delegated_tools.push(snapshot);
        }
    }
    let recent_delegated_runs = recent_records.into_iter().map(Into::into).collect();
    let delegated_run_summaries = run_summaries.into_iter().map(Into::into).collect();

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
        delegated_run_summaries,
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
