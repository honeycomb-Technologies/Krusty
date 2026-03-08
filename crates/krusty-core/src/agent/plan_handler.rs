//! Plan and mode switch tool handlers.
//!
//! These are "virtual" tools that don't go through the regular tool registry.
//! Instead, the orchestrator intercepts them and handles them directly because
//! they mutate the loop's own state (work mode, plan).

use std::path::Path;

use crate::ai::types::AiToolCall;
use crate::plan::{PlanFile, PlanManager, TaskStatus};
use crate::storage::{Database, SessionManager, WorkMode};
use crate::tools::registry::ToolResult;

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

/// Handle plan task tool calls: `task_start`, `task_complete`, `add_subtask`, `set_dependency`.
pub fn handle_plan_task(call: &AiToolCall, session_id: &str, db_path: &Path) -> ToolResult {
    let plan_manager = match PlanManager::new(db_path.to_path_buf()) {
        Ok(manager) => manager,
        Err(e) => {
            return ToolResult {
                output: format!("Error: failed to initialize plan manager: {}", e),
                is_error: true,
            };
        }
    };

    let mut plan = match plan_manager.get_plan(session_id) {
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

/// Detect and parse a plan from AI response text.
pub struct DetectedPlan {
    pub title: String,
    pub tasks: Vec<DetectedPlanTask>,
    pub plan_file: PlanFile,
}

pub struct DetectedPlanTask {
    pub description: String,
    pub completed: bool,
}

pub fn try_detect_plan(text: &str) -> Option<DetectedPlan> {
    let plan_file = PlanFile::try_parse_from_response(text)?;
    let title = if plan_file.title.trim().is_empty() {
        "Implementation Plan".to_string()
    } else {
        plan_file.title.clone()
    };

    let tasks: Vec<DetectedPlanTask> = plan_file
        .phases
        .iter()
        .flat_map(|phase| phase.tasks.iter())
        .filter_map(|task| {
            let content = task.description.trim().to_string();
            if content.is_empty() {
                None
            } else {
                Some(DetectedPlanTask {
                    description: content,
                    completed: task.completed || task.status == TaskStatus::Completed,
                })
            }
        })
        .collect();

    if tasks.is_empty() {
        return None;
    }

    Some(DetectedPlan {
        title,
        tasks,
        plan_file,
    })
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
