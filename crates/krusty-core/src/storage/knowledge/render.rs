use crate::agent::loop_events::LoopStopReason;
use crate::storage::{AgentMemory, MemoryType, Report, TaskStatus};

use super::activity::{SnapshotRunSummary, SnapshotTaskOutcome};
use super::promote_report_content;

const SNAPSHOT_MAX_MEMORY_ITEMS: usize = 6;
const SNAPSHOT_MAX_REPORT_ITEMS: usize = 4;
const SNAPSHOT_MAX_RUN_ITEMS: usize = 4;
const SNAPSHOT_MAX_TASK_OUTCOMES: usize = 6;
const SNAPSHOT_MAX_CONTENT_CHARS: usize = 180;

pub(super) fn build_current_snapshot_content(
    memories: &[AgentMemory],
    reports: &[Report],
    recent_runs: &[SnapshotRunSummary],
    task_outcomes: &[SnapshotTaskOutcome],
    project_dir: Option<&str>,
) -> Option<String> {
    let carry_forward = memories
        .iter()
        .filter(|memory| {
            !(memory.memory_type == MemoryType::Project
                && memory.title == super::CURRENT_SNAPSHOT_TITLE)
        })
        .collect::<Vec<_>>();

    if carry_forward.is_empty()
        && reports.is_empty()
        && recent_runs.is_empty()
        && task_outcomes.is_empty()
    {
        return None;
    }

    let open_tasks = recent_runs
        .iter()
        .map(|run| run.pending_tasks + run.in_progress_tasks)
        .sum::<usize>();
    let failed_tasks = recent_runs
        .iter()
        .map(|run| run.failed_tasks)
        .sum::<usize>();

    let mut sections = vec![
        "A compact summary of durable project knowledge, recent outcomes, and current Mako activity. Use this as orientation, then inspect the underlying memories, reports, or runs for detail.".to_string(),
    ];

    if let Some(project_dir) = project_dir {
        sections.push(format!("Scope: {}", project_dir));
    }

    sections.push(format!("Durable memories: {}", carry_forward.len()));
    sections.push(format!("Recent reports: {}", reports.len()));
    sections.push(format!("Recent Mako runs: {}", recent_runs.len()));
    sections.push(format!("Open tasks: {}", open_tasks));
    sections.push(format!("Failed tasks: {}", failed_tasks));

    if !carry_forward.is_empty() {
        sections.push("## Carry Forward".to_string());
        for memory in carry_forward.iter().take(SNAPSHOT_MAX_MEMORY_ITEMS) {
            sections.push(format!(
                "- [{}] {}: {}",
                format_memory_kind(memory.memory_type),
                memory.title,
                truncate_utf8(&memory.content, SNAPSHOT_MAX_CONTENT_CHARS)
            ));
        }
    }

    if !recent_runs.is_empty() {
        sections.push("## Active Work".to_string());
        for run in recent_runs.iter().take(SNAPSHOT_MAX_RUN_ITEMS) {
            sections.push(format_run_summary(run));
        }
    }

    if !task_outcomes.is_empty() {
        sections.push("## Recent Task Outcomes".to_string());
        for task in task_outcomes.iter().take(SNAPSHOT_MAX_TASK_OUTCOMES) {
            sections.push(format_task_outcome(task));
        }
    }

    if !reports.is_empty() {
        sections.push("## Recent Reports".to_string());
        for report in reports.iter().take(SNAPSHOT_MAX_REPORT_ITEMS) {
            sections.push(format!(
                "- \"{}\" ({}): {}",
                report.title,
                report.created_at,
                truncate_utf8(&promote_report_content(report), SNAPSHOT_MAX_CONTENT_CHARS)
            ));
        }
    }
    Some(sections.join("\n"))
}

fn format_memory_kind(memory_type: MemoryType) -> &'static str {
    match memory_type {
        MemoryType::User => "User",
        MemoryType::Feedback => "Feedback",
        MemoryType::Project => "Project",
        MemoryType::Reference => "Reference",
    }
}

fn format_run_summary(run: &SnapshotRunSummary) -> String {
    let mut details = Vec::new();

    if run.in_progress_tasks > 0 {
        details.push(format!("{} in progress", run.in_progress_tasks));
    }
    if run.pending_tasks > 0 {
        if run.blocked_tasks > 0 {
            details.push(format!(
                "{} pending ({} blocked)",
                run.pending_tasks, run.blocked_tasks
            ));
        } else {
            details.push(format!("{} pending", run.pending_tasks));
        }
    }
    if run.completed_tasks > 0 {
        details.push(format!("{} completed", run.completed_tasks));
    }
    if run.failed_tasks > 0 {
        details.push(format!("{} failed", run.failed_tasks));
    }
    if !run.focus_subjects.is_empty() {
        details.push(format!("focus {}", run.focus_subjects.join(", ")));
    }
    if let Some(stop_reason) = run.last_stop_reason.as_ref() {
        details.push(format!("latest stop {}", format_stop_reason(stop_reason)));
    }
    if run.awaiting_input_events > 0 {
        details.push(format!("{} awaited input", run.awaiting_input_events));
    }
    if run.provider_failures > 0 {
        details.push(format!("{} provider failures", run.provider_failures));
    }
    if run.tool_errors > 0 {
        details.push(format!("{} tool errors", run.tool_errors));
    }
    if run.tool_calls > 0 {
        details.push(format!("{} tool calls", run.tool_calls));
    }

    if details.is_empty() {
        details.push("no recent task or run signals".to_string());
    }

    format!(
        "- \"{}\" ({}): {}",
        run.title,
        run.updated_at,
        details.join(", ")
    )
}

fn format_task_outcome(task: &SnapshotTaskOutcome) -> String {
    let result = task
        .result
        .as_deref()
        .filter(|result| !result.trim().is_empty())
        .map(|result| truncate_utf8(result, SNAPSHOT_MAX_CONTENT_CHARS))
        .unwrap_or_else(|| match task.status {
            TaskStatus::Completed => "completed".to_string(),
            TaskStatus::Failed => "failed".to_string(),
            TaskStatus::Pending | TaskStatus::InProgress => "updated".to_string(),
        });

    format!(
        "- [{}] {} ({}, {}): {}",
        format_task_status(task.status),
        task.subject,
        task.session_title,
        task.updated_at,
        result
    )
}

fn format_task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
    }
}

fn format_stop_reason(reason: &LoopStopReason) -> &'static str {
    match reason {
        LoopStopReason::Completed => "completed",
        LoopStopReason::AwaitingInput => "awaiting input",
        LoopStopReason::Sleeping => "sleeping",
        LoopStopReason::BudgetExhausted => "budget exhausted",
        LoopStopReason::ProviderError => "provider error",
        LoopStopReason::LoopGuardTriggered => "loop guard",
        LoopStopReason::StreamIdleTimeout => "stream idle timeout",
        LoopStopReason::UserAbort => "user abort",
        LoopStopReason::Pinched => "pinched",
        LoopStopReason::PinchFailed => "pinch failed",
    }
}

pub(super) fn truncate_utf8(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }

    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}...", truncated)
}
