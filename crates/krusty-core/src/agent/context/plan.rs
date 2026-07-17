use std::path::Path;

use tracing::warn;

use crate::plan::PlanManager;
use crate::storage::WorkMode;

use super::truncate_utf8;

const MAX_VISIBLE_READY_TASKS: usize = 12;
const MAX_VISIBLE_BLOCKED_TASKS: usize = 6;
const MAX_PLAN_TASK_DESCRIPTION_CHARS: usize = 300;

/// Build plan context from the active plan for this session.
pub fn build_plan_context(db_path: &Path, session_id: &str, work_mode: WorkMode) -> String {
    let plan_manager = match PlanManager::new(db_path.to_path_buf()) {
        Ok(pm) => pm,
        Err(error) => {
            warn!(session_id = %session_id, db_path = %db_path.display(), error = %error, "Failed to open plan manager for context");
            return if work_mode == WorkMode::Plan {
                plan_mode_default_context()
            } else {
                String::new()
            };
        }
    };

    let plan = match plan_manager.get_active_plan(session_id) {
        Ok(Some(p)) => p,
        Err(error) => {
            warn!(session_id = %session_id, error = %error, "Failed to load active plan for context");
            return if work_mode == WorkMode::Plan {
                plan_mode_default_context()
            } else {
                String::new()
            };
        }
        _ => {
            return if work_mode == WorkMode::Plan {
                plan_mode_default_context()
            } else {
                String::new()
            };
        }
    };

    let (completed, total) = plan.progress();

    if work_mode == WorkMode::Plan {
        let markdown = plan.to_context();
        let title = plan.title.replace(['`', '"'], "'");
        format!(
            "[PLAN MODE ACTIVE - Plan: \"{}\"]\n\n\
             Progress: {}/{} tasks completed\n\n\
             {}\n\n\
             Plan mode is read-only: inspect and refine the plan, but do not modify project files.",
            title, completed, total, markdown
        )
    } else {
        let ready_tasks = plan.get_ready_tasks();
        let blocked_tasks = plan.get_blocked_tasks();

        let ready_list = if ready_tasks.is_empty() {
            "  (none)".to_string()
        } else {
            ready_tasks
                .iter()
                .take(MAX_VISIBLE_READY_TASKS)
                .map(|t| {
                    format!(
                        "  - Task {}: {}",
                        t.id,
                        truncate_utf8(&t.description, MAX_PLAN_TASK_DESCRIPTION_CHARS)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let blocked_list = if blocked_tasks.is_empty() {
            "  (none)".to_string()
        } else {
            blocked_tasks
                .iter()
                .take(MAX_VISIBLE_BLOCKED_TASKS)
                .map(|t| {
                    format!(
                        "  - Task {}: {} (waiting on: {})",
                        t.id,
                        truncate_utf8(&t.description, MAX_PLAN_TASK_DESCRIPTION_CHARS),
                        t.blocked_by.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let ready_omitted = ready_tasks.len().saturating_sub(MAX_VISIBLE_READY_TASKS);
        let blocked_omitted = blocked_tasks
            .len()
            .saturating_sub(MAX_VISIBLE_BLOCKED_TASKS);
        let omitted = match (ready_omitted, blocked_omitted) {
            (0, 0) => String::new(),
            (ready, blocked) => format!(
                "\n\nAdditional tasks omitted from prompt: {ready} ready/active, {blocked} blocked.",
            ),
        };

        let title = plan.title.replace(['`', '"'], "'");
        format!(
            "[ACTIVE PLAN - \"{}\"]\n\n\
             Progress: {}/{} tasks completed\n\n\
             Ready or active:\n{}\n\n\
             Blocked:\n{}{}\n\n\
             Work one ready task at a time. Call `task_start` before work and `task_complete` with a concrete result afterward.",
            title, completed, total, ready_list, blocked_list, omitted
        )
    }
}

fn plan_mode_default_context() -> String {
    "[PLAN MODE ACTIVE]\n\n\
     You are in PLAN MODE. The user wants a plan before implementing.\n\
     - You can READ files, search code, and explore the codebase\n\
     - You CANNOT write, edit, or create files\n\
     - Use the AskUserQuestion tool for clarifications\n\n\
     When creating a plan, use this format:\n\
     ```\n\
     # Plan: [Title]\n\n\
     ## Phase 1: [Phase Name]\n\n\
     - [ ] Task description\n\
       > Context: Implementation details\n\
     ```"
    .to_string()
}
