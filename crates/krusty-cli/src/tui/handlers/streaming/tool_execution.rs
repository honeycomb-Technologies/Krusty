//! Tool execution and result handling
//!
//! Handles the execution of AI tool calls and processing of results.

use tokio::sync::{mpsc, oneshot};

use crate::agent::subagent::AgentProgress;
use crate::ai::types::{AiToolCall, Content};
use crate::tools::{ToolContext, ToolOutputChunk};
use crate::tui::app::App;
use crate::tui::components::{PromptOption, PromptQuestion};

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
                self.active_plan = None;
                tracing::info!("Cleared existing plan");
            }

            self.work_mode = WorkMode::Plan;
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
            self.pending_tool_results.extend(results);
        }
    }

    /// Handle task_complete tool calls to update plan immediately
    /// Supports both single task_id and batch task_ids array
    pub(super) fn handle_task_complete_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling task_complete tool call: {}", tool_call.id);

            // Collect task IDs - support both single task_id and batch task_ids
            let mut task_ids: Vec<String> = Vec::new();

            if let Some(id) = tool_call.arguments.get("task_id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    task_ids.push(id.to_string());
                }
            }

            if let Some(ids) = tool_call
                .arguments
                .get("task_ids")
                .and_then(|v| v.as_array())
            {
                for id in ids {
                    if let Some(s) = id.as_str() {
                        if !s.is_empty() {
                            task_ids.push(s.to_string());
                        }
                    }
                }
            }

            if task_ids.is_empty() {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: task_id or task_ids required".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            }

            let Some(plan) = &mut self.active_plan else {
                results.push(Content::ToolResult {
                    tool_use_id: tool_call.id.clone(),
                    output: serde_json::Value::String(
                        "Error: No active plan. Create a plan first.".to_string(),
                    ),
                    is_error: Some(true),
                });
                continue;
            };

            let mut completed_ids = Vec::new();
            let mut not_found = Vec::new();

            for task_id in &task_ids {
                if plan.check_task(task_id) {
                    completed_ids.push(task_id.clone());
                } else {
                    not_found.push(task_id.clone());
                }
            }

            if !completed_ids.is_empty() {
                if let Err(e) = self.services.plan_manager.save_plan(plan) {
                    tracing::error!("Failed to save plan after task completion: {}", e);
                }
            }

            let (completed, total) = plan.progress();

            let msg = if not_found.is_empty() {
                format!(
                    "Marked {} task(s) complete. Progress: {}/{}",
                    completed_ids.len(),
                    completed,
                    total
                )
            } else {
                format!(
                    "Marked {} task(s) complete, {} not found. Progress: {}/{}",
                    completed_ids.len(),
                    not_found.len(),
                    completed,
                    total
                )
            };
            tracing::info!("{}", msg);

            results.push(Content::ToolResult {
                tool_use_id: tool_call.id.clone(),
                output: serde_json::Value::String(msg),
                is_error: if not_found.is_empty() {
                    None
                } else {
                    Some(false)
                },
            });
        }

        if !results.is_empty() {
            self.pending_tool_results.extend(results);
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
            let multi_select = q.get("multiSelect").and_then(|v| v.as_bool()).unwrap_or(false);

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
        if let Some((tag, _)) = self.messages.last() {
            if tag == "tool" {
                self.messages.pop();
            }
        }

        self.decision_prompt.show_ask_user(prompt_questions, tool_call.id);
    }

    /// Spawn tool execution as a background task for non-blocking streaming
    pub fn spawn_tool_execution(&mut self, tool_calls: Vec<AiToolCall>) {
        let tool_names: Vec<_> = tool_calls.iter().map(|t| t.name.as_str()).collect();
        tracing::info!(
            "spawn_tool_execution: {} tools to execute: {:?}",
            tool_calls.len(),
            tool_names
        );

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

        // Intercept enter_plan_mode tool
        let (plan_mode_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "enter_plan_mode");

        let has_plan_mode = !plan_mode_tools.is_empty();
        if has_plan_mode {
            self.handle_enter_plan_mode_tools(plan_mode_tools);
        }

        if tool_calls.is_empty() {
            if has_ask_user {
                self.stop_streaming();
                return;
            }

            if has_task_complete || has_plan_mode {
                let results = std::mem::take(&mut self.pending_tool_results);
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
                self.queued_tools.extend(other_tools);
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
        self.channels.bash_output = Some(output_rx);

        // Create explore progress channel if any explore tools
        let explore_progress_tx = if has_explore {
            let (tx, rx) = mpsc::unbounded_channel::<AgentProgress>();
            self.channels.explore_progress = Some(rx);
            Some(tx)
        } else {
            None
        };

        // Create build progress channel if any build tools
        let build_progress_tx = if has_build {
            let (tx, rx) = mpsc::unbounded_channel::<AgentProgress>();
            self.channels.build_progress = Some(rx);
            Some(tx)
        } else {
            None
        };

        // Create missing LSP channel
        let (missing_lsp_tx, missing_lsp_rx) =
            mpsc::unbounded_channel::<crate::lsp::manager::MissingLspInfo>();
        self.channels.missing_lsp = Some(missing_lsp_rx);

        // Create result channel
        let (result_tx, result_rx) = oneshot::channel();
        self.channels.tool_results = Some(result_rx);

        self.start_tool_execution();

        // Create blocks for visual feedback
        self.create_tool_blocks(&tools_to_execute);

        // Clone what we need for the spawned task
        let tool_registry = self.services.tool_registry.clone();
        let lsp_manager = self.services.lsp_manager.clone();
        let process_registry = self.process_registry.clone();
        let skills_manager = self.services.skills_manager.clone();
        let cancel_token = self.cancellation.child_token();
        let plan_mode = self.work_mode == crate::tui::app::WorkMode::Plan;
        let current_model = self.current_model.clone();

        tokio::spawn(async move {
            let mut tool_results: Vec<Content> = Vec::new();

            for tool_call in tools_to_execute {
                if cancel_token.is_cancelled() {
                    tracing::info!("Tool execution cancelled before running {}", tool_call.name);
                    break;
                }

                let tool_name = tool_call.name.clone();
                let working_dir =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

                let mut ctx = ToolContext::with_lsp_and_processes(
                    working_dir,
                    lsp_manager.clone(),
                    process_registry.clone(),
                )
                .with_skills_manager(skills_manager.clone())
                .with_missing_lsp_channel(missing_lsp_tx.clone())
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
                    tool_results.push(Content::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        output: serde_json::Value::String(result.output),
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
                self.blocks
                    .bash
                    .push(crate::tui::blocks::BashBlock::with_tool_id(
                        command,
                        tool_call.id.clone(),
                    ));
                self.messages
                    .push(("bash".to_string(), tool_call.id.clone()));
            }

            if tool_name == "grep" || tool_name == "glob" {
                let pattern = tool_call
                    .arguments
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*")
                    .to_string();
                self.blocks
                    .tool_result
                    .push(crate::tui::blocks::ToolResultBlock::new(
                        tool_call.id.clone(),
                        tool_name.clone(),
                        pattern,
                    ));
                self.messages
                    .push(("tool_result".to_string(), tool_call.id.clone()));
            }

            if tool_name == "read" {
                let file_path = tool_call
                    .arguments
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file")
                    .to_string();
                self.blocks.read.push(crate::tui::blocks::ReadBlock::new(
                    tool_call.id.clone(),
                    file_path,
                ));
                self.messages
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

                if let Some(block) = self.blocks.edit.last_mut() {
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

                if let Some(block) = self.blocks.write.last_mut() {
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
                self.blocks
                    .explore
                    .push(crate::tui::blocks::ExploreBlock::with_tool_id(
                        prompt,
                        tool_call.id.clone(),
                    ));
                self.messages
                    .push(("explore".to_string(), tool_call.id.clone()));
                if self.scroll.auto_scroll {
                    self.scroll.request_scroll_to_bottom();
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
                self.blocks
                    .build
                    .push(crate::tui::blocks::BuildBlock::with_tool_id(
                        prompt,
                        tool_call.id.clone(),
                    ));
                self.messages
                    .push(("build".to_string(), tool_call.id.clone()));
                if self.scroll.auto_scroll {
                    self.scroll.request_scroll_to_bottom();
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
            explore_block_count = self.blocks.explore.len(),
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
        let mut all_results = std::mem::take(&mut self.pending_tool_results);
        all_results.extend(tool_results);

        // Process queued tools if any explore tools completed
        if !self.queued_tools.is_empty() {
            let queued = std::mem::take(&mut self.queued_tools);
            tracing::info!(
                "handle_tool_results: processing {} queued tools",
                queued.len()
            );
            self.spawn_tool_execution(queued);
            // Store results for later
            self.pending_tool_results = all_results;
            return;
        }

        // Add tool results to conversation
        let tool_result_msg = crate::ai::types::ModelMessage {
            role: crate::ai::types::Role::User,
            content: all_results,
        };

        self.stop_tool_execution();
        self.conversation.push(tool_result_msg.clone());
        self.save_model_message(&tool_result_msg);

        // Continue conversation with AI
        self.send_to_ai();
    }

    /// Update ToolResultBlock with output
    fn update_tool_result_block(&mut self, tool_use_id: &str, output_str: &str) {
        for block in &mut self.blocks.tool_result {
            if block.tool_use_id() == tool_use_id {
                block.set_results(output_str);
                block.complete();
                break;
            }
        }
    }

    /// Update ReadBlock with content
    fn update_read_block(&mut self, tool_use_id: &str, output_str: &str) {
        for block in &mut self.blocks.read {
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
        for block in &mut self.blocks.bash {
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
            explore_blocks = self.blocks.explore.len(),
            tool_use_id = %tool_use_id,
            "Looking for matching ExploreBlock"
        );
        for block in &mut self.blocks.explore {
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
        for block in &mut self.blocks.build {
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
