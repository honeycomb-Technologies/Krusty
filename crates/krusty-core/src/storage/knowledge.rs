use anyhow::Result;
use std::path::Path;

use crate::agent::loop_events::LoopStopReason;

use super::{
    AgentMemory, AutonomousTask, AutonomousTaskStore, Database, MemoryStore, MemoryType, Report,
    ReportStore, RuntimeTraceStore, SessionManager, SessionType, TaskStatus,
};

pub const CURRENT_SNAPSHOT_TITLE: &str = "Current Snapshot";

const SNAPSHOT_MAX_MEMORY_ITEMS: usize = 6;
const SNAPSHOT_MAX_REPORT_ITEMS: usize = 4;
const SNAPSHOT_MAX_RUN_ITEMS: usize = 4;
const SNAPSHOT_MAX_TASK_OUTCOMES: usize = 6;
const SNAPSHOT_MAX_CONTENT_CHARS: usize = 180;

#[derive(Debug, Clone)]
struct SnapshotRunSummary {
    title: String,
    updated_at: String,
    pending_tasks: usize,
    in_progress_tasks: usize,
    completed_tasks: usize,
    failed_tasks: usize,
    blocked_tasks: usize,
    focus_subjects: Vec<String>,
    tool_calls: usize,
    awaiting_input_events: usize,
    provider_failures: usize,
    tool_errors: usize,
    last_stop_reason: Option<LoopStopReason>,
}

#[derive(Debug, Clone)]
struct SnapshotTaskOutcome {
    session_title: String,
    subject: String,
    status: TaskStatus,
    updated_at: String,
    result: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
struct SnapshotTaskCounts {
    pending: usize,
    in_progress: usize,
    completed: usize,
    failed: usize,
    blocked: usize,
}

pub fn is_current_snapshot(memory: &AgentMemory) -> bool {
    memory.memory_type == MemoryType::Project && is_current_snapshot_title(&memory.title)
}

pub fn is_current_snapshot_title(title: &str) -> bool {
    title == CURRENT_SNAPSHOT_TITLE
}

fn build_current_snapshot_content(
    memories: &[AgentMemory],
    reports: &[Report],
    recent_runs: &[SnapshotRunSummary],
    task_outcomes: &[SnapshotTaskOutcome],
    project_dir: Option<&str>,
) -> Option<String> {
    let carry_forward = memories
        .iter()
        .filter(|memory| !is_current_snapshot(memory))
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

pub fn refresh_current_snapshot(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<Option<AgentMemory>> {
    let memory_store = MemoryStore::new(Database::new(db_path)?);
    let report_store = ReportStore::new(Database::new(db_path)?);
    let memories = memory_store.list(project_dir, user_id);
    let reports = report_store.list_reports_for_user(project_dir, user_id)?;
    let (recent_runs, task_outcomes) = load_snapshot_activity(db_path, project_dir, user_id)?;

    let Some(content) = build_current_snapshot_content(
        &memories,
        &reports,
        &recent_runs,
        &task_outcomes,
        project_dir,
    ) else {
        if let Some(existing) =
            memory_store.find_by_title_in_exact_scope(CURRENT_SNAPSHOT_TITLE, project_dir, user_id)
        {
            memory_store.delete(&existing.id)?;
        }
        return Ok(None);
    };

    let (snapshot, _) = memory_store.save_or_update_by_title(
        MemoryType::Project,
        CURRENT_SNAPSHOT_TITLE,
        &content,
        project_dir,
        user_id,
    )?;

    Ok(Some(snapshot))
}

fn load_snapshot_activity(
    db_path: &Path,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> Result<(Vec<SnapshotRunSummary>, Vec<SnapshotTaskOutcome>)> {
    let session_manager = SessionManager::new(Database::new(db_path)?);
    let task_store = AutonomousTaskStore::new(Database::new(db_path)?);
    let trace_db = Database::new(db_path)?;
    let trace_store = RuntimeTraceStore::new(&trace_db);
    let sessions =
        session_manager.list_sessions_for_user_by_type(project_dir, user_id, SessionType::Mako)?;

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

fn format_memory_kind(memory_type: MemoryType) -> &'static str {
    match memory_type {
        MemoryType::User => "User",
        MemoryType::Feedback => "Feedback",
        MemoryType::Project => "Project",
        MemoryType::Reference => "Reference",
    }
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
        LoopStopReason::ContextCompactionFailed => "context compaction failed",
    }
}

fn truncate_utf8(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }

    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

pub use super::reports::promote_report_content;

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::TempDir;

    use super::*;

    fn create_db() -> (std::path::PathBuf, TempDir) {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("knowledge.db");
        let db = Database::new(&db_path).expect("db");
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["sess-1", "Knowledge Test", now, now],
            )
            .expect("seed session");
        (db_path, temp)
    }

    #[test]
    fn build_current_snapshot_content_excludes_existing_snapshot_memory() {
        let snapshot = AgentMemory {
            id: "snapshot".to_string(),
            memory_type: MemoryType::Project,
            title: CURRENT_SNAPSHOT_TITLE.to_string(),
            content: "old".to_string(),
            project_dir: Some("/repo".to_string()),
            user_id: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let durable = AgentMemory {
            id: "durable".to_string(),
            memory_type: MemoryType::Feedback,
            title: "Wake cadence".to_string(),
            content: "Favor faster cadence while the queue is active.".to_string(),
            project_dir: Some("/repo".to_string()),
            user_id: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
        };

        let content =
            build_current_snapshot_content(&[snapshot, durable], &[], &[], &[], Some("/repo"))
                .unwrap();

        assert!(content.contains("Wake cadence"));
        assert!(!content.contains("old"));
    }

    #[test]
    fn refresh_current_snapshot_creates_and_updates_snapshot_memory() {
        let (db_path, _temp) = create_db();
        let memory_store = MemoryStore::new(Database::new(&db_path).expect("db"));
        let report_store = ReportStore::new(Database::new(&db_path).expect("db"));

        memory_store
            .save(
                MemoryType::Project,
                "Auth decision",
                "Keep wake state canonical in runtime state.",
                Some("/repo"),
                None,
            )
            .expect("seed memory");
        report_store
            .create_report(super::super::reports::CreateReportInput {
                title: "Wake audit",
                session_id: "sess-1",
                project_dir: Some("/repo"),
                report_root: None,
                content: "# Wake\nStable.",
                summary: "Wake is stable.",
                tags: &[],
                sources: &[],
            })
            .expect("seed report");

        let snapshot = refresh_current_snapshot(&db_path, Some("/repo"), None)
            .expect("refresh snapshot")
            .expect("snapshot");
        assert_eq!(snapshot.title, CURRENT_SNAPSHOT_TITLE);
        assert!(snapshot.content.contains("Auth decision"));
        assert!(snapshot.content.contains("Wake audit"));

        let all_memories = memory_store.list(Some("/repo"), None);
        assert_eq!(
            all_memories
                .iter()
                .filter(|memory| is_current_snapshot(memory))
                .count(),
            1
        );
    }

    #[test]
    fn build_current_snapshot_content_includes_run_and_task_activity() {
        let content = build_current_snapshot_content(
            &[],
            &[],
            &[SnapshotRunSummary {
                title: "Wake audit".to_string(),
                updated_at: "2026-04-07T00:00:00Z".to_string(),
                pending_tasks: 1,
                in_progress_tasks: 1,
                completed_tasks: 2,
                failed_tasks: 0,
                blocked_tasks: 1,
                focus_subjects: vec!["Lock runtime cadence".to_string()],
                tool_calls: 4,
                awaiting_input_events: 0,
                provider_failures: 0,
                tool_errors: 0,
                last_stop_reason: Some(LoopStopReason::Completed),
            }],
            &[SnapshotTaskOutcome {
                session_title: "Wake audit".to_string(),
                subject: "Lock runtime cadence".to_string(),
                status: TaskStatus::Completed,
                updated_at: "2026-04-07T00:10:00Z".to_string(),
                result: Some("Cadence is now configurable per workspace.".to_string()),
            }],
            Some("/repo"),
        )
        .expect("snapshot content");

        assert!(content.contains("Recent Mako runs: 1"));
        assert!(content.contains("## Active Work"));
        assert!(content.contains("Lock runtime cadence"));
    }
}
