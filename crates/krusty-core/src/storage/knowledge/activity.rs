use anyhow::Result;
use std::path::Path;

use crate::agent::loop_events::LoopStopReason;
use crate::storage::{
    AutonomousTask, AutonomousTaskStore, Database, RuntimeTraceStore, SessionManager, SessionType,
    TaskStatus,
};

#[derive(Debug, Clone)]
pub(super) struct SnapshotRunSummary {
    pub(super) title: String,
    pub(super) updated_at: String,
    pub(super) pending_tasks: usize,
    pub(super) in_progress_tasks: usize,
    pub(super) completed_tasks: usize,
    pub(super) failed_tasks: usize,
    pub(super) blocked_tasks: usize,
    pub(super) focus_subjects: Vec<String>,
    pub(super) tool_calls: usize,
    pub(super) awaiting_input_events: usize,
    pub(super) provider_failures: usize,
    pub(super) tool_errors: usize,
    pub(super) last_stop_reason: Option<LoopStopReason>,
}

#[derive(Debug, Clone)]
pub(super) struct SnapshotTaskOutcome {
    pub(super) session_title: String,
    pub(super) subject: String,
    pub(super) status: TaskStatus,
    pub(super) updated_at: String,
    pub(super) result: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SnapshotTaskCounts {
    pending: usize,
    in_progress: usize,
    completed: usize,
    failed: usize,
    blocked: usize,
}

pub(super) fn load_snapshot_activity(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<(Vec<SnapshotRunSummary>, Vec<SnapshotTaskOutcome>)> {
    let session_manager = SessionManager::new(Database::new(db_path)?);
    let task_store = AutonomousTaskStore::new(Database::new(db_path)?);
    let trace_db = Database::new(db_path)?;
    let trace_store = RuntimeTraceStore::new(&trace_db);
    // The legacy session listing API treats `None` as an administrator-style
    // wildcard. Snapshot generation is a prompt boundary, so narrow the result
    // back to the exact owner (including local `None`) and exact project.
    let sessions = session_manager
        .list_sessions_for_user_by_type(None, user_id, SessionType::Mako)?
        .into_iter()
        .filter(|session| session.user_id.as_deref() == user_id)
        .filter(|session| project_dir.is_none() || session.project_dir.as_deref() == project_dir)
        .collect::<Vec<_>>();

    let mut recent_runs = Vec::new();
    let mut task_outcomes = Vec::new();

    for session in sessions {
        let tasks = task_store.list_tasks(&session.id)?;
        let counts = summarize_task_counts(&tasks);
        let trace_summary = trace_store.summarize_latest_run(&session.id)?;

        if counts.pending + counts.in_progress + counts.completed + counts.failed > 0
            || trace_summary.total_events > 0
        {
            let focus_subjects = tasks
                .iter()
                .filter(|task| matches!(task.status, TaskStatus::Pending | TaskStatus::InProgress))
                .map(|task| task.subject.clone())
                .take(2)
                .collect();
            recent_runs.push(SnapshotRunSummary {
                title: session.title.clone(),
                updated_at: session.updated_at.to_rfc3339(),
                pending_tasks: counts.pending,
                in_progress_tasks: counts.in_progress,
                completed_tasks: counts.completed,
                failed_tasks: counts.failed,
                blocked_tasks: counts.blocked,
                focus_subjects,
                tool_calls: trace_summary.tool_calls,
                awaiting_input_events: trace_summary.awaiting_input_events,
                provider_failures: trace_summary.provider_failures,
                tool_errors: trace_summary.tool_errors + trace_summary.server_tool_errors,
                last_stop_reason: trace_summary.last_stop_reason,
            });
        }

        task_outcomes.extend(tasks.into_iter().filter_map(|task| match task.status {
            TaskStatus::Completed | TaskStatus::Failed => Some(SnapshotTaskOutcome {
                session_title: session.title.clone(),
                subject: task.subject,
                status: task.status,
                updated_at: task.updated_at,
                result: task.result,
            }),
            TaskStatus::Pending | TaskStatus::InProgress => None,
        }));
    }

    recent_runs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    task_outcomes.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

    Ok((recent_runs, task_outcomes))
}

fn summarize_task_counts(tasks: &[AutonomousTask]) -> SnapshotTaskCounts {
    let completed_ids: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Completed)
        .map(|task| task.id.as_str())
        .collect();

    let mut counts = SnapshotTaskCounts::default();
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
