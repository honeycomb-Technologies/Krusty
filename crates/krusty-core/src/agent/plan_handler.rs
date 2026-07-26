//! Plan and mode switch tool handlers.
//!
//! These are "virtual" tools that don't go through the regular tool registry.
//! Instead, the orchestrator intercepts them and handles them directly because
//! they mutate the loop's own state (work mode, plan).

use std::path::Path;
use std::str::FromStr;

use serde::Deserialize;

use crate::ai::types::AiToolCall;
use crate::plan::{PlanFile, PlanManager, TaskStatus};
use crate::storage::{Database, SessionManager, WorkMode};
use crate::tools::registry::{PermissionMode, ToolResult};
use crate::workflow::{
    CompleteStepInput, CreateGoalInput, CriterionStatus, PlanProposalInput, SetCriterionInput,
    StartAttemptInput, WorkflowManager, WorkflowMutation, WorkflowSnapshot, WorkflowStepStatus,
    DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS, DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS,
    DEFAULT_GOAL_ATTEMPT_MAX_TURNS, DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS,
};

/// Result of a mode switch attempt.
pub struct ModeSwitchResult {
    pub tool_result: ToolResult,
    pub next_mode: WorkMode,
    pub mode_change_reason: Option<String>,
}

/// Handle `set_work_mode` or `enter_plan_mode` tool calls.
pub fn handle_mode_switch(
    call: &AiToolCall,
    session_id: &str,
    db_path: &Path,
    current_mode: WorkMode,
) -> ModeSwitchResult {
    let clear_existing = call
        .arguments
        .get("clear_existing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (target_mode, fallback_reason) = if call.name == "enter_plan_mode" {
        (WorkMode::Plan, "Starting planning phase")
    } else {
        let Some(mode) = call.arguments.get("mode").and_then(|v| v.as_str()) else {
            return ModeSwitchResult {
                tool_result: ToolResult {
                    output: "Error: mode parameter is required (build|plan)".to_string(),
                    is_error: true,
                },
                next_mode: current_mode,
                mode_change_reason: None,
            };
        };
        let parsed_mode = match mode {
            "build" => WorkMode::Build,
            "plan" => WorkMode::Plan,
            other => {
                return ModeSwitchResult {
                    tool_result: ToolResult {
                        output: format!("Error: invalid mode '{}'. Use 'build' or 'plan'.", other),
                        is_error: true,
                    },
                    next_mode: current_mode,
                    mode_change_reason: None,
                };
            }
        };
        let fallback_reason = if parsed_mode == WorkMode::Plan {
            "Starting planning phase"
        } else {
            "Starting implementation phase"
        };
        (parsed_mode, fallback_reason)
    };

    let reason = call
        .arguments
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_reason)
        .to_string();

    let mut clear_plan_note = String::new();
    if clear_existing && target_mode == WorkMode::Plan {
        match PlanManager::new(db_path.to_path_buf()) {
            Ok(plan_manager) => {
                if let Err(e) = plan_manager.abandon_plan(session_id) {
                    clear_plan_note = format!("\n\nWarning: failed to clear existing plan: {}", e);
                } else {
                    clear_plan_note = "\n\nCleared any existing active plan.".to_string();
                }
            }
            Err(e) => {
                clear_plan_note = format!("\n\nWarning: failed to initialize plan manager: {}", e);
            }
        }
    }

    let mut next_mode = current_mode;
    let mut mode_change_reason = None;
    if target_mode != current_mode {
        let session_manager = match Database::new(db_path) {
            Ok(db) => SessionManager::new(db),
            Err(e) => {
                return ModeSwitchResult {
                    tool_result: ToolResult {
                        output: format!("Error: failed to open database for mode switch: {}", e),
                        is_error: true,
                    },
                    next_mode: current_mode,
                    mode_change_reason: None,
                };
            }
        };
        if let Err(e) = session_manager.update_session_work_mode(session_id, target_mode) {
            return ModeSwitchResult {
                tool_result: ToolResult {
                    output: format!("Error: failed to switch work mode: {}", e),
                    is_error: true,
                },
                next_mode: current_mode,
                mode_change_reason: None,
            };
        }
        next_mode = target_mode;
        mode_change_reason = Some(reason.clone());
    }

    let output = if target_mode == WorkMode::Plan {
        format!(
            "Now in Plan mode. {}\n\nCreate a phase-based checkbox plan before making changes.{}",
            reason, clear_plan_note
        )
    } else {
        format!(
            "Now in Build mode. {}\n\nProceed with implementation and keep plan task status updated.{}",
            reason, clear_plan_note
        )
    };

    ModeSwitchResult {
        tool_result: ToolResult {
            output,
            is_error: false,
        },
        next_mode,
        mode_change_reason,
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowProposalArguments {
    #[serde(default)]
    goal: Option<CreateGoalInput>,
    #[serde(default)]
    goal_id: Option<String>,
    #[serde(default)]
    expected_revision: Option<u64>,
    plan: PlanProposalInput,
}

/// Handle the typed draft-only workflow proposal tool.
pub fn handle_workflow_proposal(call: &AiToolCall, session_id: &str, db_path: &Path) -> ToolResult {
    let arguments: WorkflowProposalArguments = match serde_json::from_value(call.arguments.clone())
    {
        Ok(arguments) => arguments,
        Err(error) => {
            return ToolResult::invalid_parameters(error);
        }
    };
    let manager = match WorkflowManager::new(db_path.to_path_buf()) {
        Ok(manager) => manager,
        Err(error) => return ToolResult::error(error),
    };
    let snapshot = match manager.get_snapshot(session_id) {
        Ok(snapshot) => snapshot,
        Err(error) => return ToolResult::error(error),
    };

    let (goal_id, expected_revision) = match snapshot {
        Some(snapshot) if snapshot.goal.status.is_unfinished() => {
            if arguments.goal.is_some() {
                return ToolResult::invalid_parameters(
                    "goal must be omitted when revising an existing unfinished Goal",
                );
            }
            if arguments.goal_id.as_deref() != Some(snapshot.goal.id.as_str()) {
                return ToolResult::invalid_parameters(format!(
                    "goal_id must match current Goal {}",
                    snapshot.goal.id
                ));
            }
            if arguments.expected_revision != Some(snapshot.aggregate_revision) {
                return ToolResult::invalid_parameters(format!(
                    "expected_revision must be {}",
                    snapshot.aggregate_revision
                ));
            }
            (snapshot.goal.id, snapshot.aggregate_revision)
        }
        _ => {
            let Some(goal) = arguments.goal else {
                return ToolResult::invalid_parameters(
                    "goal is required when the session has no unfinished Goal",
                );
            };
            let created = match manager.create_goal(
                session_id,
                goal,
                &format!("{}:goal", call.id),
                "agent",
            ) {
                Ok(created) => created,
                Err(error) => return ToolResult::error(error),
            };
            (
                created.snapshot.goal.id,
                created.snapshot.aggregate_revision,
            )
        }
    };

    match manager.propose_plan(
        session_id,
        &goal_id,
        expected_revision,
        arguments.plan,
        &format!("{}:plan", call.id),
        "agent",
    ) {
        Ok(mutation) => workflow_tool_result(&mutation, "plan_proposed"),
        Err(error) => ToolResult::error(error),
    }
}

/// Handle evidence-backed Goal verification and explicit completion.
pub fn handle_workflow_update(call: &AiToolCall, session_id: &str, db_path: &Path) -> ToolResult {
    let manager = match WorkflowManager::new(db_path.to_path_buf()) {
        Ok(manager) => manager,
        Err(error) => return ToolResult::error(error),
    };
    let snapshot = match manager.get_snapshot(session_id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return ToolResult::error("No durable Goal exists for this session"),
        Err(error) => return ToolResult::error(error),
    };
    let goal_id = call
        .arguments
        .get("goal_id")
        .and_then(|value| value.as_str());
    let expected_revision = call
        .arguments
        .get("expected_revision")
        .and_then(|value| value.as_u64());
    if goal_id != Some(snapshot.goal.id.as_str())
        || expected_revision != Some(snapshot.aggregate_revision)
    {
        return ToolResult::invalid_parameters(format!(
            "goal_id and expected_revision must match Goal {} revision {}",
            snapshot.goal.id, snapshot.aggregate_revision
        ));
    }
    match call
        .arguments
        .get("action")
        .and_then(|value| value.as_str())
    {
        Some("verify_criterion") => {
            let Some(criterion_id) = call
                .arguments
                .get("criterion_id")
                .and_then(|value| value.as_str())
            else {
                return ToolResult::invalid_parameters(
                    "criterion_id is required for verify_criterion",
                );
            };
            let status = match call
                .arguments
                .get("status")
                .and_then(|value| value.as_str())
                .and_then(|value| CriterionStatus::from_str(value).ok())
            {
                Some(CriterionStatus::Passed) => CriterionStatus::Passed,
                Some(CriterionStatus::Failed) => CriterionStatus::Failed,
                _ => {
                    return ToolResult::invalid_parameters(
                        "status must be passed or failed for agent verification",
                    );
                }
            };
            let evidence = call
                .arguments
                .get("evidence")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match manager.set_criterion(
                session_id,
                &snapshot.goal.id,
                criterion_id,
                snapshot.aggregate_revision,
                SetCriterionInput {
                    status,
                    evidence,
                    verifier: "agent".to_string(),
                },
                &format!("{}:criterion", call.id),
                "agent",
            ) {
                Ok(mutation) => workflow_tool_result(&mutation, "criterion_updated"),
                Err(error) => ToolResult::error(error),
            }
        }
        Some("complete_goal") => match manager.complete_goal(
            session_id,
            &snapshot.goal.id,
            snapshot.aggregate_revision,
            &format!("{}:complete", call.id),
            "agent",
        ) {
            Ok(mutation) => workflow_tool_result(&mutation, "goal_completed"),
            Err(error) => ToolResult::error(error),
        },
        Some(other) => {
            ToolResult::invalid_parameters(format!("unsupported workflow update action {other}"))
        }
        None => ToolResult::invalid_parameters("action is required"),
    }
}

/// Handle plan task tool calls. Workflow v2 is canonical when present; the
/// legacy Markdown plan remains a compatibility fallback during migration.
pub fn handle_plan_task(
    call: &AiToolCall,
    session_id: &str,
    db_path: &Path,
    permission_mode: PermissionMode,
) -> ToolResult {
    match WorkflowManager::new(db_path.to_path_buf()).and_then(|manager| {
        manager
            .get_snapshot(session_id)
            .map(|snapshot| (manager, snapshot))
    }) {
        Ok((manager, Some(snapshot))) if snapshot.goal.status.is_unfinished() => {
            return handle_workflow_plan_task(
                call,
                session_id,
                &manager,
                snapshot,
                permission_mode,
            );
        }
        Ok(_) => {}
        Err(error) => return ToolResult::error(error),
    }

    let plan_manager = match PlanManager::new(db_path.to_path_buf()) {
        Ok(manager) => manager,
        Err(e) => {
            return ToolResult {
                output: format!("Error: failed to initialize plan manager: {}", e),
                is_error: true,
            };
        }
    };

    let mut plan = match plan_manager.get_active_plan(session_id) {
        Ok(Some(plan)) => plan,
        Ok(None) => {
            return ToolResult {
                output: "Error: No active plan. Create a plan first.".to_string(),
                is_error: true,
            };
        }
        Err(e) => {
            return ToolResult {
                output: format!("Error: failed to load plan: {}", e),
                is_error: true,
            };
        }
    };

    match call.name.as_str() {
        "task_start" => handle_task_start(call, session_id, &plan_manager, &mut plan),
        "task_complete" => handle_task_complete(call, session_id, &plan_manager, &mut plan),
        "add_subtask" => handle_add_subtask(call, session_id, &plan_manager, &mut plan),
        "set_dependency" => handle_set_dependency(call, session_id, &plan_manager, &mut plan),
        _ => ToolResult {
            output: format!("Error: unsupported plan tool '{}'", call.name),
            is_error: true,
        },
    }
}

fn handle_workflow_plan_task(
    call: &AiToolCall,
    session_id: &str,
    manager: &WorkflowManager,
    snapshot: WorkflowSnapshot,
    permission_mode: PermissionMode,
) -> ToolResult {
    match call.name.as_str() {
        "task_start" => {
            let Some(task_id) = call.arguments.get("task_id").and_then(|value| value.as_str())
            else {
                return ToolResult::invalid_parameters("task_id is required");
            };
            let Some(step) = snapshot
                .steps
                .iter()
                .find(|step| step.id == task_id || step.display_key == task_id)
            else {
                return ToolResult::invalid_parameters(format!(
                    "step {task_id} is not part of the current plan revision"
                ));
            };
            if step.status == WorkflowStepStatus::InProgress {
                let output = serde_json::json!({
                    "message": format!("Step {} is already in progress", step.display_key),
                    "goal_id": snapshot.goal.id,
                    "step_id": step.id,
                    "revision": snapshot.aggregate_revision,
                });
                return ToolResult::success_data(output).with_changed(false);
            }
            let operation_id = format!("{}:start", call.id);
            let mutation = if let Some(attempt) = snapshot
                .latest_attempt
                .as_ref()
                .filter(|attempt| attempt.status == crate::workflow::AttemptStatus::Running)
            {
                manager.claim_step(
                    session_id,
                    &snapshot.goal.id,
                    &attempt.id,
                    &step.id,
                    snapshot.aggregate_revision,
                    &operation_id,
                    "agent",
                )
            } else {
                manager.start_attempt(
                    session_id,
                    &snapshot.goal.id,
                    snapshot.aggregate_revision,
                    StartAttemptInput {
                        step_id: Some(step.id.clone()),
                        permission_mode: permission_mode.as_str().to_string(),
                        max_turns: DEFAULT_GOAL_ATTEMPT_MAX_TURNS,
                        max_tool_calls: DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS,
                        max_wall_time_secs: DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS,
                        max_research_actions: DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS,
                    },
                    &operation_id,
                    "agent",
                )
            };
            match mutation {
                Ok(mutation) => workflow_tool_result(&mutation, "step_started"),
                Err(error) => ToolResult::error(error),
            }
        }
        "task_complete" => {
            let Some(task_id) = call.arguments.get("task_id").and_then(|value| value.as_str())
            else {
                return ToolResult::invalid_parameters("task_id is required");
            };
            let outcome = call
                .arguments
                .get("result")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim();
            if outcome.is_empty() {
                return ToolResult::invalid_parameters(
                    "result is required and must describe the concrete outcome",
                );
            }
            let evidence = call
                .arguments
                .get("evidence")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|items| !items.is_empty())
                .unwrap_or_else(|| vec![outcome.to_string()]);
            let Some(step) = snapshot
                .steps
                .iter()
                .find(|step| step.id == task_id || step.display_key == task_id)
            else {
                return ToolResult::invalid_parameters(format!(
                    "step {task_id} is not part of the current plan revision"
                ));
            };
            let Some(attempt_id) = step.claimed_attempt_id.as_deref() else {
                return ToolResult::error_with_code(
                    "step_not_claimed",
                    format!(
                        "Step {} is not in progress. Call task_start first.",
                        step.display_key
                    ),
                );
            };
            match manager.complete_step(
                session_id,
                &snapshot.goal.id,
                &step.id,
                snapshot.aggregate_revision,
                CompleteStepInput {
                    attempt_id: attempt_id.to_string(),
                    outcome: outcome.to_string(),
                    evidence,
                },
                &format!("{}:complete", call.id),
                "agent",
            ) {
                Ok(mutation) => workflow_tool_result(&mutation, "step_completed"),
                Err(error) => ToolResult::error(error),
            }
        }
        "add_subtask" | "set_dependency" => ToolResult::error_with_code(
            "immutable_plan_revision",
            "Approved plan revisions are immutable. Enter Plan mode and use workflow_propose to create a replacement revision.",
        ),
        other => ToolResult::error_with_code(
            "unsupported_workflow_tool",
            format!("Unsupported workflow task tool {other}"),
        ),
    }
}

fn workflow_tool_result(mutation: &WorkflowMutation, action: &str) -> ToolResult {
    let total_steps = mutation.snapshot.steps.len();
    let completed_steps = mutation
        .snapshot
        .steps
        .iter()
        .filter(|step| step.status == WorkflowStepStatus::Completed)
        .count();
    let in_progress_steps = mutation
        .snapshot
        .steps
        .iter()
        .filter(|step| step.status == WorkflowStepStatus::InProgress)
        .count();
    let blocked_steps = mutation
        .snapshot
        .steps
        .iter()
        .filter(|step| step.status == WorkflowStepStatus::Blocked)
        .count();
    let terminal_steps = mutation
        .snapshot
        .steps
        .iter()
        .filter(|step| step.status.is_terminal())
        .count();
    let required_criteria = mutation
        .snapshot
        .criteria
        .iter()
        .filter(|criterion| criterion.required)
        .count();
    let passed_required_criteria = mutation
        .snapshot
        .criteria
        .iter()
        .filter(|criterion| criterion.required && criterion.status == CriterionStatus::Passed)
        .count();
    let next_steps = mutation
        .snapshot
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step.status,
                WorkflowStepStatus::Pending
                    | WorkflowStepStatus::InProgress
                    | WorkflowStepStatus::Blocked
            )
        })
        .take(3)
        .map(|step| {
            serde_json::json!({
                "display_key": step.display_key,
                "description": step.description,
                "status": step.status,
            })
        })
        .collect::<Vec<_>>();

    ToolResult::success_data(serde_json::json!({
        "action": action,
        "changed": mutation.changed,
        "goal_id": mutation.snapshot.goal.id,
        "goal_status": mutation.snapshot.goal.status,
        "revision": mutation.snapshot.aggregate_revision,
        "plan_revision_id": mutation
            .snapshot
            .plan_revision
            .as_ref()
            .map(|plan| plan.id.as_str()),
        "step_progress": {
            "completed": completed_steps,
            "terminal": terminal_steps,
            "in_progress": in_progress_steps,
            "blocked": blocked_steps,
            "total": total_steps,
        },
        "criteria_progress": {
            "passed_required": passed_required_criteria,
            "required": required_criteria,
            "total": mutation.snapshot.criteria.len(),
        },
        "next_steps": next_steps,
    }))
    .with_changed(mutation.changed)
    .with_progress_change_key(format!(
        "workflow:{}:{}",
        mutation.snapshot.goal.id, mutation.snapshot.aggregate_revision
    ))
}

/// Parse a plan confirmation choice from user input.
pub fn parse_plan_confirm_choice(raw: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(choice) = value.get("choice").and_then(|v| v.as_str()) {
            let normalized = choice.trim().to_ascii_lowercase();
            if normalized == "execute" || normalized == "abandon" {
                return Some(normalized);
            }
        }
    }

    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.contains("execute") {
        Some("execute".to_string())
    } else if normalized.contains("abandon") {
        Some("abandon".to_string())
    } else {
        None
    }
}

// ── Private helpers ────────────────────────────────────────────────────

fn handle_task_start(
    call: &AiToolCall,
    session_id: &str,
    plan_manager: &PlanManager,
    plan: &mut PlanFile,
) -> ToolResult {
    let Some(task_id) = call.arguments.get("task_id").and_then(|v| v.as_str()) else {
        return ToolResult {
            output: "Error: task_id required".to_string(),
            is_error: true,
        };
    };

    match plan.start_task(task_id) {
        Ok(()) => {
            if let Err(e) = plan_manager.save_plan_for_session(session_id, plan) {
                return ToolResult {
                    output: format!("Error: failed to save plan: {}", e),
                    is_error: true,
                };
            }
            ToolResult {
                output: format!("Started task {}. Status: in_progress", task_id),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            output: format!("Error: {}", e),
            is_error: true,
        },
    }
}

fn handle_task_complete(
    call: &AiToolCall,
    session_id: &str,
    plan_manager: &PlanManager,
    plan: &mut PlanFile,
) -> ToolResult {
    let result_text = call
        .arguments
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if result_text.is_empty() {
        return ToolResult {
            output: "Error: 'result' parameter is required. Describe what you accomplished for this specific task.".to_string(),
            is_error: true,
        };
    }

    if call
        .arguments
        .get("task_ids")
        .and_then(|v| v.as_array())
        .is_some()
    {
        return ToolResult {
            output: "Error: Batch completion (task_ids) is not allowed. Complete ONE task at a time with task_id. This ensures focused, quality work.".to_string(),
            is_error: true,
        };
    }

    let Some(task_id) = call.arguments.get("task_id").and_then(|v| v.as_str()) else {
        return ToolResult {
            output: "Error: task_id required. Specify which task you're completing.".to_string(),
            is_error: true,
        };
    };

    let task_status = plan.find_task(task_id).map(|t| t.status);
    match task_status {
        None => {
            return ToolResult {
                output: format!("Error: Task '{}' not found in plan.", task_id),
                is_error: true,
            };
        }
        Some(TaskStatus::Completed) => {
            return ToolResult {
                output: format!("Error: Task '{}' is already completed.", task_id),
                is_error: true,
            };
        }
        Some(TaskStatus::Blocked) => {
            return ToolResult {
                output: format!(
                    "Error: Task '{}' is blocked. Complete its dependencies first, then use task_start.",
                    task_id
                ),
                is_error: true,
            };
        }
        Some(TaskStatus::Pending) => {
            return ToolResult {
                output: format!(
                    "Error: Task '{}' was not started. Use task_start(\"{}\") first, do the work, then complete it.",
                    task_id, task_id
                ),
                is_error: true,
            };
        }
        Some(TaskStatus::InProgress) => {}
    }

    if let Err(e) = plan.complete_task(task_id, &result_text) {
        return ToolResult {
            output: format!("Error: {}", e),
            is_error: true,
        };
    }
    if let Err(e) = plan_manager.save_plan_for_session(session_id, plan) {
        return ToolResult {
            output: format!("Error: failed to save plan: {}", e),
            is_error: true,
        };
    }

    let (completed, total) = plan.progress();
    let mut msg = format!(
        "Completed task {}. Progress: {}/{}",
        task_id, completed, total
    );
    if completed == total {
        msg.push_str("\n\nAll tasks complete. Plan finished.");
    } else {
        let ready = plan.get_ready_tasks();
        if !ready.is_empty() {
            msg.push_str("\n\nReady to work on next:");
            for task in &ready {
                msg.push_str(&format!("\n  → Task {}: {}", task.id, task.description));
            }
            msg.push_str("\n\nPick one and call task_start immediately.");
        } else {
            msg.push_str("\n\nNo tasks currently unblocked. Check dependencies.");
        }
    }

    ToolResult {
        output: msg,
        is_error: false,
    }
}

fn handle_add_subtask(
    call: &AiToolCall,
    session_id: &str,
    plan_manager: &PlanManager,
    plan: &mut PlanFile,
) -> ToolResult {
    let parent_id = call
        .arguments
        .get("parent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let description = call
        .arguments
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let context = call.arguments.get("context").and_then(|v| v.as_str());

    if parent_id.is_empty() || description.is_empty() {
        return ToolResult {
            output: "Error: parent_id and description required".to_string(),
            is_error: true,
        };
    }

    match plan.add_subtask(parent_id, description, context) {
        Ok(subtask_id) => {
            if let Err(e) = plan_manager.save_plan_for_session(session_id, plan) {
                return ToolResult {
                    output: format!("Error: failed to save plan: {}", e),
                    is_error: true,
                };
            }
            ToolResult {
                output: format!("Created subtask {} under {}", subtask_id, parent_id),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            output: format!("Error: {}", e),
            is_error: true,
        },
    }
}

fn handle_set_dependency(
    call: &AiToolCall,
    session_id: &str,
    plan_manager: &PlanManager,
    plan: &mut PlanFile,
) -> ToolResult {
    let task_id = call
        .arguments
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let blocked_by = call
        .arguments
        .get("blocked_by")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if task_id.is_empty() || blocked_by.is_empty() {
        return ToolResult {
            output: "Error: task_id and blocked_by required".to_string(),
            is_error: true,
        };
    }

    match plan.add_dependency(task_id, blocked_by) {
        Ok(()) => {
            if let Err(e) = plan_manager.save_plan_for_session(session_id, plan) {
                return ToolResult {
                    output: format!("Error: failed to save plan: {}", e),
                    is_error: true,
                };
            }
            ToolResult {
                output: format!("Task {} is now blocked by {}", task_id, blocked_by),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            output: format!("Error: {}", e),
            is_error: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{
        CollaborationMode, CriterionStatus, Goal, GoalCriterion, GoalStatus, WorkflowStep,
    };

    fn workflow_mutation_with_steps(step_count: usize) -> WorkflowMutation {
        let steps = (0..step_count)
            .map(|position| WorkflowStep {
                id: format!("step-{position}"),
                plan_revision_id: "plan-1".to_string(),
                parent_step_id: None,
                display_key: format!("{}", position + 1),
                position: position as u32,
                description: format!(
                    "A deliberately verbose workflow step description number {position}"
                ),
                context: Some("context that must not be echoed into tool history".repeat(20)),
                acceptance_criteria: vec!["criterion that must remain durable".repeat(10)],
                required: true,
                status: if position < 4 {
                    WorkflowStepStatus::Completed
                } else if position == 4 {
                    WorkflowStepStatus::InProgress
                } else {
                    WorkflowStepStatus::Pending
                },
                outcome: (position < 4).then(|| "completed outcome".repeat(10)),
                evidence: vec!["durable evidence".repeat(10)],
                claimed_attempt_id: None,
                revision: 1,
                created_at: "2026-07-26T00:00:00Z".to_string(),
                started_at: None,
                completed_at: None,
            })
            .collect();
        let criteria = vec![
            GoalCriterion {
                id: "criterion-1".to_string(),
                goal_id: "goal-1".to_string(),
                position: 0,
                description: "required criterion".to_string(),
                required: true,
                status: CriterionStatus::Passed,
                evidence: vec!["verified".to_string()],
                verifier: Some("agent".to_string()),
                verified_at: Some("2026-07-26T00:00:00Z".to_string()),
            },
            GoalCriterion {
                id: "criterion-2".to_string(),
                goal_id: "goal-1".to_string(),
                position: 1,
                description: "pending criterion".to_string(),
                required: true,
                status: CriterionStatus::Pending,
                evidence: Vec::new(),
                verifier: None,
                verified_at: None,
            },
        ];

        WorkflowMutation {
            changed: true,
            operation_id: "operation-1".to_string(),
            snapshot: WorkflowSnapshot {
                schema_version: 1,
                aggregate_revision: 7,
                collaboration_mode: CollaborationMode::Default,
                permission_mode: "autonomous".to_string(),
                goal: Goal {
                    id: "goal-1".to_string(),
                    session_id: "session-1".to_string(),
                    title: "Compact workflow results".to_string(),
                    objective: "Keep durable state out of model-facing tool history".to_string(),
                    constraints: Vec::new(),
                    status: GoalStatus::Active,
                    status_reason: None,
                    needs_definition: false,
                    revision: 7,
                    token_budget: Some(100_000),
                    tokens_used: 10_000,
                    source: "user".to_string(),
                    legacy_plan_id: None,
                    created_at: "2026-07-26T00:00:00Z".to_string(),
                    updated_at: "2026-07-26T00:00:00Z".to_string(),
                    activated_at: Some("2026-07-26T00:00:00Z".to_string()),
                    completed_at: None,
                    cancelled_at: None,
                },
                criteria,
                plan_revision: None,
                steps,
                dependencies: Vec::new(),
                latest_attempt: None,
                allowed_actions: vec!["pause_goal".to_string()],
            },
        }
    }

    #[test]
    fn workflow_tool_result_keeps_large_plans_out_of_model_history() {
        let result = workflow_tool_result(&workflow_mutation_with_steps(100), "step_completed");
        let envelope: serde_json::Value =
            serde_json::from_str(&result.output).expect("structured tool result");
        let data = &envelope["data"];

        assert!(data.get("steps").is_none());
        assert_eq!(data["step_progress"]["completed"], 4);
        assert_eq!(data["step_progress"]["in_progress"], 1);
        assert_eq!(data["step_progress"]["total"], 100);
        assert_eq!(data["criteria_progress"]["passed_required"], 1);
        assert_eq!(data["criteria_progress"]["required"], 2);
        assert_eq!(data["next_steps"].as_array().map(Vec::len), Some(3));
        assert!(
            result.output.len() < 2_500,
            "workflow tool result unexpectedly large: {} bytes",
            result.output.len()
        );
        assert!(!result.output.contains("context that must not be echoed"));
        assert!(!result.output.contains("durable evidence"));
    }
}
