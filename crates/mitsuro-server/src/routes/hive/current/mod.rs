use std::path::PathBuf;

use axum::{extract::State, Json};
use serde::Serialize;
use serde_json::Value;

use mitsuro_core::storage::{
    AutonomousTaskStore, Database, HiveControllerEventStore, HiveControllerStore, HiveRunPriority,
    HiveRuntimeState, HiveRuntimeStateStore, ProjectSettings, ReportStore, SessionType,
};

use self::diagnostics::{
    build_run_diagnostic, classify_run_state, diagnostic_needs_attention, earlier_timestamp,
    has_due_soon_wake, is_stalled_diagnostic, later_timestamp, latest_task_activity_at,
    load_run_trace_diagnostics, overall_home_status,
    pending_approval_summaries_from_durable_events, run_has_open_work, summarize_health_state,
    summarize_queue_pressure, summarize_tasks,
};
use self::knowledge::summarize_knowledge_health;
use self::ordering::{compare_pending_approvals, compare_run_summaries};
use super::super::session_access::{
    current_user_id, load_agent_state_or_idle, request_workspace_scope, session_visible_to_user,
};
use super::open_session_manager;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::workspace::resolve_optional_workspace_path;
use crate::AppState;

mod diagnostics;
mod knowledge;
mod ordering;

pub(super) use self::diagnostics::parse_timestamp;

#[derive(Debug, Serialize)]
pub(super) struct HiveCurrentRunSummary {
    pub(super) session_id: String,
    pub(super) title: String,
    pub(super) updated_at: String,
    pub(super) project_dir: Option<String>,
    pub(super) target_branch: Option<String>,
    pub(super) agent_state: String,
    pub(super) runtime: Option<HiveRuntimeState>,
    pub(super) pending_tasks: usize,
    pub(super) in_progress_tasks: usize,
    pub(super) completed_tasks: usize,
    pub(super) failed_tasks: usize,
    pub(super) blocked_tasks: usize,
    pub(super) cadence: HiveCadenceSummary,
    pub(super) diagnostic: Option<HiveRunDiagnosticSummary>,
}

#[derive(Debug, Serialize, Clone)]
pub(super) struct HivePendingApprovalSummary {
    pub(super) session_id: String,
    pub(super) run_id: String,
    pub(super) session_title: String,
    pub(super) project_dir: Option<String>,
    pub(super) target_branch: Option<String>,
    pub(super) tool_call_id: String,
    pub(super) tool_name: String,
    pub(super) arguments: Value,
    pub(super) requested_at: String,
    pub(super) priority: HiveRunPriority,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveStatusSummary {
    pub(super) home_status: String,
    pub(super) total_count: usize,
    pub(super) running_count: usize,
    pub(super) sleeping_count: usize,
    pub(super) scheduled_count: usize,
    pub(super) high_priority_count: usize,
    pub(super) paused_count: usize,
    pub(super) waiting_count: usize,
    pub(super) failed_count: usize,
    pub(super) idle_count: usize,
    pub(super) pending_approvals_count: usize,
    pub(super) next_wake_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveRunDiagnosticSummary {
    pub(super) kind: String,
    pub(super) severity: String,
    pub(super) summary: String,
    pub(super) detail: String,
    pub(super) last_activity_at: Option<String>,
    pub(super) last_trace_at: Option<String>,
    pub(super) stalled_for_secs: Option<u64>,
    pub(super) overdue_by_secs: Option<u64>,
    pub(super) failure_streak: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveKnowledgeHealthSummary {
    pub(super) scope_count: usize,
    pub(super) healthy_scope_count: usize,
    pub(super) missing_snapshot_count: usize,
    pub(super) stale_snapshot_count: usize,
    pub(super) latest_snapshot_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveDiagnosticsSummary {
    pub(super) degraded_count: usize,
    pub(super) stalled_count: usize,
    pub(super) overdue_wake_count: usize,
    pub(super) repeating_failure_count: usize,
    pub(super) open_run_count: usize,
    pub(super) attention_run_count: usize,
    pub(super) due_soon_wake_count: usize,
    pub(super) health_state: String,
    pub(super) queue_pressure: String,
    pub(super) latest_trace_at: Option<String>,
    pub(super) daemon: HiveDaemonSummary,
    pub(super) knowledge: HiveKnowledgeHealthSummary,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveDaemonSummary {
    pub(super) uptime_secs: u64,
    pub(super) active_runtime_count: usize,
    pub(super) scheduled_wake_count: usize,
    pub(super) event_stream_count: usize,
    pub(super) recoverable_session_count: usize,
}

#[derive(Debug, Serialize, Clone, Copy)]
pub(super) struct HiveCadenceSummary {
    pub(super) tick_interval_secs: u64,
    pub(super) max_ticks: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct HiveCurrentResponse {
    pub(super) status: HiveStatusSummary,
    pub(super) diagnostics: HiveDiagnosticsSummary,
    pub(super) runs: Vec<HiveCurrentRunSummary>,
    pub(super) approvals: Vec<HivePendingApprovalSummary>,
}

pub(super) async fn current(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<HiveCurrentResponse>, AppError> {
    Ok(Json(
        build_hive_current_response(&state, user.as_ref()).await?,
    ))
}

pub(super) async fn build_hive_current_response(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> Result<HiveCurrentResponse, AppError> {
    let user_id = current_user_id(user);
    let sessions = {
        let session_manager = open_session_manager(state)?;
        session_manager
            .list_sessions_for_user_by_type(None, user_id, SessionType::Hive)?
            .into_iter()
            .filter(|session| session_visible_to_user(session, user_id))
            .collect::<Vec<_>>()
    };
    let session_ids = sessions
        .iter()
        .map(|session| session.id.clone())
        .collect::<Vec<_>>();
    let daemon_stats = state
        .hive_runtime
        .stats_for_sessions_for_user(&session_ids, user_id)
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    let session_manager = open_session_manager(state)?;
    let runtime_store = HiveRuntimeStateStore::new(Database::new(&state.db_path)?);
    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let report_store = ReportStore::new(Database::new(&state.db_path)?);
    let trace_db = Database::new(&state.db_path)?;
    let trace_store = mitsuro_core::storage::RuntimeTraceStore::new(&trace_db);
    let controller_store = HiveControllerStore::new(Database::new(&state.db_path)?);
    let controller_event_store = HiveControllerEventStore::new(Database::new(&state.db_path)?);
    let workspace_scope = request_workspace_scope(state, user);
    let runtime_states = runtime_store.list_states_for_sessions(&session_ids)?;
    let recoverable_session_ids = runtime_store
        .list_recoverable_states()?
        .into_iter()
        .map(|state| state.session_id)
        .collect::<std::collections::HashSet<_>>();

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
    let mut degraded_count = 0usize;
    let mut stalled_count = 0usize;
    let mut overdue_wake_count = 0usize;
    let mut repeating_failure_count = 0usize;
    let mut open_run_count = 0usize;
    let mut attention_run_count = 0usize;
    let mut due_soon_wake_count = 0usize;
    let mut latest_trace_at: Option<String> = None;

    for session in &sessions {
        let agent_state = load_agent_state_or_idle(&session_manager, &session.id)?.state;
        let runtime = runtime_states.get(&session.id).cloned();
        let tasks = task_store.list_tasks(&session.id)?;
        let task_counts = summarize_tasks(&tasks);
        let priority = runtime
            .as_ref()
            .map(|state| state.priority)
            .unwrap_or(HiveRunPriority::Normal);
        let pending_approvals =
            if let Some(controller) = controller_store.get_by_session(&session.id)? {
                let events = controller_event_store.list_pending_tool_approvals(&controller.id)?;
                pending_approval_summaries_from_durable_events(
                    &events,
                    &session.id,
                    &session.title,
                    session.project_dir.as_deref(),
                    session.target_branch.as_deref(),
                    priority,
                )
            } else {
                Vec::new()
            };
        let trace_diagnostics =
            load_run_trace_diagnostics(&trace_store, &session.id, pending_approvals)?;
        let cadence = load_hive_cadence(
            session.project_dir.as_deref(),
            session.working_dir.as_deref(),
            &workspace_scope.base_dir,
            &workspace_scope.allowed_root,
        );
        if priority == HiveRunPriority::High {
            high_priority_count += 1;
        }

        let run_state = classify_run_state(runtime.as_ref(), agent_state.as_str());
        match run_state {
            diagnostics::RunState::Running => running_count += 1,
            diagnostics::RunState::Scheduled => {
                scheduled_count += 1;
                if let Some(runtime) = runtime.as_ref() {
                    next_wake_at = earlier_timestamp(next_wake_at, runtime.next_wake_at.clone());
                }
            }
            diagnostics::RunState::Sleeping => {
                sleeping_count += 1;
                if let Some(runtime) = runtime.as_ref() {
                    next_wake_at = earlier_timestamp(next_wake_at, runtime.next_wake_at.clone());
                }
            }
            diagnostics::RunState::Paused => paused_count += 1,
            diagnostics::RunState::Waiting => waiting_count += 1,
            diagnostics::RunState::Failed => failed_count += 1,
            diagnostics::RunState::Idle => idle_count += 1,
        }

        if run_has_open_work(run_state, &task_counts) {
            open_run_count += 1;
        }
        if has_due_soon_wake(runtime.as_ref()) {
            due_soon_wake_count += 1;
        }

        latest_trace_at =
            later_timestamp(latest_trace_at, trace_diagnostics.latest_trace_at.clone());
        approvals.extend(trace_diagnostics.pending_approvals.clone());

        let diagnostic = build_run_diagnostic(
            session,
            runtime.as_ref(),
            run_state,
            &task_counts,
            latest_task_activity_at(&tasks),
            &trace_diagnostics,
        );
        if let Some(diagnostic) = diagnostic.as_ref() {
            degraded_count += 1;
            if diagnostic_needs_attention(diagnostic.kind.as_str()) {
                attention_run_count += 1;
            }
            if is_stalled_diagnostic(diagnostic.kind.as_str()) {
                stalled_count += 1;
            }
            if diagnostic.kind == "overdue_wake" {
                overdue_wake_count += 1;
            }
            if diagnostic.failure_streak > 1 {
                repeating_failure_count += 1;
            }
        }

        runs.push(HiveCurrentRunSummary {
            session_id: session.id.clone(),
            title: session.title.clone(),
            updated_at: session.updated_at.to_rfc3339(),
            project_dir: session.project_dir.clone(),
            target_branch: session.target_branch.clone(),
            agent_state,
            runtime,
            pending_tasks: task_counts.pending,
            in_progress_tasks: task_counts.in_progress,
            completed_tasks: task_counts.completed,
            failed_tasks: task_counts.failed,
            blocked_tasks: task_counts.blocked,
            cadence,
            diagnostic,
        });
    }

    let knowledge = summarize_knowledge_health(&state.db_path, &report_store, &sessions, user_id)?;
    runs.sort_by(compare_run_summaries);
    approvals.sort_by(compare_pending_approvals);

    Ok(HiveCurrentResponse {
        status: HiveStatusSummary {
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
        diagnostics: HiveDiagnosticsSummary {
            degraded_count,
            stalled_count,
            overdue_wake_count,
            repeating_failure_count,
            open_run_count,
            attention_run_count,
            due_soon_wake_count,
            health_state: summarize_health_state(
                stalled_count,
                overdue_wake_count,
                repeating_failure_count,
                attention_run_count,
                approvals.len(),
            )
            .to_string(),
            queue_pressure: summarize_queue_pressure(
                attention_run_count,
                approvals.len(),
                open_run_count,
                due_soon_wake_count,
            )
            .to_string(),
            latest_trace_at,
            daemon: HiveDaemonSummary {
                uptime_secs: daemon_stats.uptime_secs,
                active_runtime_count: daemon_stats.active_runtime_count,
                scheduled_wake_count: daemon_stats.scheduled_wake_count,
                event_stream_count: daemon_stats.event_stream_count,
                recoverable_session_count: sessions
                    .iter()
                    .filter(|session| recoverable_session_ids.contains(session.id.as_str()))
                    .count(),
            },
            knowledge,
        },
        runs,
        approvals,
    })
}

pub(super) fn load_hive_cadence(
    project_dir: Option<&str>,
    working_dir: Option<&str>,
    workspace_base: &std::path::Path,
    allowed_root: &std::path::Path,
) -> HiveCadenceSummary {
    let resolved_project_dir =
        resolve_optional_workspace_path(project_dir.or(working_dir), workspace_base, allowed_root)
            .ok()
            .flatten()
            .map(PathBuf::from);
    let settings = ProjectSettings::load_hive_settings(resolved_project_dir.as_deref());

    HiveCadenceSummary {
        tick_interval_secs: settings.tick_interval_secs,
        max_ticks: settings.max_ticks,
    }
}
