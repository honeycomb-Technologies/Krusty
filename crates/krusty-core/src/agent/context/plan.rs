use std::path::Path;

use tracing::warn;

use crate::plan::PlanManager;
use crate::storage::WorkMode;

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
    let markdown = plan.to_context();

    if work_mode == WorkMode::Plan {
        let title = plan.title.replace(['`', '"'], "'");
        format!(
            "[PLAN MODE ACTIVE - Plan: \"{}\"]\n\n\
             Progress: {}/{} tasks completed\n\n\
             ## Current Plan\n\n{}\n\n---\n\n\
             In plan mode you can READ but CANNOT write/edit files.",
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
                .map(|t| format!("  - Task {}: {}", t.id, t.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let blocked_list = if blocked_tasks.is_empty() {
            "  (none)".to_string()
        } else {
            blocked_tasks
                .iter()
                .map(|t| {
                    format!(
                        "  - Task {}: {} (waiting on: {})",
                        t.id,
                        t.description,
                        t.blocked_by.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let title = plan.title.replace(['`', '"'], "'");
        format!(
            "[ACTIVE PLAN - \"{}\"]\n\n\
             Progress: {}/{} tasks completed\n\n\
             ## Ready to Work\n{}\n\n\
             ## Blocked Tasks\n{}\n\n\
             ## Current Plan\n\n{}\n\n---\n\n\
             ## Task Workflow Protocol\n\n\
             1. PICK ONE ready task\n\
             2. `task_start(task_id)` - marks as in-progress\n\
             3. DO THE WORK\n\
             4. `task_complete(task_id, result)` - with specific result\n\
             5. Move to next task\n\n\
             Rules: One task at a time. Always start before completing. \
             Use `add_subtask` for complex tasks. Check Ready list for unblocked tasks.",
            title, completed, total, ready_list, blocked_list, markdown
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
