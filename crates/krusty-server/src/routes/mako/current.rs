use std::collections::BTreeMap;
use std::path::PathBuf;

use axum::{extract::State, Json};
use serde::Serialize;
use serde_json::Value;

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::storage::{
    AutonomousTask, AutonomousTaskStore, Database, MakoRunPriority, MakoRuntimeState,
    MakoRuntimeStateStatus, MakoRuntimeStateStore, MemoryStore, ProjectSettings, ReportStore,
    RuntimeTraceEvent, RuntimeTraceStore, RuntimeTraceSummary, SessionInfo, SessionType,
    TaskStatus, TraceFailureCategory, CURRENT_SNAPSHOT_TITLE,
};

use super::super::session_access::{
    current_user_id, load_agent_state_or_idle, request_workspace_scope,
};
use super::open_session_manager;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::workspace::resolve_optional_workspace_path;
use crate::AppState;

const ACTIVE_STALE_SECS: i64 = 30 * 60;
const WAITING_STALE_SECS: i64 = 15 * 60;
const QUEUED_STALE_SECS: i64 = 60 * 60;
const OVERDUE_WAKE_GRACE_SECS: i64 = 5 * 60;

#[derive(Debug, Serialize)]
pub(super) struct MakoCurrentRunSummary {
    pub(super) session_id: String,
    pub(super) title: String,
    pub(super) updated_at: String,
    pub(super) project_dir: Option<String>,
    pub(super) agent_state: String,
    pub(super) runtime: Option<MakoRuntimeState>,
    pub(super) pending_tasks: usize,
    pub(super) in_progress_tasks: usize,
    pub(super) completed_tasks: usize,
    pub(super) failed_tasks: usize,
    pub(super) blocked_tasks: usize,
    pub(super) cadence: MakoCadenceSummary,
    pub(super) diagnostic: Option<MakoRunDiagnosticSummary>,
}

#[derive(Debug, Serialize, Clone)]
pub(super) struct MakoPendingApprovalSummary {
    pub(super) session_id: String,
    pub(super) session_title: String,
    pub(super) project_dir: Option<String>,
    pub(super) tool_call_id: String,
    pub(super) tool_name: String,
    pub(super) arguments: Value,
    pub(super) requested_at: String,
    pub(super) priority: MakoRunPriority,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoStatusSummary {
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
pub(super) struct MakoRunDiagnosticSummary {
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
pub(super) struct MakoKnowledgeHealthSummary {
    pub(super) scope_count: usize,
    pub(super) healthy_scope_count: usize,
    pub(super) missing_snapshot_count: usize,
    pub(super) stale_snapshot_count: usize,
    pub(super) latest_snapshot_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoDiagnosticsSummary {
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
    pub(super) daemon: MakoDaemonSummary,
    pub(super) knowledge: MakoKnowledgeHealthSummary,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoDaemonSummary {
    pub(super) uptime_secs: u64,
    pub(super) active_runtime_count: usize,
    pub(super) scheduled_wake_count: usize,
    pub(super) event_stream_count: usize,
    pub(super) recoverable_session_count: usize,
}

#[derive(Debug, Serialize, Clone, Copy)]
pub(super) struct MakoCadenceSummary {
    pub(super) tick_interval_secs: u64,
    pub(super) max_ticks: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct MakoCurrentResponse {
    pub(super) status: MakoStatusSummary,
    pub(super) diagnostics: MakoDiagnosticsSummary,
    pub(super) runs: Vec<MakoCurrentRunSummary>,
    pub(super) approvals: Vec<MakoPendingApprovalSummary>,
}

#[derive(Debug, Default, Clone, Copy)]
struct TaskCounts {
    pending: usize,
    in_progress: usize,
    completed: usize,
    failed: usize,
    blocked: usize,
}

#[derive(Debug, Default, Clone)]
struct RunTraceDiagnostics {
    latest_trace_at: Option<String>,
    latest_run_summary: RuntimeTraceSummary,
    failure_streak: usize,
    pending_approvals: Vec<MakoPendingApprovalSummary>,
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

pub(super) async fn current(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<MakoCurrentResponse>, AppError> {
    Ok(Json(
        build_mako_current_response(&state, user.as_ref()).await?,
    ))
}

pub(super) async fn build_mako_current_response(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> Result<MakoCurrentResponse, AppError> {
    let user_id = current_user_id(user);
    let sessions = {
        let session_manager = open_session_manager(state)?;
        session_manager.list_sessions_for_user_by_type(None, user_id, SessionType::Mako)?
    };
    let session_ids = sessions
        .iter()
        .map(|session| session.id.clone())
        .collect::<Vec<_>>();
    let daemon_stats = state.mako_runtime.stats_for_sessions(&session_ids).await;
    let session_manager = open_session_manager(state)?;
    let runtime_store = MakoRuntimeStateStore::new(Database::new(&state.db_path)?);
    let task_store = AutonomousTaskStore::new(Database::new(&state.db_path)?);
    let memory_store = MemoryStore::new(Database::new(&state.db_path)?);
    let report_store = ReportStore::new(Database::new(&state.db_path)?);
    let trace_db = Database::new(&state.db_path)?;
    let trace_store = RuntimeTraceStore::new(&trace_db);
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
        let trace_diagnostics = load_run_trace_diagnostics(
            &trace_store,
            &session.id,
            &session.title,
            session.project_dir.as_deref(),
            runtime
                .as_ref()
                .map(|state| state.priority)
                .unwrap_or(MakoRunPriority::Normal),
        )?;
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
            RunState::Waiting => waiting_count += 1,
            RunState::Failed => failed_count += 1,
            RunState::Idle => idle_count += 1,
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

        runs.push(MakoCurrentRunSummary {
            session_id: session.id.clone(),
            title: session.title.clone(),
            updated_at: session.updated_at.to_rfc3339(),
            project_dir: session.project_dir.clone(),
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

    let knowledge = summarize_knowledge_health(&memory_store, &report_store, &sessions, user_id)?;
    runs.sort_by(compare_run_summaries);
    approvals.sort_by(compare_pending_approvals);

    Ok(MakoCurrentResponse {
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
        diagnostics: MakoDiagnosticsSummary {
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
            daemon: MakoDaemonSummary {
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

fn latest_task_activity_at(tasks: &[AutonomousTask]) -> Option<String> {
    tasks
        .iter()
        .map(|task| task.updated_at.as_str())
        .max()
        .map(str::to_string)
}

pub(super) fn load_mako_cadence(
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

fn load_run_trace_diagnostics(
    trace_store: &RuntimeTraceStore<'_>,
    session_id: &str,
    session_title: &str,
    project_dir: Option<&str>,
    priority: MakoRunPriority,
) -> Result<RunTraceDiagnostics, AppError> {
    let events = trace_store.list_events(session_id, Some(200))?;
    let latest_trace_at = events.last().map(|event| event.created_at.clone());
    let latest_run_summary = summarize_latest_run_from_events(&events);
    let failure_streak = recent_failure_streak(&events);
    let pending_approvals = load_pending_approvals_from_events(
        &events,
        session_id,
        session_title,
        project_dir,
        priority,
    );

    Ok(RunTraceDiagnostics {
        latest_trace_at,
        latest_run_summary,
        failure_streak,
        pending_approvals,
    })
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

fn load_pending_approvals_from_events(
    events: &[RuntimeTraceEvent],
    session_id: &str,
    session_title: &str,
    project_dir: Option<&str>,
    priority: MakoRunPriority,
) -> Vec<MakoPendingApprovalSummary> {
    let mut pending = BTreeMap::new();

    for event in events {
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
                        requested_at: event.created_at.clone(),
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

    pending.into_values().collect()
}

fn summarize_latest_run_from_events(events: &[RuntimeTraceEvent]) -> RuntimeTraceSummary {
    let Some(run_id) = events.last().map(|event| event.run_id.as_str()) else {
        return RuntimeTraceSummary::default();
    };
    let latest_run = events
        .iter()
        .filter(|event| event.run_id == run_id)
        .cloned()
        .collect::<Vec<_>>();
    RuntimeTraceSummary::from_events(&latest_run)
}

fn recent_failure_streak(events: &[RuntimeTraceEvent]) -> usize {
    if events.is_empty() {
        return 0;
    }

    let mut ordered_run_ids = Vec::new();
    for event in events.iter().rev() {
        if ordered_run_ids.last().copied() != Some(event.run_id.as_str()) {
            ordered_run_ids.push(event.run_id.as_str());
        }
    }

    let mut streak = 0usize;
    for run_id in ordered_run_ids {
        let summary = RuntimeTraceSummary::from_events(
            &events
                .iter()
                .filter(|event| event.run_id == run_id)
                .cloned()
                .collect::<Vec<_>>(),
        );
        if run_summary_failed(&summary) {
            streak += 1;
        } else {
            break;
        }
    }
    streak
}

fn run_summary_failed(summary: &RuntimeTraceSummary) -> bool {
    summary.agent_errors > 0
        || summary.provider_failures > 0
        || summary.tool_errors > 0
        || summary.server_tool_errors > 0
        || matches!(
            summary.last_stop_reason,
            Some(
                LoopStopReason::ProviderError
                    | LoopStopReason::BudgetExhausted
                    | LoopStopReason::LoopGuardTriggered
                    | LoopStopReason::StreamIdleTimeout
                    | LoopStopReason::PinchFailed
            )
        )
}

fn build_run_diagnostic(
    session: &SessionInfo,
    runtime: Option<&MakoRuntimeState>,
    run_state: RunState,
    task_counts: &TaskCounts,
    latest_task_activity_at: Option<String>,
    trace: &RunTraceDiagnostics,
) -> Option<MakoRunDiagnosticSummary> {
    let now = chrono::Utc::now();
    let last_activity_at = latest_activity_timestamp(
        Some(session.updated_at.to_rfc3339()),
        runtime.map(|state| state.updated_at.clone()),
        latest_task_activity_at,
        trace.latest_trace_at.clone(),
    );
    let stalled_for_secs = last_activity_at
        .as_deref()
        .and_then(parse_timestamp)
        .map(|timestamp| (now - timestamp).num_seconds().max(0) as u64);
    let pending_approvals = trace.pending_approvals.len();

    if let Some(runtime) = runtime {
        if runtime.status == MakoRuntimeStateStatus::Sleeping
            && runtime.sleep_reason.as_deref() == Some("scheduled")
        {
            if let Some(wake_at) = runtime.next_wake_at.as_deref().and_then(parse_timestamp) {
                let overdue_by = (now - wake_at).num_seconds();
                if overdue_by > OVERDUE_WAKE_GRACE_SECS {
                    return Some(MakoRunDiagnosticSummary {
                        kind: "overdue_wake".to_string(),
                        severity: "critical".to_string(),
                        summary: "Wake overdue".to_string(),
                        detail: format!(
                            "Scheduled wake slipped by {}.",
                            format_duration_seconds(overdue_by as u64)
                        ),
                        last_activity_at,
                        last_trace_at: trace.latest_trace_at.clone(),
                        stalled_for_secs,
                        overdue_by_secs: Some(overdue_by as u64),
                        failure_streak: trace.failure_streak,
                    });
                }
            }
        }
    }

    if run_state == RunState::Failed || run_summary_failed(&trace.latest_run_summary) {
        let severity = if trace.failure_streak > 1 {
            "critical"
        } else {
            "warning"
        };
        return Some(MakoRunDiagnosticSummary {
            kind: "failed".to_string(),
            severity: severity.to_string(),
            summary: if trace.failure_streak > 1 {
                "Repeated failures".to_string()
            } else {
                "Recent run failed".to_string()
            },
            detail: failure_detail(trace, runtime),
            last_activity_at,
            last_trace_at: trace.latest_trace_at.clone(),
            stalled_for_secs,
            overdue_by_secs: None,
            failure_streak: trace.failure_streak,
        });
    }

    if pending_approvals > 0 {
        return Some(MakoRunDiagnosticSummary {
            kind: "awaiting_approval".to_string(),
            severity: "warning".to_string(),
            summary: "Awaiting approval".to_string(),
            detail: format!(
                "{} pending tool approval{}.",
                pending_approvals,
                if pending_approvals == 1 { "" } else { "s" }
            ),
            last_activity_at,
            last_trace_at: trace.latest_trace_at.clone(),
            stalled_for_secs,
            overdue_by_secs: None,
            failure_streak: trace.failure_streak,
        });
    }

    if run_state == RunState::Waiting {
        if let Some(age_secs) = stalled_for_secs {
            if age_secs as i64 > WAITING_STALE_SECS {
                return Some(MakoRunDiagnosticSummary {
                    kind: "stale_waiting".to_string(),
                    severity: "warning".to_string(),
                    summary: "Blocked without movement".to_string(),
                    detail: format!(
                        "Run has been waiting for {} without a new trace or state update.",
                        format_duration_seconds(age_secs)
                    ),
                    last_activity_at,
                    last_trace_at: trace.latest_trace_at.clone(),
                    stalled_for_secs: Some(age_secs),
                    overdue_by_secs: None,
                    failure_streak: trace.failure_streak,
                });
            }
        }
        return Some(MakoRunDiagnosticSummary {
            kind: "awaiting_input".to_string(),
            severity: "warning".to_string(),
            summary: "Awaiting input".to_string(),
            detail: "Run is waiting for user input before it can continue.".to_string(),
            last_activity_at,
            last_trace_at: trace.latest_trace_at.clone(),
            stalled_for_secs,
            overdue_by_secs: None,
            failure_streak: trace.failure_streak,
        });
    }

    if let Some(age_secs) = stalled_for_secs {
        if run_state == RunState::Running && age_secs as i64 > ACTIVE_STALE_SECS {
            return Some(MakoRunDiagnosticSummary {
                kind: "stale_active".to_string(),
                severity: "warning".to_string(),
                summary: "No recent activity".to_string(),
                detail: format!(
                    "Run is marked awake but has been quiet for {}.",
                    format_duration_seconds(age_secs)
                ),
                last_activity_at,
                last_trace_at: trace.latest_trace_at.clone(),
                stalled_for_secs: Some(age_secs),
                overdue_by_secs: None,
                failure_streak: trace.failure_streak,
            });
        }

        let open_work = task_counts.pending + task_counts.in_progress + task_counts.blocked;
        if matches!(
            run_state,
            RunState::Idle | RunState::Sleeping | RunState::Paused
        ) && open_work > 0
            && age_secs as i64 > QUEUED_STALE_SECS
        {
            return Some(MakoRunDiagnosticSummary {
                kind: "stale_queued".to_string(),
                severity: "warning".to_string(),
                summary: "Queued without movement".to_string(),
                detail: format!(
                    "Open work has not moved for {}.",
                    format_duration_seconds(age_secs)
                ),
                last_activity_at,
                last_trace_at: trace.latest_trace_at.clone(),
                stalled_for_secs: Some(age_secs),
                overdue_by_secs: None,
                failure_streak: trace.failure_streak,
            });
        }
    }

    None
}

fn latest_activity_timestamp(
    session_updated_at: Option<String>,
    runtime_updated_at: Option<String>,
    task_updated_at: Option<String>,
    trace_updated_at: Option<String>,
) -> Option<String> {
    let mut latest: Option<String> = None;
    latest = later_timestamp(latest, session_updated_at);
    latest = later_timestamp(latest, runtime_updated_at);
    latest = later_timestamp(latest, task_updated_at);
    later_timestamp(latest, trace_updated_at)
}

fn later_timestamp(current: Option<String>, candidate: Option<String>) -> Option<String> {
    match (current, candidate) {
        (None, candidate) => candidate,
        (current, None) => current,
        (Some(current), Some(candidate)) => {
            if candidate > current {
                Some(candidate)
            } else {
                Some(current)
            }
        }
    }
}

pub(super) fn parse_timestamp(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date| date.with_timezone(&chrono::Utc))
}

fn failure_detail(trace: &RunTraceDiagnostics, runtime: Option<&MakoRuntimeState>) -> String {
    let categories = trace
        .latest_run_summary
        .failure_categories
        .iter()
        .take(2)
        .map(format_failure_category)
        .collect::<Vec<_>>();

    let mut parts = Vec::new();
    if let Some(last_error) = runtime.and_then(|state| state.last_error.as_deref()) {
        parts.push(last_error.to_string());
    }
    if !categories.is_empty() {
        parts.push(format!("Categories: {}", categories.join(", ")));
    }
    if trace.failure_streak > 1 {
        parts.push(format!("Failure streak: {} runs.", trace.failure_streak));
    }
    if parts.is_empty() {
        "Recent trace history indicates a failed run.".to_string()
    } else {
        parts.join(" ")
    }
}

fn format_failure_category(category: &TraceFailureCategory) -> &'static str {
    match category {
        TraceFailureCategory::AgentError => "agent error",
        TraceFailureCategory::ProviderError => "provider error",
        TraceFailureCategory::BudgetExhausted => "budget exhausted",
        TraceFailureCategory::LoopGuardTriggered => "loop guard",
        TraceFailureCategory::StreamIdleTimeout => "stream idle timeout",
        TraceFailureCategory::PinchFailed => "pinch failed",
        TraceFailureCategory::UserAbort => "user abort",
        TraceFailureCategory::ToolExecutionError => "tool error",
        TraceFailureCategory::ServerToolError => "server tool error",
        TraceFailureCategory::ToolDenied => "tool denied",
    }
}

fn format_duration_seconds(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    format!("{days}d")
}

fn is_stalled_diagnostic(kind: &str) -> bool {
    matches!(
        kind,
        "stale_active" | "stale_waiting" | "stale_queued" | "overdue_wake"
    )
}

fn diagnostic_needs_attention(kind: &str) -> bool {
    matches!(kind, "awaiting_approval" | "awaiting_input" | "failed")
}

fn run_has_open_work(run_state: RunState, task_counts: &TaskCounts) -> bool {
    if run_state != RunState::Idle {
        return true;
    }

    (task_counts.pending + task_counts.in_progress + task_counts.blocked) > 0
}

fn has_due_soon_wake(runtime: Option<&MakoRuntimeState>) -> bool {
    let Some(runtime) = runtime else {
        return false;
    };
    if runtime.status != MakoRuntimeStateStatus::Sleeping
        || runtime.sleep_reason.as_deref() != Some("scheduled")
    {
        return false;
    }
    let Some(next_wake_at) = runtime.next_wake_at.as_deref().and_then(parse_timestamp) else {
        return false;
    };

    let lead_secs = (next_wake_at - chrono::Utc::now()).num_seconds();
    lead_secs > 0 && lead_secs <= 60 * 60
}

fn summarize_health_state(
    stalled_count: usize,
    overdue_wake_count: usize,
    repeating_failure_count: usize,
    attention_run_count: usize,
    pending_approvals_count: usize,
) -> &'static str {
    if overdue_wake_count > 0 || repeating_failure_count > 0 {
        return "degraded";
    }
    if stalled_count > 0 || attention_run_count > 0 || pending_approvals_count > 0 {
        return "attention";
    }
    "healthy"
}

fn summarize_queue_pressure(
    attention_run_count: usize,
    pending_approvals_count: usize,
    open_run_count: usize,
    due_soon_wake_count: usize,
) -> &'static str {
    if attention_run_count > 0 || pending_approvals_count > 0 {
        return "attention";
    }
    if open_run_count >= 6 || due_soon_wake_count >= 2 {
        return "busy";
    }
    "calm"
}

fn summarize_knowledge_health(
    memory_store: &MemoryStore,
    report_store: &ReportStore,
    sessions: &[SessionInfo],
    user_id: Option<&str>,
) -> Result<MakoKnowledgeHealthSummary, AppError> {
    let mut scopes = BTreeMap::<Option<String>, Vec<&SessionInfo>>::new();
    for session in sessions {
        scopes
            .entry(session.project_dir.clone())
            .or_default()
            .push(session);
    }

    let mut missing_snapshot_count = 0usize;
    let mut stale_snapshot_count = 0usize;
    let mut latest_snapshot_at: Option<String> = None;

    for (scope, scoped_sessions) in &scopes {
        let latest_session_at = scoped_sessions
            .iter()
            .map(|session| session.updated_at.to_rfc3339())
            .max();
        let latest_report_at = report_store
            .list_reports_for_user(scope.as_deref(), user_id)?
            .first()
            .map(|report| report.created_at.clone());
        let latest_signal_at = later_timestamp(latest_session_at, latest_report_at);
        let snapshot = memory_store.find_by_title_in_exact_scope(
            CURRENT_SNAPSHOT_TITLE,
            scope.as_deref(),
            user_id,
        );

        match snapshot {
            Some(snapshot) => {
                latest_snapshot_at =
                    later_timestamp(latest_snapshot_at, Some(snapshot.updated_at.clone()));
                if latest_signal_at
                    .as_deref()
                    .zip(parse_timestamp(&snapshot.updated_at))
                    .and_then(|(signal, snapshot_at)| {
                        parse_timestamp(signal).map(|signal_at| signal_at > snapshot_at)
                    })
                    .unwrap_or(false)
                {
                    stale_snapshot_count += 1;
                }
            }
            None => missing_snapshot_count += 1,
        }
    }

    let scope_count = scopes.len();
    let healthy_scope_count = scope_count
        .saturating_sub(missing_snapshot_count)
        .saturating_sub(stale_snapshot_count);

    Ok(MakoKnowledgeHealthSummary {
        scope_count,
        healthy_scope_count,
        missing_snapshot_count,
        stale_snapshot_count,
        latest_snapshot_at,
    })
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
