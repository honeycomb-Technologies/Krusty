//! AI streaming and tool execution handlers
//!
//! Handles sending messages to AI and executing tool calls

use tokio::sync::mpsc;

use crate::agent::{AgentEvent, InterruptReason};
use crate::ai::anthropic::CallOptions;
use crate::ai::streaming::StreamPart;
use crate::ai::types::{
    AiToolCall, Content, ContextManagement, ModelMessage, Role, ThinkingConfig, WebFetchConfig,
    WebSearchConfig,
};
use crate::tools::{load_from_clipboard_rgba, load_from_path, load_from_url, ToolContext};
use crate::tui::app::{App, View};
use crate::tui::blocks::StreamBlock;
use crate::tui::input::{has_image_references, parse_input, InputSegment};

/// Maximum number of images allowed per message
const MAX_IMAGES_PER_MESSAGE: usize = 20;

/// Check if image count exceeds the maximum
fn check_image_limit(count: usize) -> anyhow::Result<()> {
    if count > MAX_IMAGES_PER_MESSAGE {
        anyhow::bail!(
            "Too many images (max {} per message)",
            MAX_IMAGES_PER_MESSAGE
        );
    }
    Ok(())
}

/// Sanitize plan titles for safe markdown embedding
/// Escapes backticks and quotes that could break formatting
fn sanitize_plan_title(title: &str) -> String {
    title
        .replace(['`', '"'], "'")
        .replace('[', "(")
        .replace(']', ")")
}

impl App {
    /// Handle user input submission (message or command)
    pub fn handle_input_submit(&mut self, text: String) {
        // Check if this is a slash command vs a file path
        // /help, /model, /clear = commands (single word after /)
        // /home/user/file.pdf = file path (has more slashes or file extension)
        if text.starts_with('/') && !Self::looks_like_file_path(&text) {
            self.handle_slash_command(&text);
            return;
        }

        if self.view == View::StartMenu {
            self.view = View::Chat;
        }

        // Check if authenticated
        if !self.is_authenticated() {
            self.messages.push((
                "system".to_string(),
                "Not authenticated. Use /auth to set up API key.".to_string(),
            ));
            return;
        }

        // Create session if this is the first message
        if self.current_session_id.is_none() {
            self.create_session(&text);
        }

        // Build user message content (may include images)
        let (content_blocks, display_text) = match self.build_user_content(&text) {
            Ok(result) => result,
            Err(e) => {
                self.messages
                    .push(("system".to_string(), format!("Error: {}", e)));
                return;
            }
        };

        // Add user message to display and conversation
        self.messages.push(("user".to_string(), display_text));
        let user_msg = ModelMessage {
            role: Role::User,
            content: content_blocks,
        };
        self.conversation.push(user_msg.clone());

        // Save user message to session
        self.save_model_message(&user_msg);

        // Start streaming response
        self.send_to_ai();
    }

    /// Build user message content from input text
    /// Parses [image] references and loads images
    fn build_user_content(&mut self, text: &str) -> anyhow::Result<(Vec<Content>, String)> {
        // Fast path: no image references
        if !has_image_references(text) {
            return Ok((
                vec![Content::Text {
                    text: text.to_string(),
                }],
                text.to_string(),
            ));
        }

        let segments = parse_input(text, &self.working_dir);
        let mut content_blocks = Vec::new();
        let mut display_parts = Vec::new();
        let mut image_count = 0;

        for segment in segments {
            match segment {
                InputSegment::Text(t) => {
                    if !t.is_empty() {
                        content_blocks.push(Content::Text { text: t.clone() });
                        display_parts.push(t);
                    }
                }
                InputSegment::ImagePath(path) => {
                    image_count += 1;
                    check_image_limit(image_count)?;
                    let loaded = load_from_path(&path)?;
                    let file_type = match &loaded.content {
                        Content::Document { .. } => "PDF",
                        _ => "Image",
                    };
                    // Track the file for preview lookup
                    self.attached_files
                        .insert(loaded.display_name.clone(), path.clone());
                    display_parts.push(format!("[{}: {}]", file_type, loaded.display_name));
                    content_blocks.push(loaded.content);
                }
                InputSegment::ImageUrl(url) => {
                    image_count += 1;
                    check_image_limit(image_count)?;
                    let loaded = load_from_url(&url)?;
                    content_blocks.push(loaded.content);
                    display_parts.push(format!("[Image: {}]", loaded.display_name));
                }
                InputSegment::ClipboardImage(id) => {
                    // Extract clipboard id (format: "clipboard:uuid")
                    let clipboard_id = id.strip_prefix("clipboard:").unwrap_or(&id);
                    if let Some((width, height, rgba_bytes)) =
                        self.pending_clipboard_images.remove(clipboard_id)
                    {
                        image_count += 1;
                        check_image_limit(image_count)?;
                        let loaded = load_from_clipboard_rgba(width, height, &rgba_bytes)?;
                        content_blocks.push(loaded.content);
                        display_parts.push(format!("[Image: {}]", loaded.display_name));
                    } else {
                        // Clipboard image not found, treat as text
                        display_parts.push(format!("[{}]", id));
                        content_blocks.push(Content::Text {
                            text: format!("[{}]", id),
                        });
                    }
                }
            }
        }

        let display_text = display_parts.join("");
        Ok((content_blocks, display_text))
    }

    /// Check if text looks like a file path rather than a slash command
    /// Returns true for paths like /home/user/file.pdf, false for /help
    fn looks_like_file_path(text: &str) -> bool {
        // Get the first "word" (text before any space)
        let first_word = text.split_whitespace().next().unwrap_or(text);

        // If there's a second / in the path, it's likely a file path
        // /home/user = file path, /help = command
        if first_word.chars().skip(1).any(|c| c == '/') {
            return true;
        }

        // If it ends with a supported file extension, it's a file path
        let extensions = [".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp"];
        let lower = first_word.to_lowercase();
        extensions.iter().any(|ext| lower.ends_with(ext))
    }

    /// Send the current conversation to the AI and start streaming response
    pub fn send_to_ai(&mut self) {
        if self.is_busy() {
            tracing::warn!("send_to_ai called while already busy - skipping");
            return;
        }

        tracing::info!(
            "=== send_to_ai START === conversation_len={}",
            self.conversation.len()
        );

        // Log conversation structure for debugging
        for (i, msg) in self.conversation.iter().enumerate() {
            let content_summary: Vec<String> = msg
                .content
                .iter()
                .map(|c| match c {
                    crate::ai::types::Content::Text { text } => format!("Text({})", text.len()),
                    crate::ai::types::Content::ToolUse { id, name, .. } => {
                        format!("ToolUse({}, {})", name, id)
                    }
                    crate::ai::types::Content::ToolResult { tool_use_id, .. } => {
                        format!("ToolResult({})", tool_use_id)
                    }
                    crate::ai::types::Content::Image { .. } => "Image".to_string(),
                    crate::ai::types::Content::Document { .. } => "Document".to_string(),
                    crate::ai::types::Content::Thinking { thinking, .. } => {
                        format!("Thinking({})", thinking.len())
                    }
                    crate::ai::types::Content::RedactedThinking { .. } => {
                        "RedactedThinking".to_string()
                    }
                })
                .collect();
            tracing::debug!(
                "  conversation[{}] {:?}: {:?}",
                i,
                msg.role,
                content_summary
            );
        }

        // Check max turns limit
        if self
            .agent_config
            .exceeded_max_turns(self.agent_state.current_turn)
        {
            self.event_bus.emit(AgentEvent::Interrupt {
                turn: self.agent_state.current_turn,
                reason: InterruptReason::MaxTurnsReached,
            });
            self.messages.push((
                "system".to_string(),
                format!(
                    "Max turns ({}) reached. Use /home to start a new session.",
                    self.agent_config.max_turns.unwrap_or(0)
                ),
            ));
            return;
        }

        let Some(ref _client) = self.ai_client else {
            self.messages
                .push(("system".to_string(), "No AI client configured".to_string()));
            return;
        };

        self.start_streaming();
        self.streaming.reset();

        // Track turn start
        self.agent_state.start_turn();
        self.event_bus.emit(AgentEvent::TurnStart {
            turn: self.agent_state.current_turn,
            message_count: self.conversation.len(),
        });

        // NOTE: Don't create assistant message placeholder here!
        // It will be created on-demand when first TextDelta arrives,
        // ensuring it appears AFTER any thinking blocks in the timeline.

        // Inject LSP diagnostics into context if any exist
        // NOTE: Only inject if thinking is disabled OR conversation is empty,
        // because injecting a fake assistant message without thinking blocks
        // will cause API errors when thinking is enabled.
        let diagnostics_context = self.build_diagnostics_context();
        let plan_context = self.build_plan_context();
        let skills_context = self.build_skills_context();
        let project_context = self.build_project_context();
        if !plan_context.is_empty() {
            tracing::info!(
                "Injecting plan context ({} chars) into conversation",
                plan_context.len()
            );
        }
        if !skills_context.is_empty() {
            tracing::debug!(
                "Injecting skills context ({} chars) into conversation",
                skills_context.len()
            );
        }
        if !project_context.is_empty() {
            tracing::info!(
                "Injecting project context ({} chars) into conversation",
                project_context.len()
            );
        }
        let has_thinking_conversation = self.thinking_enabled
            && self.conversation.iter().any(|msg| {
                msg.role == Role::Assistant
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::Thinking { .. }))
            });

        // Clone conversation and inject context as needed
        let mut conversation = self.conversation.clone();

        // Track how many System messages we've inserted
        let mut system_insert_count = 0;

        // Inject project context FIRST (foundational codebase rules from instruction files)
        if !project_context.is_empty() {
            conversation.insert(
                system_insert_count,
                ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text {
                        text: project_context,
                    }],
                },
            );
            system_insert_count += 1;
        }

        // Inject plan context as System message (gets picked up by injected_context mechanism)
        if !plan_context.is_empty() {
            conversation.insert(
                system_insert_count,
                ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text { text: plan_context }],
                },
            );
            system_insert_count += 1;
        }

        // Inject skills context as System message (after project and plan context)
        if !skills_context.is_empty() {
            conversation.insert(
                system_insert_count,
                ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text {
                        text: skills_context,
                    }],
                },
            );
        }

        // Inject diagnostics as user/assistant pair (avoids thinking mode issues)
        if !diagnostics_context.is_empty() && !has_thinking_conversation {
            // Count System messages at start (plan and/or skills context)
            let system_count = conversation
                .iter()
                .take_while(|m| m.role == Role::System)
                .count();
            let insert_pos = system_count;
            conversation.insert(
                insert_pos,
                ModelMessage {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: diagnostics_context,
                    }],
                },
            );
            conversation.insert(
                insert_pos + 1,
                ModelMessage {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: "I understand the current diagnostics. I'll take them into account."
                            .to_string(),
                    }],
                },
            );
        }

        // Create client for the call
        let client = match self.create_ai_client() {
            Some(c) => c,
            None => {
                self.messages.push((
                    "system".to_string(),
                    "No authentication available".to_string(),
                ));
                return;
            }
        };

        // Use cached tools
        let tools = self.cached_ai_tools.clone();
        let tool_names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        tracing::info!("Sending {} tools to API: {:?}", tools.len(), tool_names);

        // Thinking mode: user preference controls whether to request thinking
        // The API will handle conversations that mix thinking/non-thinking messages
        // We just need to ensure we're not sending malformed thinking blocks
        let can_use_thinking = self.thinking_enabled;

        // Build call options with thinking config if enabled and allowed
        let thinking = can_use_thinking.then(ThinkingConfig::default);

        // Context management based on whether thinking is enabled
        let context_management = match (can_use_thinking, !tools.is_empty()) {
            (true, _) => Some(ContextManagement::default_for_thinking_and_tools()),
            (false, true) => Some(ContextManagement::default_tools_only()),
            (false, false) => None,
        };

        let options = CallOptions {
            tools: (!tools.is_empty()).then_some(tools),
            thinking,
            enable_caching: true,
            context_management,
            // Enable server-executed web tools by default
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        // Reset cancellation for new request
        self.cancellation.reset();
        let cancel_token = self.cancellation.child_token();

        // Spawn task to call API
        let (tx, rx) = mpsc::unbounded_channel();
        self.streaming.start_stream(rx);

        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    // Request was cancelled
                    let _ = tx.send(StreamPart::Error {
                        error: "Interrupted by user".to_string()
                    });
                }
                result = client.call_streaming(conversation, &options) => {
                    match result {
                        Ok(mut api_rx) => {
                            loop {
                                tokio::select! {
                                    _ = cancel_token.cancelled() => {
                                        let _ = tx.send(StreamPart::Error {
                                            error: "Interrupted by user".to_string()
                                        });
                                        break;
                                    }
                                    part = api_rx.recv() => {
                                        match part {
                                            Some(p) => {
                                                if tx.send(p).is_err() {
                                                    break;
                                                }
                                            }
                                            None => break,
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(StreamPart::Error { error: e.to_string() });
                        }
                    }
                }
            }
        });
    }

    /// Handle enter_plan_mode tool calls to switch modes
    fn handle_enter_plan_mode_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        use crate::ai::types::Content;
        use crate::tui::app::WorkMode;

        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling enter_plan_mode tool call: {}", tool_call.id);

            // Parse arguments
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

            // Clear existing plan if requested
            if clear_existing {
                self.active_plan = None;
                tracing::info!("Cleared existing plan");
            }

            // Switch to Plan mode
            self.work_mode = WorkMode::Plan;

            // Note: No system message - mode switch is visible via status bar
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

        // Add results to pending
        if !results.is_empty() {
            self.pending_tool_results.extend(results);
        }
    }

    /// Handle task_complete tool calls to update plan immediately
    /// Supports both single task_id and batch task_ids array
    fn handle_task_complete_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        use crate::ai::types::Content;

        let mut results = Vec::new();

        for tool_call in tool_calls {
            tracing::info!("Handling task_complete tool call: {}", tool_call.id);

            // Collect task IDs - support both single task_id and batch task_ids
            let mut task_ids: Vec<String> = Vec::new();

            // Check for single task_id
            if let Some(id) = tool_call.arguments.get("task_id").and_then(|v| v.as_str()) {
                if !id.is_empty() {
                    task_ids.push(id.to_string());
                }
            }

            // Check for batch task_ids array
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

            // Check if we have an active plan
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

            // Mark all tasks complete
            let mut completed_ids = Vec::new();
            let mut not_found = Vec::new();

            for task_id in &task_ids {
                if plan.check_task(task_id) {
                    completed_ids.push(task_id.clone());
                } else {
                    not_found.push(task_id.clone());
                }
            }

            // Save the plan if any tasks were completed
            if !completed_ids.is_empty() {
                if let Err(e) = self.plan_manager.save_plan(plan) {
                    tracing::error!("Failed to save plan after task completion: {}", e);
                }
            }

            let (completed, total) = plan.progress();

            // Build result message
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

        // Add results to pending
        if !results.is_empty() {
            self.pending_tool_results.extend(results);
        }
    }

    /// Handle AskUserQuestion tool calls via UI instead of registry
    fn handle_ask_user_question_tools(&mut self, tool_calls: Vec<AiToolCall>) {
        use crate::tui::components::{PromptOption, PromptQuestion};

        // For now, handle only the first AskUserQuestion (batch support can come later)
        let Some(tool_call) = tool_calls.into_iter().next() else {
            return;
        };

        tracing::info!("Handling AskUserQuestion tool call: {}", tool_call.id);

        // Parse the tool arguments
        // Schema: { questions: [{ question, header, options: [{label, description}], multiSelect }] }
        let questions_json = tool_call.arguments.get("questions");

        let mut prompt_questions = Vec::new();

        if let Some(serde_json::Value::Array(questions)) = questions_json {
            for q in questions {
                let header = q
                    .get("header")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Question")
                    .to_string();
                let question = q
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let multi_select = q
                    .get("multiSelect")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let mut pq = PromptQuestion::new(header, question).multi_select(multi_select);

                if let Some(serde_json::Value::Array(options)) = q.get("options") {
                    for opt in options {
                        let label = opt
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let description = opt.get("description").and_then(|v| v.as_str());

                        let mut po = PromptOption::new(label);
                        if let Some(desc) = description {
                            po = po.with_description(desc);
                        }
                        pq = pq.add_option(po);
                    }
                }

                prompt_questions.push(pq);
            }
        }

        if prompt_questions.is_empty() {
            // No valid questions - create a dummy result
            tracing::warn!("AskUserQuestion tool had no valid questions");
            return;
        }

        // Remove the "Preparing questions..." message now that we're showing the prompt
        if let Some((tag, _)) = self.messages.last() {
            if tag == "tool" {
                self.messages.pop();
            }
        }

        // Store the tool_use_id for later result sending
        self.decision_prompt
            .show_ask_user(prompt_questions, tool_call.id);
    }

    /// Spawn tool execution as a background task for non-blocking streaming
    pub fn spawn_tool_execution(&mut self, tool_calls: Vec<AiToolCall>) {
        use crate::agent::subagent::AgentProgress;
        use crate::tools::ToolOutputChunk;
        use tokio::sync::{mpsc, oneshot};

        let tool_names: Vec<_> = tool_calls.iter().map(|t| t.name.as_str()).collect();
        tracing::info!(
            "spawn_tool_execution: {} tools to execute: {:?}",
            tool_calls.len(),
            tool_names
        );

        if tool_calls.is_empty() {
            return;
        }

        // Intercept AskUserQuestion tool - handle in UI, not via registry
        let (ask_user_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "AskUserQuestion");

        let has_ask_user = !ask_user_tools.is_empty();
        if has_ask_user {
            self.handle_ask_user_question_tools(ask_user_tools);
        }

        // Intercept task_complete tool - handle in UI to update plan immediately
        let (task_complete_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "task_complete");

        let has_task_complete = !task_complete_tools.is_empty();
        if has_task_complete {
            self.handle_task_complete_tools(task_complete_tools);
        }

        // Intercept enter_plan_mode tool - handle in UI to switch modes
        let (plan_mode_tools, tool_calls): (Vec<_>, Vec<_>) = tool_calls
            .into_iter()
            .partition(|t| t.name == "enter_plan_mode");

        let has_plan_mode = !plan_mode_tools.is_empty();
        if has_plan_mode {
            self.handle_enter_plan_mode_tools(plan_mode_tools);
        }

        if tool_calls.is_empty() {
            if has_ask_user {
                // AskUserQuestion was intercepted - stop and wait for user input
                // The user's answer will trigger send_to_ai() to continue
                self.stop_streaming();
                return;
            }

            // task_complete or enter_plan_mode - continue with pending results immediately
            if has_task_complete || has_plan_mode {
                let results = std::mem::take(&mut self.pending_tool_results);
                if !results.is_empty() {
                    self.stop_streaming();
                    self.handle_tool_results(results);
                }
                return;
            }

            // No tools to execute
            return;
        }

        // Check if there's an explore/Task tool in the batch
        let has_explore = tool_calls
            .iter()
            .any(|t| t.name == "explore" || t.name == "Task");

        // Check if there's a build tool in the batch
        let has_build = tool_calls.iter().any(|t| t.name == "build");

        // If explore tool is present, queue non-explore tools for later
        // This ensures explore completes before other tools show in the timeline
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

        // Create missing LSP channel for write/edit tools to trigger install prompts
        let (missing_lsp_tx, missing_lsp_rx) =
            mpsc::unbounded_channel::<crate::lsp::manager::MissingLspInfo>();
        self.channels.missing_lsp = Some(missing_lsp_rx);

        // Create result channel
        let (result_tx, result_rx) = oneshot::channel();
        self.channels.tool_results = Some(result_rx);

        // Set tool execution state
        self.start_tool_execution();

        // Create blocks for visual feedback BEFORE spawning task
        for tool_call in &tools_to_execute {
            let tool_name = &tool_call.name;

            if tool_name == "bash" {
                let command = tool_call
                    .arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("bash")
                    .to_string();
                // Use with_tool_id to enable output matching by tool_use_id
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
                // Default start line for edit diff display
                let start_line = 1;

                // Find the pending edit block and populate it with diff data
                if let Some(block) = self.blocks.edit.last_mut() {
                    if block.is_pending() {
                        block.set_diff_data(file_path, old_string, new_string, start_line);
                    }
                }
                // Note: message was already added in ToolStart handler
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

                // Find the pending write block and populate it with content
                if let Some(block) = self.blocks.write.last_mut() {
                    if block.is_pending() {
                        block.set_content(file_path, content);
                    }
                }
                // Note: message was already added in ToolStart handler
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
                // Auto-scroll to show the explore block
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
                // Auto-scroll to show the build block
                if self.scroll.auto_scroll {
                    self.scroll.request_scroll_to_bottom();
                }
            }
        }

        // Clone what we need for the spawned task
        let tool_registry = self.tool_registry.clone();
        let lsp_manager = self.lsp_manager.clone();
        let process_registry = self.process_registry.clone();
        let skills_manager = self.skills_manager.clone();
        let cancel_token = self.cancellation.child_token();
        let plan_mode = self.work_mode == crate::tui::app::WorkMode::Plan;
        let current_model = self.current_model.clone();

        // Spawn tool execution task with cancellation support
        tokio::spawn(async move {
            let mut tool_results: Vec<Content> = Vec::new();

            for tool_call in tools_to_execute {
                // Check for cancellation before each tool
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

                // For bash, add streaming output channel
                if tool_name == "bash" {
                    ctx = ctx.with_output_stream(output_tx.clone(), tool_call.id.clone());
                }

                // Explore tool spawns sub-agents that can run for a long time
                if tool_name == "explore" || tool_name == "Task" {
                    ctx.timeout = Some(std::time::Duration::from_secs(600)); // 10 minutes
                                                                             // Add progress channel for real-time updates
                    if let Some(ref tx) = explore_progress_tx {
                        ctx = ctx.with_explore_progress(tx.clone());
                    }
                }

                // Build tool spawns parallel Opus builders
                if tool_name == "build" {
                    ctx.timeout = Some(std::time::Duration::from_secs(900)); // 15 minutes for builders
                                                                             // Add progress channel for real-time updates
                    if let Some(ref tx) = build_progress_tx {
                        ctx = ctx.with_build_progress(tx.clone());
                    }
                }

                // Execute tool with cancellation check
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

                // Exit loop if cancelled
                if cancel_token.is_cancelled() {
                    break;
                }
            }

            // Send results back to main loop (even partial results on cancellation)
            let _ = result_tx.send(tool_results);
        });
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

        // Update ToolResultBlocks with results
        for result in &tool_results {
            if let Content::ToolResult {
                tool_use_id,
                output,
                ..
            } = result
            {
                // Extract output string
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

                // Find matching ToolResultBlock
                for block in &mut self.blocks.tool_result {
                    if block.tool_use_id() == tool_use_id {
                        block.set_results(output_str);
                        block.complete();
                        break;
                    }
                }

                // Find matching ReadBlock (content set but NOT marked complete yet)
                for block in &mut self.blocks.read {
                    if block.tool_use_id() == tool_use_id {
                        // Parse JSON response from Read tool
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output_str) {
                            let content =
                                json.get("content").and_then(|v| v.as_str()).unwrap_or("");
                            let total_lines =
                                json.get("total_lines")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                            let lines_returned =
                                json.get("lines_returned")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                            block.set_content(content.to_string(), total_lines, lines_returned);
                        } else {
                            // Fallback: treat as plain text
                            let lines: Vec<&str> = output_str.lines().collect();
                            block.set_content(output_str.to_string(), lines.len(), lines.len());
                        }
                        // NOTE: Don't call complete() here - wait for AI response
                        break;
                    }
                }

                // Find matching BashBlock and check for background process result
                for block in &mut self.blocks.bash {
                    if block.tool_use_id() == Some(tool_use_id) {
                        // Check if this is a background process result
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output_str) {
                            if let Some(process_id) = json.get("processId").and_then(|v| v.as_str())
                            {
                                // This is a background process - update block with process ID
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

                // Find matching ExploreBlock and complete it with results
                tracing::info!(
                    explore_blocks = self.blocks.explore.len(),
                    result_tool_id = %tool_use_id,
                    "Looking for matching ExploreBlock"
                );
                for (i, block) in self.blocks.explore.iter_mut().enumerate() {
                    let block_id = block.tool_use_id();
                    tracing::info!(
                        block_index = i,
                        block_tool_id = ?block_id,
                        result_tool_id = %tool_use_id,
                        matches = block_id == Some(tool_use_id.as_str()),
                        "Checking ExploreBlock match"
                    );
                    if block_id == Some(tool_use_id.as_str()) {
                        tracing::info!(
                            tool_use_id = %tool_use_id,
                            output_len = output_str.len(),
                            "Completing ExploreBlock with results"
                        );
                        // Parse the explore tool output to extract agent info
                        // The output format is markdown with agent sections
                        block.complete(output_str.to_string());
                        break;
                    }
                }

                // Find matching BuildBlock and complete it with results
                for block in &mut self.blocks.build {
                    if block.tool_use_id() == Some(tool_use_id.as_str()) {
                        tracing::info!(
                            tool_use_id = %tool_use_id,
                            output_len = output_str.len(),
                            "Completing BuildBlock with results"
                        );
                        block.complete(output_str.to_string());
                        break;
                    }
                }
            }
        }

        // Check if we have queued tools to process (explore finished, other tools waiting)
        let has_queued_tools =
            !self.queued_tools.is_empty() && self.blocks.explore.iter().all(|b| !b.is_streaming());

        if has_queued_tools {
            // Store these results (don't save to conversation yet)
            // We'll combine with queued tool results later
            tracing::info!(
                "Storing {} tool results while processing queued tools",
                tool_results.len()
            );
            self.pending_tool_results.extend(tool_results);

            // Spawn queued tools
            let queued = std::mem::take(&mut self.queued_tools);
            tracing::info!(
                "Processing {} queued tools after explore completion",
                queued.len()
            );
            self.spawn_tool_execution(queued);
            return;
        }

        // Combine with any pending results from explore phase
        let all_results = if !self.pending_tool_results.is_empty() {
            let mut combined = std::mem::take(&mut self.pending_tool_results);
            let pending_count = combined.len();
            let new_count = tool_results.len();
            combined.extend(tool_results);
            tracing::info!(
                "Combining {} pending + {} new = {} total tool results",
                pending_count,
                new_count,
                combined.len()
            );
            combined
        } else {
            tool_results
        };

        // Add all tool results to conversation as ONE message
        let tool_result_msg = ModelMessage {
            role: Role::User,
            content: all_results,
        };

        tracing::info!(
            "SAVING combined tool_result message with {} results to conversation",
            tool_result_msg.content.len()
        );
        self.conversation.push(tool_result_msg.clone());
        self.save_model_message(&tool_result_msg);

        // Continue conversation with AI
        self.send_to_ai();
    }

    /// Build diagnostics context for AI from LSP
    pub fn build_diagnostics_context(&self) -> String {
        let cache = self.lsp_manager.diagnostics_cache();
        let error_count = cache.error_count();
        let warning_count = cache.warning_count();

        if error_count == 0 && warning_count == 0 {
            return String::new();
        }

        let diagnostics_str = cache.format_for_display();

        format!(
            "[SYSTEM CONTEXT] Current LSP Diagnostics ({} errors, {} warnings):\n{}",
            error_count, warning_count, diagnostics_str
        )
    }

    /// Build plan context for AI - shown in both PLAN and BUILD modes when a plan is active
    pub fn build_plan_context(&self) -> String {
        use crate::tui::app::WorkMode;

        match self.work_mode {
            WorkMode::Plan => {
                let Some(plan) = &self.active_plan else {
                    // In plan mode but no active plan - provide instructions with format
                    return r#"[PLAN MODE ACTIVE]

You are in PLAN MODE. The user wants to create a plan before implementing.

In plan mode:
- You can READ files, search code, and explore the codebase
- You CANNOT write, edit, or create files
- You CANNOT run modifying bash commands (git commit, rm, mv, etc.)
- Focus on understanding the codebase and designing an implementation approach

IMPORTANT: When requirements are ambiguous or you need clarification, use the AskUserQuestion tool instead of asking in plain text. This provides a better UX with clickable options.

When creating a plan, use this EXACT format (the system will auto-detect and save it):

```
# Plan: [Title]

## Phase 1: [Phase Name]

- [ ] Task description here
- [ ] Another task

## Phase 2: [Phase Name]

- [ ] Task description
```

Key formatting rules:
- Title line: `# Plan: Your Title Here`
- Phase headers: `## Phase N: Phase Name`
- Tasks: `- [ ] Description` (unchecked) or `- [x] Description` (completed)

After exploring the codebase, output your plan in this format. The user can exit plan mode with Ctrl+B to begin implementation."#.to_string();
                };

                // Build context from active plan (truncated if large)
                let (completed, total) = plan.progress();
                let markdown = plan.to_context();

                format!(
                    r#"[PLAN MODE ACTIVE - Plan: "{}"]

Progress: {}/{} tasks completed

In plan mode:
- You can READ files, search code, and explore the codebase
- You CANNOT write, edit, or create files until plan mode is exited
- Focus on the current plan and track progress
- Use the AskUserQuestion tool for clarifications (not plain text questions)

## Current Plan

{}

---

When working on tasks, update progress by telling the user which task you're working on.
The user can exit plan mode with Ctrl+B when ready to implement."#,
                    sanitize_plan_title(&plan.title),
                    completed,
                    total,
                    markdown
                )
            }
            WorkMode::Build => {
                // In BUILD mode, if there's an active plan, show it with execution instructions
                let Some(plan) = &self.active_plan else {
                    return String::new();
                };

                let (completed, total) = plan.progress();
                let markdown = plan.to_context();

                format!(
                    r#"[ACTIVE PLAN - "{}"]

Progress: {}/{} tasks completed

## Current Plan

{}

---

## Task Management

Mark tasks complete silently - the plan sidebar updates automatically. NO announcements needed.

- Single: `task_complete(task_id: "1.1")`
- Batch: `task_complete(task_ids: ["1.1", "1.2", "2.1"])`

Workflow: Do the work → Call task_complete → Continue."#,
                    sanitize_plan_title(&plan.title),
                    completed,
                    total,
                    markdown
                )
            }
        }
    }

    /// Build skills context for AI - lists available skills with metadata only
    ///
    /// Uses progressive disclosure: only names/descriptions in system prompt,
    /// AI can invoke the skill tool to load full content when needed.
    pub fn build_skills_context(&self) -> String {
        // Get skill infos - needs write lock as list_skills may refresh cache
        let mut skills_guard = match self.skills_manager.try_write() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Skills manager locked, skipping skills context");
                return String::new();
            }
        };

        let skills = skills_guard.list_skills();
        if skills.is_empty() {
            return String::new();
        }

        let mut context = String::from("[AVAILABLE SKILLS]\n\n");
        context.push_str("The following skills are available. Use the `skill` tool to invoke a skill and get detailed instructions.\n\n");

        for info in skills {
            context.push_str(&format!("- **{}**: {}\n", info.name, info.description));
            if !info.tags.is_empty() {
                context.push_str(&format!("  Tags: {}\n", info.tags.join(", ")));
            }
        }

        context.push_str("\nTo use a skill: `skill(skill: \"skill-name\")`\n");
        context
    }

    /// Build project context from instruction files.
    ///
    /// Reads project-specific instructions from the working directory.
    /// These files provide context about the codebase, conventions, and guidelines.
    pub fn build_project_context(&self) -> String {
        // Support common AI coding assistant instruction file formats
        const PROJECT_FILES: &[&str] = &[
            "KRAB.md",
            "krab.md",
            "AGENTS.md",
            "agents.md",
            "CLAUDE.md",
            "claude.md",
            ".cursorrules",
            ".windsurfrules",
            ".clinerules",
            ".github/copilot-instructions.md",
            "JULES.md",
            "gemini.md",
        ];
        for filename in PROJECT_FILES {
            let path = self.working_dir.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                tracing::debug!(
                    "Loaded project context from {} ({} chars)",
                    filename,
                    content.len()
                );
                return format!(
                    "[PROJECT INSTRUCTIONS - {}]\n\n{}\n\n[END PROJECT INSTRUCTIONS]",
                    filename, content
                );
            }
        }
        String::new()
    }
}
