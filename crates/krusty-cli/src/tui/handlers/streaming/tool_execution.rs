//! Tool execution and result handling
//!
//! Handles the execution of AI tool calls and processing of results.

use tokio::sync::{mpsc, oneshot};

use crate::agent::dual_mind::{DialogueResult, Observation, ObservedAction};
use crate::agent::subagent::AgentProgress;
use crate::ai::types::{AiToolCall, Content};
use crate::tools::{ToolContext, ToolOutputChunk};
use crate::tui::app::App;
use crate::tui::components::{PromptOption, PromptQuestion};
use crate::tui::utils::DualMindUpdate;

impl App {
    /// Handle enter_plan_mode tool calls to switch modes
    pub(super) fn handle_enter_plan_mode_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        use crate::tui::app::WorkMode;

        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling enter_plan_mode tool call: {}", tool_call.id);

            let reason = tool_call
                .arguments
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Starting planning phase")
                .to_string();

            let clear_existing = tool_call
                .arguments
                .get("clear_existing")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if clear_existing {
                self.clear_plan();
                tracing::info!("Cleared existing plan");
            }
            self.ui.work_mode = WorkMode::Plan;
            tracing::info!("Switched to Plan mode: {}", reason);

            results.push(Content::ToolResult {
                tool_use_id: tool_call.id.clone(),
                output: serde_json::Value::String(format!(
                    "Now in Plan mode. {}. Create a plan using the standard format (# Plan: Title, ## Phase N: Name, - [ ] Task). The user will review and approve before implementation.",
                    reason
                )),
                is_error: None,
            });
        }

        if !results.is_empty() {
            self.runtime.pending_tool_results.extend(results);
        }
    }

    /// Handle task_complete tool calls to update plan immediately
    /// ENFORCES: Task must be InProgress (started) before it can be completed
    /// ENFORCES: Only ONE task per call (no batch completion)
    /// ENFORCES: Result parameter required
    pub(super) fn handle_task_complete_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        use crate::plan::TaskStatus;
        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling task_complete tool call: {}", tool_call.id);

            // Extract required result parameter
            let result_text = tool_call
                .arguments
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if result_text.is_empty() {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: 'result' parameter is required. Describe what you accomplished for this specific task.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            }

            // HARD CONSTRAINT: Only single task_id allowed (no batch)
            let task_id = tool_call.arguments.get("task_id").and_then(|v| v.as_str());
            let task_ids = tool_call
                .arguments
                .get("task_ids")
                .and_then(|v| v.as_array());

            if task_ids.is_some() {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: Batch completion (task_ids) is not allowed. Complete ONE task at a time with task_id. This ensures focused, quality work.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            }

            let Some(task_id) = task_id.filter(|s| !s.is_empty()) else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: task_id required. Specify which task you're completing."
                            .to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            let Some(plan) = &mut self.runtime.active_plan else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: No active plan. Create a plan first.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            // HARD CONSTRAINT: Task must be InProgress to complete
            let task_status = plan.find_task(task_id).map(|t| t.status);
            match task_status {
                None => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Error: Task '{}' not found in plan.",
                            task_id
                        )),
                        is_error: Some(true),
                    });
                    continue;
                }
                Some(TaskStatus::Completed) => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Error: Task '{}' is already completed.",
                            task_id
                        )),
                        is_error: Some(true),
                    });
                    continue;
                }
                Some(TaskStatus::Blocked) => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Error: Task '{}' is blocked. Complete its dependencies first, then use task_start.",
                            task_id
                        )),
                        is_error: Some(true),
                    });
                    continue;
                }
                Some(TaskStatus::Pending) => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Error: Task '{}' was not started. Use task_start(\"{}\") first, do the work, then complete it.",
                            task_id, task_id
                        )),
                        is_error: Some(true),
                    });
                    continue;
                }
                Some(TaskStatus::InProgress) => {
                    // Good - task is in progress, can be completed
                }
            }

            // Complete the task
            if let Err(e) = plan.complete_task(task_id, &result_text) {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(format!("Error: {}", e)),
                    is_error: Some(true),
                });
                continue;
            }

            if let Err(e) = self.services.plan_manager.save_plan(plan) {
                tracing::error!("Failed to save plan after task completion: {}", e);
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

            tracing::info!("{}", msg);

            results.push(Content::ToolResult {
                tool_use_id: tool_call.id.clone(),
                output: serde_json::Value::String(msg),
                is_error: None,
            });
        }

        if !results.is_empty() {
            self.runtime.pending_tool_results.extend(results);
        }
    }

    /// Handle task_start tool calls to mark tasks as in-progress
    pub(super) fn handle_task_start_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling task_start tool call: {}", tool_call.id);

            let Some(task_id) = tool_call.arguments.get("task_id").and_then(|v| v.as_str()) else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String("Error: task_id required".to_string()),
                    is_error: Some(true),
                });
                continue;
            };

            let Some(plan) = &mut self.runtime.active_plan else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: No active plan. Create a plan first.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            match plan.start_task(task_id) {
                Ok(()) => {
                    if let Err(e) = self.services.plan_manager.save_plan(plan) {
                        tracing::error!("Failed to save plan after task start: {}", e);
                    }
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Started task {}. Status: in_progress",
                            task_id
                        )),
                        is_error: None,
                    });
                }
                Err(e) => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!("Error: {}", e)),
                        is_error: Some(true),
                    });
                }
            }
        }

        if !results.is_empty() {
            self.runtime.pending_tool_results.extend(results);
        }
    }

    /// Handle add_subtask tool calls to create subtasks
    pub(super) fn handle_add_subtask_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling add_subtask tool call: {}", tool_call.id);

            let parent_id = tool_call
                .arguments
                .get("parent_id")
                .and_then(|v| v.as_str());
            let description = tool_call
                .arguments
                .get("description")
                .and_then(|v| v.as_str());
            let context = tool_call.arguments.get("context").and_then(|v| v.as_str());

            let (Some(parent_id), Some(description)) = (parent_id, description) else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: parent_id and description required".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            let Some(plan) = &mut self.runtime.active_plan else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: No active plan. Create a plan first.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            match plan.add_subtask(parent_id, description, context) {
                Ok(subtask_id) => {
                    if let Err(e) = self.services.plan_manager.save_plan(plan) {
                        tracing::error!("Failed to save plan after adding subtask: {}", e);
                    }
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Created subtask {} under {}",
                            subtask_id, parent_id
                        )),
                        is_error: None,
                    });
                }
                Err(e) => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!("Error: {}", e)),
                        is_error: Some(true),
                    });
                }
            }
        }

        if !results.is_empty() {
            self.runtime.pending_tool_results.extend(results);
        }
    }

    /// Handle set_dependency tool calls to create task dependencies
    pub(super) fn handle_set_dependency_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling set_dependency tool call: {}", tool_call.id);

            let task_id = tool_call.arguments.get("task_id").and_then(|v| v.as_str());
            let blocked_by = tool_call
                .arguments
                .get("blocked_by")
                .and_then(|v| v.as_str());

            let (Some(task_id), Some(blocked_by)) = (task_id, blocked_by) else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: task_id and blocked_by required".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            let Some(plan) = &mut self.runtime.active_plan else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: No active plan. Create a plan first.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            match plan.add_dependency(task_id, blocked_by) {
                Ok(()) => {
                    if let Err(e) = self.services.plan_manager.save_plan(plan) {
                        tracing::error!("Failed to save plan after adding dependency: {}", e);
                    }
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Task {} is now blocked by {}",
                            task_id, blocked_by
                        )),
                        is_error: None,
                    });
                }
                Err(e) => {
                    results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!("Error: {}", e)),
                        is_error: Some(true),
                    });
                }
            }
        }

        if !results.is_empty() {
            self.runtime.pending_tool_results.extend(results);
        }
    }

    /// Handle AskUserQuestion tool calls via UI instead of registry
    pub(super) fn handle_ask_user_question_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        let Some(tool_call) = tool_calls.into_iter().next() else {
            return;
        };

        tracing::info!("Handling AskUserQuestion tool call: {}", tool_call.id);

        let questions_arg = tool_call.arguments.get("questions");
        let Some(questions_array) = questions_arg.and_then(|v| v.as_array()) else {
            tracing::warn!("AskUserQuestion missing questions array");
            return;
        };

        let mut prompt_questions: Vec<PromptQuestion> = Vec::new();

        for q in questions_array {
            let question = q.get("question").and_then(|v| v.as_str()).unwrap_or("");
            let header = q.get("header").and_then(|v| v.as_str()).unwrap_or("Q");
            let multi_select = q
                .get("multiSelect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let mut options: Vec<PromptOption> = Vec::new();
            if let Some(opts) = q.get("options").and_then(|v| v.as_array()) {
                for opt in opts {
                    let label = opt.get("label").and_then(|v| v.as_str()).unwrap_or("");
                    let description = opt.get("description").and_then(|v| v.as_str());
                    options.push(PromptOption {
                        label: label.to_string(),
                        description: description.map(|s| s.to_string()),
                    });
                }
            }

            // Add "Other" option for custom input
            options.push(PromptOption {
                label: "Other".to_string(),
                description: Some("Enter custom response".to_string()),
            });

            prompt_questions.push(PromptQuestion {
                question: question.to_string(),
                header: header.to_string(),
                options,
                multi_select,
            });
        }

        if prompt_questions.is_empty() {
            tracing::warn!("AskUserQuestion has no valid questions");
            return;
        }

        // Remove the "Preparing questions..." message
        if let Some((tag, _)) = self.runtime.chat.messages.last() {
            if tag == "tool" {
                self.runtime.chat.messages.pop();
            }
        }

        self.ui
            .decision_prompt
            .show_ask_user(prompt_questions, tool_call.id);
    }

    /// Spawn tool execution as a background task for non-blocking streaming
    pub fn spawn_tool_execution(&mut self, tool_calls: Vec<AiToolCall>) {
        let tool_names: Vec<_> = tool_calls.iter().map(|t| t.name.as_str()).collect();
        tracing::info!(
            "spawn_tool_execution: {} tools to execute: {:?}",
            tool_calls.len(),
            tool_names
        );

        // Track exploration budget: count consecutive read-only tool calls
        let all_readonly = tool_calls.iter().all(|t| {
            matches!(
                t.name.as_str(),
                "read" | "glob" | "grep" | "search_codebase"
            )
        });
        let has_action = tool_calls.iter().any(|t| {
            matches!(
                t.name.as_str(),
                "edit" | "write" | "bash" | "build" | "task_start" | "task_complete"
            )
        });
        if has_action {
            self.runtime.exploration_budget_count = 0;
        } else if all_readonly {
            self.runtime.exploration_budget_count += tool_calls.len();
        }

        if tool_calls.is_empty() {
            return;
        }

        // Intercept AskUserQuestion tool
        let (ask_user_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "AskUserQuestion");

        let has_ask_user = !ask_user_tools.is_empty();
        if has_ask_user {
            self.handle_ask_user_question_tools(ask_user_tools);
        }

        // Intercept task_complete tool
        let (task_complete_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "task_complete");

        let has_task_complete = !task_complete_tools.is_empty();
        if has_task_complete {
            self.handle_task_complete_tools(task_complete_tools);
        }

        // Intercept task_start tool
        let (task_start_tools, tool_calls): (Vec<_>, Vec<_>) =
            tool_calls.into_iter().partition(|t| t.name == "task_start");

        let has_task_start = !task_start_tools.is_empty();
        if has_task_start {
            self.handle_task_start_tools(task_start_tools);
        }

        // Intercept add_subtask tool
        let (add_subtask_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "add_subtask");

        let has_add_subtask = !add_subtask_tools.is_empty();
        if has_add_subtask {
            self.handle_add_subtask_tools(add_subtask_tools);
        }

        // Intercept set_dependency tool
        let (set_dependency_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "set_dependency");

        let has_set_dependency = !set_dependency_tools.is_empty();
        if has_set_dependency {
            self.handle_set_dependency_tools(set_dependency_tools);
        }

        // Intercept enter_plan_mode tool
        let (plan_mode_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "enter_plan_mode");

        let has_plan_mode = !plan_mode_tools.is_empty();
        if has_plan_mode {
            self.handle_enter_plan_mode_tools(plan_mode_tools);
        }

        let has_plan_tools = has_task_complete
            || has_task_start
            || has_add_subtask
            || has_set_dependency
            || has_plan_mode;

        if tool_calls.is_empty() {
            if has_ask_user {
                self.stop_streaming();
                return;
            }

            if has_plan_tools {
                let results = std::mem::take(&mut self.runtime.pending_tool_results);
                if !results.is_empty() {
                    self.stop_streaming();
                    self.handle_tool_results(results);
                }
                return;
            }
            return;
        }

        // Check if there's an explore/Task tool in the batch
        let has_explore = tool_calls
            .iter()
            .any(|t| t.name == "explore" || t.name == "Task");
        let has_build = tool_calls.iter().any(|t| t.name == "build");

        // If explore tool is present, queue non-explore tools for later
        let tools_to_execute = if has_explore {
            let (explore_tools, other_tools): (Vec<_>, Vec<_>) = tool_calls
                .into_iter()
                .partition(|t| t.name == "explore" || t.name == "Task");

            if !other_tools.is_empty() {
                tracing::info!(
                    "spawn_tool_execution: queuing {} tools until explore completes",
                    other_tools.len()
                );
                self.runtime.queued_tools.extend(other_tools);
            }

            explore_tools
        } else {
            tool_calls
        };

        if tools_to_execute.is_empty() {
            return;
        }

        // Create streaming output channel for bash
        let (output_tx, output_rx) = mpsc::unbounded_channel::<ToolOutputChunk>();
        self.runtime.channels.bash_output = Some(output_rx);

        // Create explore progress channel if any explore tools
        let explore_progress_tx = if has_explore {
            let (tx, rx) = mpsc::unbounded_channel::<AgentProgress>();
            self.runtime.channels.explore_progress = Some(rx);
            Some(tx)
        } else {
            None
        };

        // Create build progress channel if any build tools
        let build_progress_tx = if has_build {
            let (tx, rx) = mpsc::unbounded_channel::<AgentProgress>();
            self.runtime.channels.build_progress = Some(rx);
            Some(tx)
        } else {
            None
        };

        // Create dual-mind dialogue channel if dual-mind is active
        let dual_mind_tx = if self.runtime.dual_mind.is_some() {
            let (tx, rx) = mpsc::unbounded_channel::<DualMindUpdate>();
            self.runtime.channels.dual_mind = Some(rx);
            // DEBUG: Log that dual-mind is active for this execution
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/dual_mind_dialogue.log")
            {
                use std::io::Write;
                let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(
                    file,
                    "\n[{}] TOOL EXECUTION: dual_mind is ACTIVE",
                    timestamp
                );
            }
            Some(tx)
        } else {
            // DEBUG: Log that dual-mind is not active
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/dual_mind_dialogue.log")
            {
                use std::io::Write;
                let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(file, "\n[{}] TOOL EXECUTION: dual_mind is NONE", timestamp);
            }
            None
        };

        // Create result channel
        let (result_tx, result_rx) = oneshot::channel();
        self.runtime.channels.tool_results = Some(result_rx);

        self.start_tool_execution();

        // Create blocks for visual feedback
        self.create_tool_blocks(&tools_to_execute);

        // Clone what we need for the spawned task
        let tool_registry = self.services.tool_registry.clone();
        let process_registry = self.runtime.process_registry.clone();
        let skills_manager = self.services.skills_manager.clone();
        let cancel_token = self.runtime.cancellation.child_token();
        let plan_mode = self.ui.work_mode == crate::tui::app::WorkMode::Plan;
        let current_model = self.runtime.current_model.clone();
        let dual_mind = self.runtime.dual_mind.clone();
        let dual_mind_tx = dual_mind_tx;

        tokio::spawn(async move {
            let mut tool_results: Vec<Content> = Vec::new();

            for tool_call in tools_to_execute {
                if cancel_token.is_cancelled() {
                    tracing::info!("Tool execution cancelled before running {}", tool_call.name);
                    break;
                }

                let tool_name = tool_call.name.clone();

                // Pre-review: Little Claw questions the intent before execution
                // Only review mutating tools - read-only tools don't need quality review
                let is_mutating_tool = matches!(
                    tool_name.as_str(),
                    "edit" | "write" | "bash" | "build" | "Edit" | "Write" | "Bash"
                );

                if let (true, Some(dm)) = (is_mutating_tool, dual_mind.as_ref()) {
                    // Create concise intent summary (not full JSON dump)
                    let intent = create_intent_summary(&tool_name, &tool_call.arguments);

                    let review_result = {
                        let mut dm_guard = dm.write().await;
                        dm_guard.pre_review(&intent).await
                    };

                    // Only act on actual concerns - approvals are silent
                    if let DialogueResult::NeedsEnhancement { critique, .. } = review_result {
                        tracing::info!(
                            "Little Claw raised concern before {}: {}",
                            tool_name,
                            critique
                        );
                        // Send enhancement for potential UI display
                        if let Some(ref tx) = dual_mind_tx {
                            let _ = tx.send(DualMindUpdate {
                                enhancement: Some(critique),
                                review_output: None,
                            });
                        }
                    }
                }
                let working_dir =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

                let mut ctx =
                    ToolContext::with_process_registry(working_dir, process_registry.clone())
                        .with_skills_manager(skills_manager.clone())
                        .with_current_model(current_model.clone());
                ctx.plan_mode = plan_mode;

                if tool_name == "bash" {
                    ctx = ctx.with_output_stream(output_tx.clone(), tool_call.id.clone());
                }

                if tool_name == "explore" || tool_name == "Task" {
                    ctx.timeout = Some(std::time::Duration::from_secs(600));
                    if let Some(ref tx) = explore_progress_tx {
                        ctx = ctx.with_explore_progress(tx.clone());
                    }
                }

                if tool_name == "build" {
                    ctx.timeout = Some(std::time::Duration::from_secs(900));
                    if let Some(ref tx) = build_progress_tx {
                        ctx = ctx.with_build_progress(tx.clone());
                    }
                }

                let result = tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::info!("Tool execution cancelled during {}", tool_name);
                        Some(crate::tools::registry::ToolResult {
                            output: "Cancelled by user".to_string(),
                            is_error: true,
                        })
                    }
                    result = tool_registry.execute(&tool_call.name, tool_call.arguments.clone(), &ctx) => {
                        result
                    }
                };

                if let Some(result) = result {
                    // Sync observation to Little Claw (so it knows what happened)
                    if let Some(ref dm) = dual_mind {
                        let observation = create_observation(
                            &tool_name,
                            &tool_call.arguments,
                            &result.output,
                            !result.is_error,
                        );
                        let dm_guard = dm.read().await;
                        dm_guard.little_claw().observe(observation).await;
                    }

                    // Post-review: Little Claw validates the output
                    // Only reviews mutating tools with successful, non-trivial outputs
                    let final_output =
                        if is_mutating_tool && !result.is_error && result.output.len() > 100 {
                            if let Some(ref dm) = dual_mind {
                                let review_result = {
                                    let mut dm_guard = dm.write().await;
                                    dm_guard.post_review(&result.output).await
                                };

                                if let DialogueResult::NeedsEnhancement { critique, .. } =
                                    review_result
                                {
                                    tracing::info!(
                                        "Little Claw found issue with {} output: {}",
                                        tool_name,
                                        critique
                                    );
                                    if let Some(ref tx) = dual_mind_tx {
                                        let _ = tx.send(DualMindUpdate {
                                            enhancement: Some(critique.clone()),
                                            review_output: Some(critique.clone()),
                                        });
                                    }
                                    format!("{}\n\n[Quality Review]: {}", result.output, critique)
                                } else {
                                    if let Some(ref tx) = dual_mind_tx {
                                        let _ = tx.send(DualMindUpdate {
                                            enhancement: None,
                                            review_output: Some(result.output.clone()),
                                        });
                                    }
                                    result.output
                                }
                            } else {
                                result.output
                            }
                        } else {
                            result.output
                        };

                    tool_results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(final_output),
                        is_error: if result.is_error { Some(true) } else { None },
                    });
                } else {
                    tool_results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(format!(
                            "Error: Unknown tool '{}'",
                            tool_name
                        )),
                        is_error: Some(true),
                    });
                }

                // Clear dialogue depth after each tool so subsequent tools aren't skipped
                if let Some(ref dm) = dual_mind {
                    let mut dm_guard = dm.write().await;
                    dm_guard.take_dialogue();
                }

                if cancel_token.is_cancelled() {
                    break;
                }
            }

            let _ = result_tx.send(tool_results);
        });
    }

    /// Create visual blocks for tool calls
    fn create_tool_blocks(&mut self, tools: &[AiToolCall]) {
        for tool_call in tools {
            let tool_name = &tool_call.name;

            if tool_name == "bash" {
                let command = tool_call
                    .arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("bash")
                    .to_string();
                self.runtime
                    .blocks
                    .bash
                    .push(crate::tui::blocks::BashBlock::with_tool_id(
                        command,
                        tool_call.id.clone(),
                    ));
                self.runtime
                    .chat
                    .messages
                    .push(("bash".to_string(), tool_call.id.clone()));
            }

            if tool_name == "grep" || tool_name == "glob" || tool_name == "search_codebase" {
                let pattern = tool_call
                    .arguments
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string();
                self.runtime
                    .blocks
                    .tool_result
                    .push(crate::tui::blocks::ToolResultBlock::new(
                        tool_call.id.clone(),
                        tool_name.clone(),
                        pattern,
                    ));
                self.runtime
                    .chat
                    .messages
                    .push(("tool_result".to_string(), tool_call.id.clone()));
            }

            if tool_name == "read" {
                let file_path = tool_call
                    .arguments
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file")
                    .to_string();
                self.runtime
                    .blocks
                    .read
                    .push(crate::tui::blocks::ReadBlock::new(
                        tool_call.id.clone(),
                        file_path,
                    ));
                self.runtime
                    .chat
                    .messages
                    .push(("read".to_string(), tool_call.id.clone()));
            }

            if tool_name == "edit" {
                let file_path = tool_call
                    .arguments
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file")
                    .to_string();
                let old_string = tool_call
                    .arguments
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let new_string = tool_call
                    .arguments
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let start_line = 1;

                if let Some(block) = self.runtime.blocks.edit.last_mut() {
                    if block.is_pending() {
                        block.set_diff_data(file_path, old_string, new_string, start_line);
                    }
                }
            }

            if tool_name == "write" {
                let file_path = tool_call
                    .arguments
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file")
                    .to_string();
                let content = tool_call
                    .arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(block) = self.runtime.blocks.write.last_mut() {
                    if block.is_pending() {
                        block.set_content(file_path, content);
                    }
                }
            }

            if tool_name == "explore" || tool_name == "Task" {
                let prompt = tool_call
                    .arguments
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Exploring...")
                    .to_string();
                tracing::info!(
                    "spawn_tool_execution: creating ExploreBlock for '{}' with id={}",
                    tool_name,
                    tool_call.id
                );
                self.runtime
                    .blocks
                    .explore
                    .push(crate::tui::blocks::ExploreBlock::with_tool_id(
                        prompt,
                        tool_call.id.clone(),
                    ));
                self.runtime
                    .chat
                    .messages
                    .push(("explore".to_string(), tool_call.id.clone()));
                if self.ui.scroll_system.scroll.auto_scroll {
                    self.ui.scroll_system.scroll.request_scroll_to_bottom();
                }
            }

            if tool_name == "build" {
                let prompt = tool_call
                    .arguments
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Building...")
                    .to_string();
                tracing::info!(
                    "spawn_tool_execution: creating BuildBlock for 'build' with id={}",
                    tool_call.id
                );
                self.runtime
                    .blocks
                    .build
                    .push(crate::tui::blocks::BuildBlock::with_tool_id(
                        prompt,
                        tool_call.id.clone(),
                    ));
                self.runtime
                    .chat
                    .messages
                    .push(("build".to_string(), tool_call.id.clone()));
                if self.ui.scroll_system.scroll.auto_scroll {
                    self.ui.scroll_system.scroll.request_scroll_to_bottom();
                }
            }
        }
    }

    /// Handle completed tool results
    pub fn handle_tool_results(&mut self, tool_results: Vec<Content>) {
        if tool_results.is_empty() {
            return;
        }

        tracing::info!(
            result_count = tool_results.len(),
            explore_block_count = self.runtime.blocks.explore.len(),
            "handle_tool_results called"
        );

        // Update blocks with results
        for result in &tool_results {
            if let Content::ToolResult {
                tool_use_id,
                output,
                ..
            } = result
            {
                let output_str = match output {
                    serde_json::Value::String(s) => s.as_str(),
                    _ => "",
                };

                tracing::info!(
                    tool_use_id = %tool_use_id,
                    output_len = output_str.len(),
                    has_summary = output_str.contains("**Summary**"),
                    "Processing tool result"
                );

                self.update_tool_result_block(tool_use_id, output_str);
                self.update_read_block(tool_use_id, output_str);
                self.update_bash_block(tool_use_id, output_str);
                self.update_explore_block(tool_use_id, output_str);
                self.update_build_block(tool_use_id, output_str);
            }
        }

        // Combine with any pending results
        let mut all_results = std::mem::take(&mut self.runtime.pending_tool_results);
        all_results.extend(tool_results);

        // Process queued tools if any explore tools completed
        if !self.runtime.queued_tools.is_empty() {
            let queued = std::mem::take(&mut self.runtime.queued_tools);
            tracing::info!(
                "handle_tool_results: processing {} queued tools",
                queued.len()
            );
            self.spawn_tool_execution(queued);
            // Store results for later
            self.runtime.pending_tool_results = all_results;
            return;
        }

        // If decision prompt is visible, defer tool results until user decides
        // This prevents the AI from continuing while waiting for user input
        if self.ui.decision_prompt.visible {
            tracing::info!(
                "Decision prompt visible - deferring {} tool results",
                all_results.len()
            );
            self.runtime.pending_tool_results = all_results;
            self.stop_tool_execution();
            return;
        }

        // Inject exploration budget warning if threshold exceeded
        const EXPLORATION_BUDGET_SOFT: usize = 15;
        const EXPLORATION_BUDGET_HARD: usize = 30;
        if self.runtime.exploration_budget_count >= EXPLORATION_BUDGET_HARD {
            let warning = format!(
                "[EXPLORATION BUDGET EXCEEDED]\n\
                You have made {} consecutive read-only operations without taking action.\n\
                STOP exploring and take action NOW.\n\
                If you are working on a plan, call task_start and begin implementation.\n\
                If you need more context, use search_codebase for targeted results.\n\
                Further exploration without action is unacceptable.",
                self.runtime.exploration_budget_count
            );
            all_results.push(Content::Text { text: warning });
        } else if self.runtime.exploration_budget_count >= EXPLORATION_BUDGET_SOFT {
            let warning = format!(
                "[EXPLORATION BUDGET]\n\
                You have made {} consecutive read-only operations without taking action.\n\
                If you are working on a plan, call task_start and begin implementation.\n\
                If you need more context, use search_codebase for targeted results.\n\
                Continued exploration without action is wasteful. Act now or explain why you need more context.",
                self.runtime.exploration_budget_count
            );
            all_results.push(Content::Text { text: warning });
        }

        // Add tool results to conversation
        let tool_result_msg = crate::ai::types::ModelMessage {
            role: crate::ai::types::Role::User,
            content: all_results,
        };

        self.stop_tool_execution();
        self.runtime.chat.conversation.push(tool_result_msg.clone());
        self.save_model_message(&tool_result_msg);

        // Continue conversation with AI
        self.send_to_ai();
    }

    /// Update ToolResultBlock with output
    fn update_tool_result_block(&mut self, tool_use_id: &str, output_str: &str) {
        for block in &mut self.runtime.blocks.tool_result {
            if block.tool_use_id() == tool_use_id {
                block.set_results(output_str);
                block.complete();
                break;
            }
        }
    }

    /// Update ReadBlock with content
    fn update_read_block(&mut self, tool_use_id: &str, output_str: &str) {
        for block in &mut self.runtime.blocks.read {
            if block.tool_use_id() == tool_use_id {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(output_str) {
                    let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let total_lines = json
                        .get("total_lines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let lines_returned = json
                        .get("lines_returned")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    block.set_content(content.to_string(), total_lines, lines_returned);
                } else {
                    let lines: Vec<&str> = output_str.lines().collect();
                    block.set_content(output_str.to_string(), lines.len(), lines.len());
                }
                break;
            }
        }
    }

    /// Update BashBlock for background processes
    fn update_bash_block(&mut self, tool_use_id: &str, output_str: &str) {
        for block in &mut self.runtime.blocks.bash {
            if block.tool_use_id() == Some(tool_use_id) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(output_str) {
                    if let Some(process_id) = json.get("processId").and_then(|v| v.as_str()) {
                        block.set_background_process_id(process_id.to_string());
                        tracing::info!(
                            tool_use_id = %tool_use_id,
                            process_id = %process_id,
                            "BashBlock converted to background process"
                        );
                    }
                }
                break;
            }
        }
    }

    /// Update ExploreBlock with results
    fn update_explore_block(&mut self, tool_use_id: &str, output_str: &str) {
        tracing::info!(
            explore_blocks = self.runtime.blocks.explore.len(),
            tool_use_id = %tool_use_id,
            "Looking for matching ExploreBlock"
        );
        for block in &mut self.runtime.blocks.explore {
            if block.tool_use_id() == Some(tool_use_id) {
                tracing::info!(
                    tool_use_id = %tool_use_id,
                    output_len = output_str.len(),
                    "Found matching ExploreBlock, completing with output"
                );
                block.complete(output_str.to_string());
                break;
            }
        }
    }

    /// Update BuildBlock with results
    fn update_build_block(&mut self, tool_use_id: &str, output_str: &str) {
        for block in &mut self.runtime.blocks.build {
            if block.tool_use_id() == Some(tool_use_id) {
                tracing::info!(
                    tool_use_id = %tool_use_id,
                    output_len = output_str.len(),
                    "Found matching BuildBlock, completing with output"
                );
                block.complete(output_str.to_string());
                break;
            }
        }
    }
}

/// Create appropriate observation based on tool type
/// Uses specific observation constructors for file operations to preserve path info
fn create_observation(
    tool_name: &str,
    args: &serde_json::Value,
    output: &str,
    success: bool,
) -> Observation {
    // Truncate output for summary (char-boundary safe)
    let summary = if output.len() > 500 {
        let mut end = 500;
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...[truncated]", &output[..end])
    } else {
        output.to_string()
    };

    match tool_name {
        "edit" => {
            let path = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if success {
                Observation::file_edit(path, &summary, output)
            } else {
                Observation::tool_result(
                    tool_name,
                    &format!("Failed to edit {}: {}", path, summary),
                    false,
                )
            }
        }
        "write" => {
            let path = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if success {
                Observation::file_write(path, &summary)
            } else {
                Observation::tool_result(
                    tool_name,
                    &format!("Failed to write {}: {}", path, summary),
                    false,
                )
            }
        }
        "read" => {
            let path = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Observation {
                action: ObservedAction::FileRead {
                    path: path.to_string(),
                },
                summary: format!("Read {}", path),
                content: Some(summary),
                success,
            }
        }
        "bash" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Observation::bash(cmd, output, success)
        }
        _ => Observation::tool_result(tool_name, &summary, success),
    }
}

/// Create a concise intent summary for Little Claw review
/// Avoids dumping full JSON - extracts key info only
fn create_intent_summary(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "edit" => {
            let file = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("Edit file: {}", file)
        }
        "write" => {
            let file = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("Write file: {}", file)
        }
        "read" => {
            let file = args
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("Read file: {}", file)
        }
        "bash" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            // Truncate long commands (char-boundary safe)
            let cmd_preview = if cmd.len() > 100 {
                let mut end = 100;
                while end > 0 && !cmd.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &cmd[..end])
            } else {
                cmd.to_string()
            };
            format!("Run command: {}", cmd_preview)
        }
        "grep" | "glob" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");
            format!("{}: {}", tool_name, pattern)
        }
        _ => format!("Execute {} tool", tool_name),
    }
}
