use std::collections::BTreeMap;

use serde_json::Value;

use krusty_core::agent::loop_events::LoopStopReason;
use krusty_core::storage::{
    MakoRunPriority, RuntimeTraceEvent, RuntimeTraceStore, RuntimeTraceSummary,
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
