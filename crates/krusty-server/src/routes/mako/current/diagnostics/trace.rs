use serde_json::Value;

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::storage::{
    MakoControllerEvent, MakoRunPriority, RuntimeTraceEvent, RuntimeTraceStore, RuntimeTraceSummary,
};

use crate::error::AppError;

use super::super::MakoPendingApprovalSummary;

#[derive(Debug, Default, Clone)]
pub(in super::super) struct RunTraceDiagnostics {
    pub(in super::super) latest_trace_at: Option<String>,
    pub(in super::super) latest_run_summary: RuntimeTraceSummary,
    pub(in super::super) failure_streak: usize,
    pub(in super::super) pending_approvals: Vec<MakoPendingApprovalSummary>,
}

pub(in super::super) fn load_run_trace_diagnostics(
    trace_store: &RuntimeTraceStore<'_>,
    session_id: &str,
    pending_approvals: Vec<MakoPendingApprovalSummary>,
) -> Result<RunTraceDiagnostics, AppError> {
    let events = trace_store.list_events(session_id, Some(200))?;
    let latest_trace_at = events.last().map(|event| event.created_at.clone());
    let latest_run_summary = summarize_latest_run_from_events(&events);
    let failure_streak = recent_failure_streak(&events);
    Ok(RunTraceDiagnostics {
        latest_trace_at,
        latest_run_summary,
        failure_streak,
        pending_approvals,
    })
}

pub(super) fn run_summary_failed(summary: &RuntimeTraceSummary) -> bool {
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

pub(in super::super) fn pending_approval_summaries_from_durable_events(
    events: &[MakoControllerEvent],
    session_id: &str,
    session_title: &str,
    project_dir: Option<&str>,
    target_branch: Option<&str>,
    priority: MakoRunPriority,
) -> Vec<MakoPendingApprovalSummary> {
    events
        .iter()
        .filter_map(|event| {
            let run_id = event.run_id.as_ref()?;
            let tool_call_id = event.payload.get("id").and_then(Value::as_str)?;
            let tool_name = event.payload.get("name").and_then(Value::as_str)?;
            Some(MakoPendingApprovalSummary {
                session_id: session_id.to_string(),
                run_id: run_id.clone(),
                session_title: session_title.to_string(),
                project_dir: project_dir.map(str::to_string),
                target_branch: target_branch.map(str::to_string),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: event
                    .payload
                    .get("arguments")
                    .cloned()
                    .unwrap_or(Value::Null),
                requested_at: event.created_at.clone(),
                priority,
            })
        })
        .collect()
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
