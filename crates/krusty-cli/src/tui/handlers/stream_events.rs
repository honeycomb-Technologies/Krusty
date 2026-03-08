//! Stream event handlers
//!
//! Processes orchestrator loop events and updates application state.
//! The core orchestrator handles the agentic cycle (stream -> tools -> repeat)
//! and emits LoopEvents that this module translates to TUI visual state.

use crate::agent::loop_events::LoopEvent;
use crate::agent::AgentEvent;
use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
use crate::plan::PlanFile;
use crate::tui::app::{App, WorkMode};
use crate::tui::blocks::{StreamBlock, WebSearchBlock};

impl App {
    fn append_streaming_assistant_delta(&mut self, delta: String) {
        // Use cached streaming assistant index (O(1)) instead of O(n) scan per delta.
        let append_idx = if let Some(idx) = self.runtime.chat.streaming_assistant_idx {
            if idx < self.runtime.chat.messages.len()
                && self
                    .runtime
                    .chat
                    .messages
                    .get(idx)
                    .map(|(role, _)| role == "assistant")
                    .unwrap_or(false)
            {
                Some(idx)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(idx) = append_idx {
            self.runtime.chat.streaming_assistant_idx = Some(idx);
            if let Some((_, content)) = self.runtime.chat.messages.get_mut(idx) {
                content.push_str(&delta);
            }
        } else {
            // Create new assistant message at end and cache its index
            let new_idx = self.runtime.chat.messages.len();
            self.runtime
                .chat
                .messages
                .push(("assistant".to_string(), delta));
            self.runtime.chat.streaming_assistant_idx = Some(new_idx);
        }
    }

    // =========================================================================
    // Core Orchestrator LoopEvent Consumer
    // =========================================================================

    /// Process all pending events from the core orchestrator loop.
    ///
    /// The orchestrator handles the entire agentic cycle (stream -> tools -> repeat)
    /// and emits LoopEvents for every state change. This method translates those
    /// events to TUI visual state (blocks, messages, prompts).
    ///
    /// Returns true if any events were processed.
    pub fn process_loop_events(&mut self) -> bool {
        let Some(mut rx) = self.runtime.channels.loop_events.take() else {
            return false;
        };

        let mut processed_any = false;
        let mut events = Vec::new();
        let mut disconnected = false;

        // Drain available events into a local buffer (avoids borrow conflict)
        loop {
            match rx.try_recv() {
                Ok(event) => events.push(event),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        // Put receiver back unless disconnected
        if !disconnected {
            self.runtime.channels.loop_events = Some(rx);
        }

        // Process all buffered events
        for event in events {
            processed_any = true;
            self.handle_loop_event(event);
        }

        processed_any
    }

    /// Handle a single orchestrator LoopEvent
    fn handle_loop_event(&mut self, event: LoopEvent) {
        match event {
            // -- Streaming ------------------------------------------------
            LoopEvent::TextDelta { delta } => {
                self.handle_text_delta(delta);
            }
            LoopEvent::TextDeltaWithCitations { delta, citations } => {
                self.handle_text_delta_with_citations(delta, citations);
            }
            LoopEvent::ThinkingDelta { thinking } => {
                // Create ThinkingBlock on first delta (orchestrator doesn't emit ThinkingStart)
                let needs_block = self
                    .runtime
                    .blocks
                    .thinking
                    .last()
                    .map(|b| !b.is_streaming())
                    .unwrap_or(true);
                if needs_block {
                    self.handle_thinking_start();
                }
                self.handle_thinking_delta(thinking);
            }
            LoopEvent::ThinkingComplete {
                thinking: _,
                signature,
            } => {
                self.handle_thinking_complete(signature);
            }

            // -- Tool lifecycle -------------------------------------------
            LoopEvent::ToolCallStart { id: _, name } => {
                self.handle_tool_start(name);
            }
            LoopEvent::ToolCallComplete {
                id,
                name,
                arguments,
            } => {
                // Store AskUser calls for when AwaitingInput arrives
                if name == "AskUserQuestion" {
                    self.runtime.pending_ask_user_calls.push(AiToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    });
                }
                // Create visual blocks from completed tool call
                let tool_call = AiToolCall {
                    id,
                    name,
                    arguments,
                };
                self.create_tool_blocks(&[tool_call]);
            }
            LoopEvent::ToolExecuting { id: _, name: _ } => {
                if !self.runtime.chat.is_executing_tools {
                    self.start_tool_execution();
                }
            }
            LoopEvent::ToolOutputDelta { id, delta } => {
                // Route streaming output to the matching BashBlock
                for block in &mut self.runtime.blocks.bash {
                    if block.tool_use_id() == Some(&id) {
                        block.append(&delta);
                        break;
                    }
                }
                if self.ui.scroll_system.scroll.auto_scroll {
                    self.ui.scroll_system.scroll.request_scroll_to_bottom();
                }
            }
            LoopEvent::ToolResult {
                id,
                output,
                is_error: _,
            } => {
                self.update_tool_result_block(&id, &output);
                self.update_read_block(&id, &output);
                self.update_bash_block(&id, &output);
                self.update_explore_block(&id, &output);
                self.update_build_block(&id, &output);
            }

            // -- Interaction ----------------------------------------------
            LoopEvent::ToolApprovalRequired {
                id,
                name,
                arguments: _,
            } => {
                let names = vec![name];
                let ids = vec![id];
                self.ui.decision_prompt.show_tool_approval(names, ids);
                self.runtime.approval_requested_at = Some(std::time::Instant::now());
            }
            LoopEvent::ToolApproved { id } => {
                tracing::info!("Tool approved: {}", id);
            }
            LoopEvent::ToolDenied { id } => {
                tracing::info!("Tool denied: {}", id);
            }
            LoopEvent::AwaitingInput {
                tool_call_id,
                tool_name,
            } => {
                if tool_name == "AskUserQuestion" {
                    if let Some(call) = self
                        .runtime
                        .pending_ask_user_calls
                        .iter()
                        .find(|c| c.id == tool_call_id)
                        .cloned()
                    {
                        self.handle_ask_user_question_tools(vec![call]);
                    }
                } else if tool_name == "PlanConfirm" {
                    tracing::info!("Plan confirmation awaiting input: {}", tool_call_id);
                }
            }

            // -- Server-side tools ----------------------------------------
            LoopEvent::ServerToolStart { id, name } => {
                self.handle_server_tool_start(id, name);
            }
            LoopEvent::ServerToolComplete { id, name } => {
                tracing::info!("Server tool completed: {} ({})", name, id);
            }
            LoopEvent::WebSearchResults {
                tool_use_id,
                results,
            } => {
                self.handle_web_search_results(tool_use_id, results);
            }
            LoopEvent::WebFetchResult {
                tool_use_id,
                content,
            } => {
                self.handle_web_fetch_result(tool_use_id, content);
            }
            LoopEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => {
                tracing::warn!("Server tool error: {} ({})", error_code, tool_use_id);
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!("Web tool error: {}", error_code),
                ));
            }

            // -- Mode + Plan ----------------------------------------------
            LoopEvent::ModeChange { mode, reason } => {
                let new_mode = match mode.as_str() {
                    "plan" | "Plan" => WorkMode::Plan,
                    _ => WorkMode::Build,
                };
                self.ui.work_mode = new_mode;
                if let Some(reason) = reason {
                    tracing::info!("Mode changed to {:?}: {}", new_mode, reason);
                }
            }
            LoopEvent::PlanUpdate { tasks } => {
                tracing::info!("Plan updated: {} tasks", tasks.len());
            }
            LoopEvent::PlanComplete {
                tool_call_id: _,
                title,
                task_count,
            } => {
                // Reload plan from DB and show confirmation
                if let Some(session_id) = self.runtime.current_session_id.clone() {
                    if let Some(ref pm) = self.services.plan_manager {
                        if let Ok(Some(plan)) = pm.get_plan(&session_id) {
                            self.set_plan(plan);
                            self.ui.work_mode = WorkMode::Plan;
                            if !self.ui.plan_sidebar.visible {
                                self.ui.plan_sidebar.toggle();
                            }
                            self.ui
                                .decision_prompt
                                .show_plan_confirm(&title, task_count);
                        }
                    }
                }
            }

            // -- Turn lifecycle -------------------------------------------
            LoopEvent::TurnComplete { turn, has_more } => {
                self.runtime.agent_state.current_turn = turn;
                if !has_more {
                    self.stop_tool_execution();
                }
            }
            LoopEvent::Usage {
                prompt_tokens,
                completion_tokens,
            } => {
                self.runtime.context_tokens_used = prompt_tokens + completion_tokens;
                self.save_session_token_count();
            }
            LoopEvent::TitleGenerated { title } => {
                self.runtime.session_title = Some(title);
            }
            LoopEvent::Finished { session_id } => {
                tracing::info!("Orchestrator finished for session {}", session_id);
                self.reload_conversation_from_db();
                self.stop_streaming();
                self.stop_tool_execution();
                self.runtime.channels.loop_events = None;
                self.runtime.channels.loop_input = None;
                self.runtime.pending_ask_user_calls.clear();
                self.check_auto_pinch();
            }
            LoopEvent::Error { error } => {
                self.handle_stream_error(error);
            }
        }
    }

    /// Reload the conversation from the database.
    ///
    /// Called when the orchestrator finishes to sync the TUI's in-memory
    /// conversation with what the orchestrator saved to the DB.
    fn reload_conversation_from_db(&mut self) {
        let Some(session_id) = &self.runtime.current_session_id else {
            return;
        };
        let Some(sm) = &self.services.session_manager else {
            return;
        };

        match sm.load_session_messages(session_id) {
            Ok(messages) => {
                self.runtime.chat.conversation.clear();
                for (role, content_json) in messages {
                    let api_role = match role.as_str() {
                        "user" => Role::User,
                        "assistant" => Role::Assistant,
                        "system" => Role::System,
                        _ => Role::User,
                    };
                    let content: Vec<Content> = serde_json::from_str::<Vec<Content>>(&content_json)
                        .or_else(|_| {
                            serde_json::from_str::<Content>(&content_json).map(|c| vec![c])
                        })
                        .unwrap_or_else(|_| vec![Content::Text { text: content_json }]);
                    self.runtime.chat.conversation.push(ModelMessage {
                        role: api_role,
                        content,
                    });
                }
                tracing::info!(
                    "Reloaded {} conversation messages from DB",
                    self.runtime.chat.conversation.len()
                );
            }
            Err(e) => {
                tracing::warn!("Failed to reload conversation from DB: {}", e);
            }
        }
    }

    // =========================================================================
    // Text Handling
    // =========================================================================

    /// Handle text delta from AI response
    fn handle_text_delta(&mut self, delta: String) {
        // Mark all streaming blocks complete when AI starts responding
        self.complete_streaming_blocks();

        // Check for task completion keywords in delta for real-time updates
        const COMPLETION_KEYWORDS: &[&str] = &[
            "complete", "Complete", "done", "Done", "finished", "Finished", "\u{2713}", "\u{2705}",
        ];
        let should_check_completion = self.runtime.active_plan.is_some()
            && COMPLETION_KEYWORDS.iter().any(|kw| delta.contains(kw));

        // Cache is cleared at the start of each new streaming session (start_streaming),
        // so a None cache means this is the first text delta of a new turn.
        self.append_streaming_assistant_delta(delta);

        // Real-time task completion detection
        if should_check_completion {
            // Clone last message content to avoid borrow issues
            let check_text = self.runtime.chat.messages.last().map(|(_, content)| {
                if content.len() > 500 {
                    // Find a valid char boundary near the target position
                    // to avoid panicking on multi-byte UTF-8 characters
                    let target = content.len() - 500;
                    let start = content
                        .char_indices()
                        .rev()
                        .find(|(i, _)| *i <= target)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    content[start..].to_string()
                } else {
                    content.clone()
                }
            });
            if let Some(text) = check_text {
                self.try_update_task_completions_realtime(&text);
            }
        }

        if self.ui.scroll_system.scroll.auto_scroll {
            self.ui.scroll_system.scroll.request_scroll_to_bottom();
        }
    }

    /// Handle text delta with citations
    fn handle_text_delta_with_citations(
        &mut self,
        delta: String,
        citations: Vec<crate::ai::types::Citation>,
    ) {
        self.append_streaming_assistant_delta(delta);

        if !citations.is_empty() {
            tracing::info!("Received {} citations", citations.len());
            for cite in &citations {
                tracing::debug!("  Citation: {} - {}", cite.title, cite.url);
            }
        }

        if self.ui.scroll_system.scroll.auto_scroll {
            self.ui.scroll_system.scroll.request_scroll_to_bottom();
        }
    }

    // =========================================================================
    // Tool Handling
    // =========================================================================

    /// Handle tool start event
    fn handle_tool_start(&mut self, name: String) {
        // Mark all streaming blocks complete when tool starts
        self.complete_streaming_blocks();

        // Invalidate streaming assistant index cache since tool blocks will insert
        // new messages, making the cached index stale
        self.runtime.chat.streaming_assistant_idx = None;

        // Create pending blocks for edit/write tools
        if name == "edit" {
            self.runtime
                .blocks
                .edit
                .push(crate::tui::blocks::EditBlock::new_pending(
                    "...".to_string(),
                ));
            if let Some(block) = self.runtime.blocks.edit.last_mut() {
                block.set_diff_mode(self.runtime.blocks.diff_mode);
            }
            self.runtime
                .chat
                .messages
                .push(("edit".to_string(), String::new()));
        }

        if name == "write" {
            self.runtime
                .blocks
                .write
                .push(crate::tui::blocks::WriteBlock::new_pending(
                    "...".to_string(),
                ));
            self.runtime
                .chat
                .messages
                .push(("write".to_string(), String::new()));
        }

        if name == "Task" || name == "explore" {
            tracing::info!(
                "handle_tool_start: explore tool '{}' detected, block will be created on execution",
                name
            );
        }

        // Skip tool message for tools with custom blocks or silent utility tools
        if !matches!(
            name.as_str(),
            "bash"
                | "grep"
                | "glob"
                | "read"
                | "edit"
                | "write"
                | "processes"
                | "Task"
                | "explore"
                | "build"
                | "AskUserQuestion"
                | "task_start"         // Silent - updates plan sidebar
                | "task_complete"      // Silent - updates plan sidebar
                | "add_subtask"        // Silent - updates plan sidebar
                | "set_dependency"     // Silent - updates plan sidebar
                | "enter_plan_mode"    // Silent - updates status bar
                | "set_work_mode" // Silent - updates status bar
        ) {
            self.runtime
                .chat
                .messages
                .push(("tool".to_string(), format!("Using tool: {} ...", name)));
        }

        // Special loading message for AskUserQuestion
        if name == "AskUserQuestion" {
            self.runtime
                .chat
                .messages
                .push(("tool".to_string(), "Preparing questions...".to_string()));
        }
    }

    // =========================================================================
    // Thinking Handling
    // =========================================================================

    /// Handle thinking start event
    fn handle_thinking_start(&mut self) {
        // Mark all streaming blocks complete
        self.complete_streaming_blocks();

        // Create a new ThinkingBlock
        self.runtime
            .blocks
            .thinking
            .push(crate::tui::blocks::ThinkingBlock::new());
        self.runtime
            .chat
            .messages
            .push(("thinking".to_string(), String::new()));
    }

    /// Handle thinking delta event
    fn handle_thinking_delta(&mut self, thinking: String) {
        if let Some(block) = self.runtime.blocks.thinking.last_mut() {
            block.append(&thinking);
        }
    }

    /// Handle thinking complete event
    fn handle_thinking_complete(&mut self, signature: String) {
        let signature_len = signature.len();
        if let Some(block) = self.runtime.blocks.thinking.last_mut() {
            block.set_signature(signature);
            block.complete();
        }
        tracing::info!("ThinkingComplete - signature_len={}", signature_len);
    }

    // =========================================================================
    // Error Handling
    // =========================================================================

    /// Handle stream error event
    fn handle_stream_error(&mut self, error: String) {
        self.runtime.event_bus.emit(AgentEvent::StreamError {
            error: error.clone(),
        });

        self.stop_streaming();
        self.runtime.agent_state.interrupt();
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), format!("Error: {}", error)));

        // If last message was a tool_result, add error assistant message
        let needs_assistant = self
            .runtime
            .chat
            .conversation
            .last()
            .map(|msg| {
                msg.role == Role::User
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::ToolResult { .. }))
            })
            .unwrap_or(false);

        if needs_assistant {
            tracing::debug!("Adding error assistant message after stream error");
            let assistant_msg = ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: format!("[Error: {}]", error),
                }],
            };
            self.runtime.chat.conversation.push(assistant_msg);
            if let Some(saved_msg) = self.runtime.chat.conversation.last() {
                self.save_model_message(saved_msg);
            }
        }
    }

    // =========================================================================
    // Web Tools
    // =========================================================================

    /// Handle server tool start (web_search, web_fetch)
    fn handle_server_tool_start(&mut self, tool_use_id: String, name: String) {
        // For web_search, create a WebSearchBlock
        // Query will be empty initially - we don't have it until results come back
        if name == "web_search" {
            let block = WebSearchBlock::new(tool_use_id, String::new());
            self.runtime.blocks.web_search.push(block);
            self.runtime
                .chat
                .messages
                .push(("web_search".to_string(), String::new()));
        }
        // web_fetch doesn't need a block - results go inline
    }

    /// Handle web search results
    fn handle_web_search_results(
        &mut self,
        tool_use_id: String,
        results: Vec<crate::ai::types::WebSearchResult>,
    ) {
        tracing::info!(
            "Web search returned {} results ({})",
            results.len(),
            tool_use_id
        );

        // Find matching WebSearchBlock and update it
        if let Some(block) = self
            .runtime
            .blocks
            .web_search
            .iter_mut()
            .find(|b| b.tool_use_id() == tool_use_id)
        {
            block.set_results(results);
        }
    }

    /// Handle web fetch result
    fn handle_web_fetch_result(
        &mut self,
        tool_use_id: String,
        content: crate::ai::types::WebFetchContent,
    ) {
        tracing::info!("Web fetch completed: {} ({})", content.url, tool_use_id);
        let title = content.title.as_deref().unwrap_or("page");
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), format!("Fetched: {}", title)));
    }

    // =========================================================================
    // Plan Tracking
    // =========================================================================

    /// Real-time task completion detection during streaming
    ///
    /// Called when completion keywords are detected in text deltas.
    /// Only updates tasks that haven't already been marked complete.
    fn try_update_task_completions_realtime(&mut self, text: &str) {
        let completed_ids = PlanFile::extract_completed_task_ids(text);
        if completed_ids.is_empty() {
            return;
        }

        let active_plan = match self.runtime.active_plan.as_mut() {
            Some(plan) => plan,
            None => return,
        };

        let mut updated_any = false;
        let mut updated_tasks: Vec<String> = Vec::new();

        for task_id in &completed_ids {
            // Only update if task exists and isn't already complete
            if let Some(task) = active_plan.find_task(task_id) {
                if !task.completed && active_plan.check_task(task_id) {
                    tracing::info!("Real-time: Marked task {} as complete", task_id);
                    updated_tasks.push(task_id.clone());
                    updated_any = true;
                }
            }
        }

        if updated_any {
            let (completed, total) = active_plan.progress();
            let plan_complete = active_plan.is_complete();
            let plan_title = active_plan.title.clone();

            if plan_complete {
                active_plan.status = crate::plan::PlanStatus::Completed;
            }

            // Save immediately for real-time persistence
            if let Some(ref pm) = self.services.plan_manager {
                if let Err(e) = pm.save_plan(active_plan) {
                    tracing::warn!("Failed to save plan after real-time task update: {}", e);
                }
            }

            // Show inline feedback
            let task_list = updated_tasks.join(", ");
            self.runtime.chat.messages.push((
                "system".to_string(),
                format!(
                    "\u{2713} Task {} complete ({}/{})",
                    task_list, completed, total
                ),
            ));

            if plan_complete {
                tracing::info!("Plan '{}' completed (real-time detection)", plan_title);
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "\u{1f389} Plan '{}' complete! All {} tasks finished.",
                        plan_title, total
                    ),
                ));
                self.ui.plan_sidebar.start_collapse();
            }
        }
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Mark all streaming blocks as complete
    fn complete_streaming_blocks(&mut self) {
        for rb in &mut self.runtime.blocks.read {
            if rb.is_streaming() {
                rb.complete();
            }
        }
        for eb in &mut self.runtime.blocks.edit {
            if eb.is_streaming() {
                eb.complete();
            }
        }
        for wb in &mut self.runtime.blocks.write {
            if wb.is_streaming() {
                wb.complete();
            }
        }
        for ws in &mut self.runtime.blocks.web_search {
            if ws.is_streaming() {
                ws.complete();
            }
        }
    }
}
