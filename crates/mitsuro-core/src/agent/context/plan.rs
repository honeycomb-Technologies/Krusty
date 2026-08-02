use std::path::Path;

use tracing::warn;

use crate::plan::PlanManager;
use crate::storage::WorkMode;
use crate::workflow::{GoalStatus, WorkflowManager, WorkflowSnapshot, WorkflowStepStatus};

use super::truncate_utf8;

const MAX_VISIBLE_READY_TASKS: usize = 12;
const MAX_VISIBLE_BLOCKED_TASKS: usize = 6;
const MAX_PLAN_TASK_DESCRIPTION_CHARS: usize = 300;

/// Build plan context from the active plan for this session.
pub fn build_plan_context(db_path: &Path, session_id: &str, work_mode: WorkMode) -> String {
    match WorkflowManager::new(db_path.to_path_buf())
        .and_then(|manager| manager.get_snapshot(session_id))
    {
        Ok(Some(snapshot)) => return build_workflow_context(&snapshot, work_mode),
        Ok(None) => {}
        Err(error) => {
            warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load canonical workflow context"
            );
        }
    }

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

fn build_workflow_context(snapshot: &WorkflowSnapshot, work_mode: WorkMode) -> String {
    let goal = &snapshot.goal;
    let constraints = if goal.constraints.is_empty() {
        "  (none)".to_string()
    } else {
        goal.constraints
            .iter()
            .map(|constraint| format!("  - {constraint}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let criteria = if snapshot.criteria.is_empty() {
        "  (none defined)".to_string()
    } else {
        snapshot
            .criteria
            .iter()
            .map(|criterion| {
                format!(
                    "  - [{}] {} (criterion_id: {})",
                    criterion.status, criterion.description, criterion.id
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let plan = snapshot
        .plan_revision
        .as_ref()
        .map(|plan| {
            format!(
                "{} (revision {}, {})",
                plan.title, plan.revision_number, plan.status
            )
        })
        .unwrap_or_else(|| "(no plan revision proposed)".to_string());
    let steps = if snapshot.steps.is_empty() {
        "  (none)".to_string()
    } else {
        snapshot
            .steps
            .iter()
            .map(|step| {
                format!(
                    "  - [{}] {}: {}",
                    step.status, step.display_key, step.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    if work_mode == WorkMode::Plan {
        return format!(
            "[PLAN MODE ACTIVE - CANONICAL WORKFLOW]\n\n\
             Goal ID: {goal_id}\n\
             Goal revision: {revision}\n\
             Goal status: {status}\n\
             Outcome: {objective}\n\n\
             Constraints:\n{constraints}\n\n\
             Verification:\n{criteria}\n\n\
             Current plan: {plan}\n\
             Steps:\n{steps}\n\n\
             Plan mode is read-only for project state. Investigate, ask questions, and call \
             `workflow_propose` with typed Goal/plan data to create or revise a draft. Assistant \
             prose is never authoritative. Only the user can approve a plan revision and activate \
             or resume the Goal; permission mode does not change that boundary.",
            goal_id = goal.id,
            revision = snapshot.aggregate_revision,
            status = goal.status,
            objective = goal.objective,
        );
    }

    let current_step = snapshot
        .steps
        .iter()
        .find(|step| step.status == WorkflowStepStatus::InProgress)
        .or_else(|| {
            snapshot.steps.iter().find(|step| {
                step.status == WorkflowStepStatus::Pending
                    && snapshot
                        .dependencies
                        .iter()
                        .filter(|dependency| dependency.step_id == step.id)
                        .all(|dependency| {
                            snapshot.steps.iter().any(|candidate| {
                                candidate.id == dependency.depends_on_step_id
                                    && matches!(
                                        candidate.status,
                                        WorkflowStepStatus::Completed | WorkflowStepStatus::Skipped
                                    )
                            })
                        })
            })
        });
    let execution_instruction = match goal.status {
        GoalStatus::Active => current_step.map_or_else(
            || {
                if snapshot.steps.iter().all(|step| step.status.is_terminal()) {
                    "All plan steps are terminal. Run the required verification, call \
                     `workflow_update` with `verify_criterion` and concrete evidence for each \
                     criterion, then call it with `complete_goal`. Do not claim completion in \
                     prose."
                        .to_string()
                } else {
                    "No executable step is currently available. Do not invent progress; report the \
                     dependency or verification state."
                        .to_string()
                }
            },
            |step| {
                if step.status == WorkflowStepStatus::InProgress {
                    format!(
                        "Continue only Step {}: {}. Complete it with `task_complete` and concrete \
                         evidence. Do not start another step.",
                        step.display_key, step.description
                    )
                } else {
                    format!(
                        "Next Step {}: {}. Call `task_start` once before implementation, then \
                         complete it with concrete evidence.",
                        step.display_key, step.description
                    )
                }
            },
        ),
        GoalStatus::Draft => {
            "This Goal is a draft. Do not implement it until the user approves the exact plan \
             revision and activates the Goal."
                .to_string()
        }
        GoalStatus::Paused => {
            "This Goal is paused. Do not continue implementation until the user resumes it."
                .to_string()
        }
        GoalStatus::Blocked => {
            "This Goal is blocked. Report the persisted blocker and wait for an explicit user \
             edit or resume."
                .to_string()
        }
        GoalStatus::Completed | GoalStatus::Cancelled => {
            "This Goal is terminal. Do not continue its plan.".to_string()
        }
    };

    format!(
        "[DURABLE GOAL - {title}]\n\n\
         Goal ID: {goal_id}\n\
         Revision: {revision}\n\
         Status: {status}\n\
         Outcome: {objective}\n\n\
         Constraints:\n{constraints}\n\n\
         Verification:\n{criteria}\n\n\
         Current plan: {plan}\n\
         Progress:\n{steps}\n\n\
         {execution_instruction}\n\n\
         Follow-up messages steer this Goal; they do not replace its outcome. Finishing an \
         assistant response or all plan steps does not complete the Goal. Goal completion requires \
         verification criteria to pass.",
        title = goal.title,
        goal_id = goal.id,
        revision = snapshot.aggregate_revision,
        status = goal.status,
        objective = goal.objective,
    )
}

fn plan_mode_default_context() -> String {
    "[PLAN MODE ACTIVE]\n\n\
     You are in PLAN MODE. The user wants a plan before implementing.\n\
     - You can READ files, search code, and explore the codebase\n\
     - You CANNOT write, edit, or create files\n\
     - Use the AskUserQuestion tool for clarifications\n\
     - Use `workflow_propose` to submit a typed draft Goal and plan\n\n\
     Define the desired outcome, constraints, measurable verification criteria, and ordered plan \
     steps. Assistant Markdown is presentation only and never creates execution state. The user \
     must explicitly approve the exact plan revision and activate the Goal before implementation."
        .to_string()
}
