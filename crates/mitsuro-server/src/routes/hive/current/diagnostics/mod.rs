use mitsuro_core::storage::{
    HiveRuntimeState, HiveRuntimeStateStatus, SessionInfo, TraceFailureCategory,
};

use self::trace::run_summary_failed;
use super::HiveRunDiagnosticSummary;

mod state;
mod time;
mod trace;

pub(super) use self::state::{
    classify_run_state, has_due_soon_wake, latest_task_activity_at, overall_home_status,
    run_has_open_work, summarize_health_state, summarize_queue_pressure, summarize_tasks, RunState,
    TaskCounts,
};
pub(crate) use self::time::parse_timestamp;
pub(super) use self::time::{earlier_timestamp, later_timestamp};
pub(super) use self::trace::{
    load_run_trace_diagnostics, pending_approval_summaries_from_durable_events, RunTraceDiagnostics,
};

const ACTIVE_STALE_SECS: i64 = 30 * 60;
const WAITING_STALE_SECS: i64 = 15 * 60;
const QUEUED_STALE_SECS: i64 = 60 * 60;
const OVERDUE_WAKE_GRACE_SECS: i64 = 5 * 60;

pub(super) fn build_run_diagnostic(
    session: &SessionInfo,
    runtime: Option<&HiveRuntimeState>,
    run_state: RunState,
    task_counts: &TaskCounts,
    latest_task_activity_at: Option<String>,
    trace: &RunTraceDiagnostics,
) -> Option<HiveRunDiagnosticSummary> {
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
        if runtime.status == HiveRuntimeStateStatus::Sleeping
            && runtime.sleep_reason.as_deref() == Some("scheduled")
        {
            if let Some(wake_at) = runtime.next_wake_at.as_deref().and_then(parse_timestamp) {
                let overdue_by = (now - wake_at).num_seconds();
                if overdue_by > OVERDUE_WAKE_GRACE_SECS {
                    return Some(HiveRunDiagnosticSummary {
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
        if trace
            .latest_run_summary
            .failure_categories
            .iter()
            .any(|category| matches!(category, TraceFailureCategory::StreamIdleTimeout))
        {
            return Some(HiveRunDiagnosticSummary {
                kind: "stalled_stream".to_string(),
                severity: "critical".to_string(),
                summary: "Provider stream stalled".to_string(),
                detail: failure_detail(trace, runtime),
                last_activity_at,
                last_trace_at: trace.latest_trace_at.clone(),
                stalled_for_secs,
                overdue_by_secs: None,
                failure_streak: trace.failure_streak,
            });
        }

        let severity = if trace.failure_streak > 1 {
            "critical"
        } else {
            "warning"
        };
        return Some(HiveRunDiagnosticSummary {
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
        return Some(HiveRunDiagnosticSummary {
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
                return Some(HiveRunDiagnosticSummary {
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
        return Some(HiveRunDiagnosticSummary {
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
            return Some(HiveRunDiagnosticSummary {
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
            return Some(HiveRunDiagnosticSummary {
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

fn failure_detail(trace: &RunTraceDiagnostics, runtime: Option<&HiveRuntimeState>) -> String {
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

pub(super) fn is_stalled_diagnostic(kind: &str) -> bool {
    matches!(
        kind,
        "stalled_stream" | "stale_active" | "stale_waiting" | "stale_queued" | "overdue_wake"
    )
}

pub(super) fn diagnostic_needs_attention(kind: &str) -> bool {
    matches!(kind, "awaiting_approval" | "awaiting_input" | "failed")
}
