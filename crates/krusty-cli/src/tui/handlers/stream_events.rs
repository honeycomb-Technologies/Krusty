//! Stream event handlers
//!
//! Processes streaming events from the AI and updates application state.
//! Extracted from app.rs to reduce its size and improve organization.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::agent::AgentEvent;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::plan::PlanFile;
use crate::tui::app::{App, WorkMode};
use crate::tui::blocks::{StreamBlock, WebSearchBlock};
use crate::tui::streaming::StreamEvent;

// ============================================================================
// Static regex patterns for plan abandonment detection (compiled once)
// Patterns are intentionally specific to avoid false positives on discussions
// ============================================================================

/// Pattern: Explicit action - "I'm abandoning/stopping the plan", "I will abandon"
/// Requires first-person confirmation to avoid matching hypotheticals
static RE_ABANDON: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:I'?m|I\s+will|I'll)\s+(?:now\s+)?(?:abandon|stop|cancel|clear|end)(?:ing)?\s+(?:the\s+)?(?:current\s+)?plan\b").unwrap()
});

/// Pattern: Confirmed past tense - "plan has been abandoned/stopped"
/// Past tense indicates action completed, not hypothetical
static RE_STOPPED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:the\s+)?(?:current\s+)?plan\s+(?:has\s+been|is\s+now|was)\s+(?:abandoned|stopped|cancelled|cleared|ended)\b").unwrap()
});

/// Pattern: Acknowledged request - "okay, stopping the plan", "understood, abandoning"
/// Requires acknowledgment word to confirm intent
static RE_ACKNOWLEDGED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:okay|ok|understood|sure|alright|very\s+well)[,.]?\s+(?:I'?m\s+)?(?:abandon|stop|cancel|clear|end)(?:ing|ed)?\s+(?:the\s+)?(?:current\s+)?plan\b").unwrap()
});

impl App {
    /// Process all pending stream events from the StreamingManager
    ///
    /// This is the main streaming event loop that handles all StreamEvent variants.
    /// Returns true if any events were processed.
    ///
    /// IMPORTANT: Processing is paused when LSP install popup is shown.
    /// This allows the popup to interrupt the conversation and wait for user input.
    pub fn process_stream_events(&mut self) -> bool {
        // Pause streaming while LSP popup is shown (waiting for user decision)
        if self.ui.popup == crate::tui::app::Popup::LspInstall {
            return false;
        }

        let mut processed_any = false;

        while let Some(event) = self.streaming.poll() {
            processed_any = true;
            self.handle_stream_event(event);
        }

        processed_any
    }

    /// Handle a single stream event
    fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::TextDelta { delta } => {
                self.handle_text_delta(delta);
            }
            StreamEvent::TextDeltaWithCitations { delta, citations } => {
                self.handle_text_delta_with_citations(delta, citations);
            }
            StreamEvent::ToolStart { name } => {
                self.handle_tool_start(name);
            }
            StreamEvent::ToolDelta => {
                // Currently ignored - tool deltas not displayed
            }
            StreamEvent::ToolComplete { call } => {
                tracing::info!(
                    "StreamEvent::ToolComplete - id={}, name={}",
                    call.id,
                    call.name
                );
            }
            StreamEvent::Finished { reason } => {
                self.handle_stream_finished(reason);
            }
            StreamEvent::Complete { text } => {
                self.handle_stream_complete(text);
            }
            StreamEvent::Error { error } => {
                self.handle_stream_error(error);
            }
            StreamEvent::Usage {
                prompt_tokens,
                completion_tokens,
                cache_read_tokens,
                cache_created_tokens,
            } => {
                self.handle_usage(
                    prompt_tokens,
                    completion_tokens,
                    cache_read_tokens,
                    cache_created_tokens,
                );
            }
            StreamEvent::ContextEdited {
                cleared_tokens,
                cleared_tool_uses,
                cleared_thinking_turns,
            } => {
                self.handle_context_edited(
                    cleared_tokens,
                    cleared_tool_uses,
                    cleared_thinking_turns,
                );
            }
            StreamEvent::ThinkingStart => {
                self.handle_thinking_start();
            }
            StreamEvent::ThinkingDelta { thinking } => {
                self.handle_thinking_delta(thinking);
            }
            StreamEvent::ThinkingComplete { signature } => {
                self.handle_thinking_complete(signature);
            }
            StreamEvent::ServerToolStart { id, name } => {
                tracing::info!("Server tool started: {} ({})", name, id);
                self.handle_server_tool_start(id, name);
            }
            StreamEvent::ServerToolDelta => {
                // Currently ignored
            }
            StreamEvent::ServerToolComplete { id, name } => {
                tracing::info!("Server tool completed: {} ({})", name, id);
            }
            StreamEvent::WebSearchResults {
                tool_use_id,
                results,
            } => {
                self.handle_web_search_results(tool_use_id, results);
            }
            StreamEvent::WebFetchResult {
                tool_use_id,
                content,
            } => {
                self.handle_web_fetch_result(tool_use_id, content);
            }
            StreamEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => {
                tracing::warn!("Server tool error: {} ({})", error_code, tool_use_id);
                self.chat.messages.push((
                    "system".to_string(),
                    format!("Web tool error: {}", error_code),
                ));
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
            "complete", "Complete", "done", "Done", "finished", "Finished", "✓", "✅",
        ];
        let should_check_completion =
            self.active_plan.is_some() && COMPLETION_KEYWORDS.iter().any(|kw| delta.contains(kw));

        // Use cached streaming assistant index (O(1)) instead of O(n) scan per delta
        let should_append = if let Some(idx) = self.chat.streaming_assistant_idx {
            // Verify cache is still valid: idx is in bounds and is the last message
            idx == self.chat.messages.len().saturating_sub(1)
                && self
                    .chat
                    .messages
                    .get(idx)
                    .map(|(role, _)| role == "assistant")
                    .unwrap_or(false)
        } else {
            // Cache miss - find the last assistant message
            let last_assistant_idx = self
                .chat
                .messages
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (role, _))| role == "assistant")
                .map(|(idx, _)| idx);

            if let Some(idx) = last_assistant_idx {
                if idx == self.chat.messages.len() - 1 {
                    // Cache it for subsequent deltas
                    self.chat.streaming_assistant_idx = Some(idx);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_append {
            if let Some((_, content)) = self.chat.messages.last_mut() {
                content.push_str(&delta);
            }
        } else {
            // Create new assistant message at end and cache its index
            let new_idx = self.chat.messages.len();
            self.chat.messages.push(("assistant".to_string(), delta));
            self.chat.streaming_assistant_idx = Some(new_idx);
        }

        // Real-time task completion detection
        if should_check_completion {
            // Clone last message content to avoid borrow issues
            let check_text = self.chat.messages.last().map(|(_, content)| {
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

        if self.scroll_system.scroll.auto_scroll {
            self.scroll_system.scroll.request_scroll_to_bottom();
        }
    }

    /// Handle text delta with citations
    fn handle_text_delta_with_citations(
        &mut self,
        delta: String,
        citations: Vec<crate::ai::types::Citation>,
    ) {
        // Same logic as TextDelta - append only if last msg is assistant
        let last_is_assistant = self
            .chat
            .messages
            .last()
            .map(|(role, _)| role == "assistant")
            .unwrap_or(false);

        if last_is_assistant {
            if let Some((_, content)) = self.chat.messages.last_mut() {
                content.push_str(&delta);
            }
        } else {
            self.chat.messages.push(("assistant".to_string(), delta));
        }

        if !citations.is_empty() {
            tracing::info!("Received {} citations", citations.len());
            for cite in &citations {
                tracing::debug!("  Citation: {} - {}", cite.title, cite.url);
            }
        }

        if self.scroll_system.scroll.auto_scroll {
            self.scroll_system.scroll.request_scroll_to_bottom();
        }
    }

    // =========================================================================
    // Tool Handling
    // =========================================================================

    /// Handle tool start event
    fn handle_tool_start(&mut self, name: String) {
        // Mark all streaming blocks complete when tool starts
        self.complete_streaming_blocks();

        // Create pending blocks for edit/write tools
        if name == "edit" {
            self.blocks
                .edit
                .push(crate::tui::blocks::EditBlock::new_pending(
                    "...".to_string(),
                ));
            if let Some(block) = self.blocks.edit.last_mut() {
                block.set_diff_mode(self.blocks.diff_mode);
            }
            self.chat.messages.push(("edit".to_string(), String::new()));
        }

        if name == "write" {
            self.blocks
                .write
                .push(crate::tui::blocks::WriteBlock::new_pending(
                    "...".to_string(),
                ));
            self.chat
                .messages
                .push(("write".to_string(), String::new()));
        }

        // NOTE: ExploreBlock is created in spawn_tool_execution where we have the tool_use_id
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
                | "enter_plan_mode" // Silent - updates status bar
        ) {
            self.chat
                .messages
                .push(("tool".to_string(), format!("Using tool: {} ...", name)));
        }

        // Special loading message for AskUserQuestion
        if name == "AskUserQuestion" {
            self.chat
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
        self.blocks
            .thinking
            .push(crate::tui::blocks::ThinkingBlock::new());
        self.chat
            .messages
            .push(("thinking".to_string(), String::new()));
    }

    /// Handle thinking delta event
    fn handle_thinking_delta(&mut self, thinking: String) {
        if let Some(block) = self.blocks.thinking.last_mut() {
            block.append(&thinking);
        }
    }

    /// Handle thinking complete event
    fn handle_thinking_complete(&mut self, signature: String) {
        if let Some(block) = self.blocks.thinking.last_mut() {
            block.set_signature(signature.clone());
            block.complete();
        }
        tracing::info!("ThinkingComplete - signature_len={}", signature.len());
    }

    // =========================================================================
    // Stream Lifecycle
    // =========================================================================

    /// Handle stream finished event (API done sending)
    fn handle_stream_finished(&mut self, reason: crate::ai::types::FinishReason) {
        let turn_duration = self
            .agent_state
            .turn_duration()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        self.event_bus.emit(AgentEvent::StreamEnd { reason });
        self.event_bus.emit(AgentEvent::TurnComplete {
            turn: self.agent_state.current_turn,
            duration_ms: turn_duration,
            tokens: crate::ai::types::Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: self.context_tokens_used,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        });
    }

    /// Handle stream complete event (channel closed)
    fn handle_stream_complete(&mut self, final_text: String) {
        let turn_duration = self
            .agent_state
            .turn_duration()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        tracing::info!(
            "StreamEvent::Complete - final_text_len={}, phase={}",
            final_text.len(),
            self.streaming.phase_name()
        );

        self.event_bus.emit(AgentEvent::TurnComplete {
            turn: self.agent_state.current_turn,
            duration_ms: turn_duration,
            tokens: crate::ai::types::Usage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: self.context_tokens_used,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        });

        // Check for plan structure in AI response when in plan mode
        if self.ui.work_mode == WorkMode::Plan && !final_text.is_empty() {
            self.try_detect_and_save_plan(&final_text);
        }

        // Check for task completion mentions (works in both modes if plan is active)
        if self.active_plan.is_some() && !final_text.is_empty() {
            tracing::info!(
                "Plan active, checking final_text ({} chars) for completions",
                final_text.len()
            );
            // Log a preview of the text for debugging
            let preview: String = final_text.chars().take(200).collect();
            tracing::debug!("Final text preview: {}...", preview);

            // Check for plan abandonment first
            if self.try_detect_plan_abandonment(&final_text) {
                // Plan was abandoned, skip completion check
            } else {
                self.try_update_task_completions(&final_text);
            }
        } else if self.active_plan.is_none() {
            tracing::debug!("No active plan, skipping task completion check");
        }

        // If no tools to execute, build and save the assistant message now
        if !self.streaming.is_ready_for_tools() {
            self.stop_streaming();

            // Build and save assistant message using StreamingManager
            if let Some(assistant_msg) = self.streaming.build_assistant_message() {
                tracing::info!(
                    "SAVING assistant message with {} content blocks (no tools)",
                    assistant_msg.content.len()
                );
                self.chat.conversation.push(assistant_msg.clone());
                self.save_model_message(&assistant_msg);
            } else {
                // Check if we need a filler message after tool_result
                self.maybe_add_filler_message();
            }

            self.streaming.reset();
        }
        // If ready for tools, the tool execution logic will handle it
    }

    /// Try to detect plan structure in AI response and save it
    fn try_detect_and_save_plan(&mut self, response_text: &str) {
        // Try to parse plan structure from response
        let parsed_plan = match PlanFile::try_parse_from_response(response_text) {
            Some(plan) => plan,
            None => return, // No plan structure found
        };

        tracing::info!(
            "Detected plan in AI response: '{}' with {} phases, {} tasks",
            parsed_plan.title,
            parsed_plan.phases.len(),
            parsed_plan.total_tasks()
        );

        // Get working directory for the plan
        let working_dir = self.working_dir.to_string_lossy().into_owned();
        let session_id = self.current_session_id.clone();

        if let Some(ref mut active_plan) = self.active_plan {
            // Merge into existing plan
            active_plan.merge_from(&parsed_plan);
            tracing::info!(
                "Merged plan updates into existing plan: '{}'",
                active_plan.title
            );

            // Save the updated plan
            if let Err(e) = self.services.plan_manager.save_plan(active_plan) {
                tracing::warn!("Failed to save merged plan: {}", e);
            } else {
                tracing::info!(
                    "Plan saved: {} ({}/{})",
                    active_plan.title,
                    active_plan.completed_tasks(),
                    active_plan.total_tasks()
                );
            }
        } else {
            // Create new plan - requires a session
            let Some(session_id) = session_id else {
                tracing::warn!("Cannot create plan without an active session");
                return;
            };

            match self.services.plan_manager.create_plan(
                &parsed_plan.title,
                &session_id,
                Some(&working_dir),
            ) {
                Ok(mut new_plan) => {
                    // Copy phases from parsed plan
                    new_plan.phases = parsed_plan.phases;
                    new_plan.notes = parsed_plan.notes;

                    // Save with the full content
                    if let Err(e) = self.services.plan_manager.save_plan(&new_plan) {
                        tracing::warn!("Failed to save new plan: {}", e);
                    } else {
                        tracing::info!("Created new plan: '{}'", new_plan.title);
                        let plan_title = new_plan.title.clone();
                        let task_count = new_plan.total_tasks();

                        self.set_plan(new_plan);
                        self.ui.work_mode = WorkMode::Plan;
                        if !self.plan_sidebar.visible {
                            self.plan_sidebar.toggle();
                        }

                        // Show decision prompt for plan confirmation
                        self.decision_prompt
                            .show_plan_confirm(&plan_title, task_count);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to create plan: {}", e);
                }
            }
        }
    }

    /// Try to detect and update task completions from AI response
    fn try_update_task_completions(&mut self, response_text: &str) {
        tracing::debug!(
            "Checking for task completions in {} chars of text",
            response_text.len()
        );

        let completed_ids = PlanFile::extract_completed_task_ids(response_text);
        tracing::debug!("Extracted task IDs: {:?}", completed_ids);

        if completed_ids.is_empty() {
            tracing::debug!("No task completion patterns found");
            return;
        }

        let active_plan = match self.active_plan.as_mut() {
            Some(plan) => plan,
            None => return,
        };

        let mut updated_any = false;
        let mut updated_tasks: Vec<String> = Vec::new();

        for task_id in &completed_ids {
            // Only update if task exists and isn't already complete
            if let Some(task) = active_plan.find_task(task_id) {
                if !task.completed && active_plan.check_task(task_id) {
                    tracing::info!("Marked task {} as complete", task_id);
                    updated_tasks.push(task_id.clone());
                    updated_any = true;
                }
            } else {
                tracing::debug!("Task {} not found in plan", task_id);
            }
        }

        if updated_any {
            let (completed, total) = active_plan.progress();
            let plan_complete = active_plan.is_complete();
            let plan_title = active_plan.title.clone();

            // If plan is complete, mark it as such before saving
            if plan_complete {
                active_plan.status = crate::plan::PlanStatus::Completed;
            }

            // Save the updated plan
            if let Err(e) = self.services.plan_manager.save_plan(active_plan) {
                tracing::warn!("Failed to save plan after task updates: {}", e);
            } else {
                tracing::info!(
                    "Plan progress updated: {}/{} tasks complete",
                    completed,
                    total
                );

                // Show visible feedback to user
                let task_list = updated_tasks.join(", ");
                self.chat.messages.push((
                    "system".to_string(),
                    format!("✓ Task {} complete ({}/{})", task_list, completed, total),
                ));

                // If plan is complete, disengage elegantly with animated collapse
                if plan_complete {
                    tracing::info!(
                        "Plan '{}' completed - starting graceful disengage",
                        plan_title
                    );
                    self.chat.messages.push((
                        "system".to_string(),
                        format!(
                            "🎉 Plan '{}' complete! All {} tasks finished.",
                            plan_title, total
                        ),
                    ));

                    // Start graceful collapse - plan clears when animation completes
                    self.plan_sidebar.start_collapse();
                }
            }
        }
    }

    /// Real-time task completion detection during streaming
    ///
    /// Called when completion keywords are detected in text deltas.
    /// Only updates tasks that haven't already been marked complete.
    fn try_update_task_completions_realtime(&mut self, text: &str) {
        let completed_ids = PlanFile::extract_completed_task_ids(text);
        if completed_ids.is_empty() {
            return;
        }

        let active_plan = match self.active_plan.as_mut() {
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
            if let Err(e) = self.services.plan_manager.save_plan(active_plan) {
                tracing::warn!("Failed to save plan after real-time task update: {}", e);
            }

            // Show inline feedback
            let task_list = updated_tasks.join(", ");
            self.chat.messages.push((
                "system".to_string(),
                format!("✓ Task {} complete ({}/{})", task_list, completed, total),
            ));

            if plan_complete {
                tracing::info!("Plan '{}' completed (real-time detection)", plan_title);
                self.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "🎉 Plan '{}' complete! All {} tasks finished.",
                        plan_title, total
                    ),
                ));
                self.plan_sidebar.start_collapse();
            }
        }
    }

    /// Detect if AI is abandoning/stopping the plan via natural language
    /// Returns true if plan was abandoned
    /// Uses pre-compiled static regexes for performance.
    fn try_detect_plan_abandonment(&mut self, response_text: &str) -> bool {
        // Check all abandonment patterns using pre-compiled static regexes
        if RE_ABANDON.is_match(response_text)
            || RE_STOPPED.is_match(response_text)
            || RE_ACKNOWLEDGED.is_match(response_text)
        {
            if let Some(ref mut plan) = self.active_plan {
                plan.status = crate::plan::PlanStatus::Abandoned;
                let title = plan.title.clone();
                let file_path = plan
                    .file_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();

                if let Err(e) = self.services.plan_manager.save_plan(plan) {
                    tracing::warn!("Failed to save abandoned plan: {}", e);
                }

                tracing::info!("Plan '{}' abandoned via natural language", title);

                // Show abandonment with file path for reference
                let msg = if file_path.is_empty() {
                    format!("Plan '{}' abandoned.", title)
                } else {
                    format!("Plan '{}' abandoned. Saved at: {}", title, file_path)
                };
                self.chat.messages.push(("system".to_string(), msg));
                self.clear_plan();
                return true;
            }
        }

        false
    }

    /// Handle stream error event
    fn handle_stream_error(&mut self, error: String) {
        self.event_bus.emit(AgentEvent::StreamError {
            error: error.clone(),
        });

        self.stop_streaming();
        self.agent_state.interrupt();
        self.chat
            .messages
            .push(("system".to_string(), format!("Error: {}", error)));

        // If last message was a tool_result, add error assistant message
        let needs_assistant = self
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
            self.chat.conversation.push(assistant_msg.clone());
            self.save_model_message(&assistant_msg);
        }

        self.streaming.reset();
    }

    // =========================================================================
    // Metrics and Context
    // =========================================================================

    /// Handle usage event
    fn handle_usage(
        &mut self,
        prompt_tokens: usize,
        completion_tokens: usize,
        cache_read_tokens: usize,
        cache_created_tokens: usize,
    ) {
        self.context_tokens_used = prompt_tokens + completion_tokens;
        if cache_read_tokens > 0 || cache_created_tokens > 0 {
            tracing::info!(
                "Cache: read={} created={} total_input={}",
                cache_read_tokens,
                cache_created_tokens,
                prompt_tokens
            );
        }
        self.save_session_token_count();

        // Check if we should trigger auto-pinch (after AI finishes)
        self.check_auto_pinch();
    }

    /// Handle context edited event
    ///
    /// When the API clears old context (tool results, thinking blocks), it sends
    /// this event. The next Usage event will have the updated (lower) token count.
    fn handle_context_edited(
        &mut self,
        cleared_tokens: usize,
        cleared_tool_uses: usize,
        cleared_thinking_turns: usize,
    ) {
        if cleared_tokens > 0 {
            tracing::info!(
                "Context edited by server: cleared {} tokens ({} tool uses, {} thinking turns)",
                cleared_tokens,
                cleared_tool_uses,
                cleared_thinking_turns
            );
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
            self.blocks.web_search.push(block);
            self.chat
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
        self.chat
            .messages
            .push(("system".to_string(), format!("Fetched: {}", title)));
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Mark all streaming blocks as complete
    fn complete_streaming_blocks(&mut self) {
        for rb in &mut self.blocks.read {
            if rb.is_streaming() {
                rb.complete();
            }
        }
        for eb in &mut self.blocks.edit {
            if eb.is_streaming() {
                eb.complete();
            }
        }
        for wb in &mut self.blocks.write {
            if wb.is_streaming() {
                wb.complete();
            }
        }
        for ws in &mut self.blocks.web_search {
            if ws.is_streaming() {
                ws.complete();
            }
        }
        // NOTE: ExploreBlocks are NOT completed here - they wait for tool results
        // for eb in &mut self.blocks.explore {
        //     if eb.is_streaming() {
        //         eb.complete(String::new());
        //     }
        // }
    }

    /// Add a filler message after tool_result if needed
    /// Note: Filler messages are only added to in-memory conversation for API alternation.
    /// They are NOT saved to database - the API client handles filler insertion dynamically.
    fn maybe_add_filler_message(&mut self) {
        let needs_assistant_follow_up = self
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

        if needs_assistant_follow_up {
            tracing::debug!(
                "Adding in-memory filler assistant message after tool_result (not saved to DB)"
            );
            let assistant_msg = ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: ".".to_string(),
                }],
            };
            // Only push to conversation for API alternation - do NOT save to database
            self.chat.conversation.push(assistant_msg);
        }
    }

    /// Check and execute pending tools when ready
    pub fn check_and_execute_tools(&mut self) {
        // Don't execute tools while decision prompt is visible (waiting for user input)
        if self.decision_prompt.visible {
            return;
        }

        if self.streaming.is_ready_for_tools() && self.channels.tool_results.is_none() {
            // Build and save assistant message BEFORE executing tools
            if let Some(assistant_msg) = self.streaming.build_assistant_message() {
                tracing::info!(
                    "SAVING assistant message with {} content blocks BEFORE tool execution",
                    assistant_msg.content.len()
                );
                self.chat.conversation.push(assistant_msg.clone());
                self.save_model_message(&assistant_msg);
            }

            // Take tool calls from StreamingManager (transitions to Complete state)
            if let Some((_text, _thinking_blocks, tool_calls)) = self.streaming.take_tool_calls() {
                // Spawn tool execution as background task
                self.spawn_tool_execution(tool_calls);
            }
        }
    }
}
