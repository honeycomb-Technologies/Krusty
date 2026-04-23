use std::path::Path;

use tracing::warn;

use crate::storage::{AutonomousTaskStore, DelegatedRunStore, TaskStatus};

use super::open_context_database;

pub(super) fn build_delegated_context(db_path: &Path, session_id: &str) -> String {
    let Some(db) = open_context_database(db_path, "building delegated context") else {
        return String::new();
    };
    let store = DelegatedRunStore::new(db);
    let recent = match store.list_runs_for_session(session_id, 3) {
        Ok(runs) => runs,
        Err(error) => {
            warn!(session_id = %session_id, error = %error, "Failed to load delegated runs for context");
            return String::new();
        }
    };
    if recent.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        "[RECENT DELEGATED RUNS]".to_string(),
        String::new(),
        "Recent delegated investigations exist for this session.".to_string(),
        "- If the user explicitly asks to use `explore` or continue/refine a prior architectural audit, prefer calling `explore` again over manual `glob`/`list`/`read` probing.".to_string(),
        "- If a matching delegated scope already exists below, let the `explore` tool resume or deepen that work instead of remapping the directory from scratch.".to_string(),
        "- Only fall back to manual probing if the user forbids delegation or the explore result was clearly unusable.".to_string(),
        String::new(),
        "Most recent delegated runs:".to_string(),
    ];

    for run in recent {
        let scopes = if run.target_scope.is_empty() {
            "(no scope)".to_string()
        } else {
            run.target_scope
                .iter()
                .map(|scope| scope.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let role = match run.role {
            crate::storage::DelegatedRunRole::Explore => "explore",
            crate::storage::DelegatedRunRole::Build => "build",
            crate::storage::DelegatedRunRole::Planner => "planner",
            crate::storage::DelegatedRunRole::Verifier => "verifier",
        };
        let review = run
            .human_review
            .as_deref()
            .unwrap_or("No finalized review was recorded.")
            .lines()
            .next()
            .unwrap_or("No finalized review was recorded.");
        lines.push(format!(
            "- {} run {} on [{}]: stage={:?}, resumable={}, semantic_review=\"{}\"",
            role, run.delegated_run_id, scopes, run.stage, run.resumable, review
        ));
    }

    lines.join("\n")
}

/// Build context for autonomous tasks in this session.
pub(super) fn build_autonomous_task_context(db_path: &Path, session_id: &str) -> String {
    let Some(db) = open_context_database(db_path, "building autonomous task context") else {
        return String::new();
    };
    let store = AutonomousTaskStore::new(db);
    let tasks = match store.list_tasks(session_id) {
        Ok(t) => t,
        Err(error) => {
            warn!(session_id = %session_id, error = %error, "Failed to load autonomous tasks for context");
            return String::new();
        }
    };
    if tasks.is_empty() {
        return String::new();
    }

    let mut lines = vec!["[AUTONOMOUS TASKS]".to_string()];

    let pending: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending)
        .collect();
    let in_progress: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::InProgress)
        .collect();
    let completed: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .collect();

    if !pending.is_empty() {
        lines.push("Pending:".to_string());
        for t in &pending {
            lines.push(format!("  - {}: {}", t.id, t.subject));
        }
    }
    if !in_progress.is_empty() {
        lines.push("In Progress:".to_string());
        for t in &in_progress {
            let owner = t
                .owner
                .as_deref()
                .map(|o| format!(" (owner: {})", o))
                .unwrap_or_default();
            lines.push(format!("  - {}: {}{}", t.id, t.subject, owner));
        }
    }
    if !completed.is_empty() {
        lines.push("Completed:".to_string());
        for t in &completed {
            lines.push(format!("  - {}: {}", t.id, t.subject));
        }
    }

    lines.join("\n")
}
