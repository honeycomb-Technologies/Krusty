use mitsuro_core::storage::{AutonomousTask, HiveRuntimeState, HiveRuntimeStateStatus, TaskStatus};

use super::parse_timestamp;

#[derive(Debug, Default, Clone, Copy)]
pub(in super::super) struct TaskCounts {
    pub(in super::super) pending: usize,
    pub(in super::super) in_progress: usize,
    pub(in super::super) completed: usize,
    pub(in super::super) failed: usize,
    pub(in super::super) blocked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum RunState {
    Running,
    Scheduled,
    Sleeping,
    Paused,
    Waiting,
    Failed,
    Idle,
}

pub(in super::super) fn summarize_tasks(tasks: &[AutonomousTask]) -> TaskCounts {
    let completed_ids = tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Completed)
        .map(|task| task.id.as_str())
        .collect::<std::collections::HashSet<_>>();
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

pub(in super::super) fn latest_task_activity_at(tasks: &[AutonomousTask]) -> Option<String> {
    tasks
        .iter()
        .map(|task| task.updated_at.as_str())
        .max()
        .map(str::to_string)
}

pub(in super::super) fn classify_run_state(
    runtime: Option<&HiveRuntimeState>,
    agent_state: &str,
) -> RunState {
    match runtime {
        Some(runtime)
            if runtime.status == HiveRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled") =>
        {
            RunState::Scheduled
        }
        Some(runtime) => match runtime.status {
            HiveRuntimeStateStatus::Running => RunState::Running,
            HiveRuntimeStateStatus::Sleeping => RunState::Sleeping,
            HiveRuntimeStateStatus::Paused => RunState::Paused,
            HiveRuntimeStateStatus::AwaitingInput => RunState::Waiting,
            HiveRuntimeStateStatus::Error => RunState::Failed,
            HiveRuntimeStateStatus::Cancelled | HiveRuntimeStateStatus::Idle => match agent_state {
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

pub(in super::super) fn overall_home_status(
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

pub(in super::super) fn run_has_open_work(run_state: RunState, task_counts: &TaskCounts) -> bool {
    if run_state != RunState::Idle {
        return true;
    }

    (task_counts.pending + task_counts.in_progress + task_counts.blocked) > 0
}

pub(in super::super) fn has_due_soon_wake(runtime: Option<&HiveRuntimeState>) -> bool {
    let Some(runtime) = runtime else {
        return false;
    };
    if runtime.status != HiveRuntimeStateStatus::Sleeping
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

pub(in super::super) fn summarize_health_state(
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

pub(in super::super) fn summarize_queue_pressure(
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
