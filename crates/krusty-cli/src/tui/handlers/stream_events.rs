//! Stream event handlers
//!
//! Processes orchestrator loop events and updates application state.
//! The core orchestrator handles the agentic cycle (stream -> tools -> repeat)
//! and emits LoopEvents that this module translates to TUI visual state.

use crate::agent::loop_events::{LoopEvent, LoopStopReason};
use crate::agent::AgentEvent;
use crate::ai::types::{AiToolCall, Content, ModelMessage, Role};
use crate::tui::app::{App, WorkMode};
use crate::tui::blocks::{StreamBlock, WebSearchBlock};
use crate::tui::state::{StreamDrainMode, StreamDrainTelemetry};

use super::sessions::storage_role_to_api_role;

impl App {
    fn append_streaming_assistant_delta(&mut self, delta: String) {
        // Use cached streaming assistant index (O(1)) instead of O(n) scan per delta.
        let append_idx = if let Some(idx) = self.runtime.chat.streaming_assistant_idx {
            if idx < self.runtime.chat.messages.len()
                && idx + 1 == self.runtime.chat.messages.len()
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
        if self.runtime.channels.loop_events.is_none() && !self.runtime.chat.has_stream_backlog() {
            return false;
        }

        let mut processed_any = false;
        let mut disconnected = false;

        if let Some(mut rx) = self.runtime.channels.loop_events.take() {
            loop {
                match rx.try_recv() {
                    Ok(event) => self.runtime.chat.stream_drain.enqueue(event),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }

            if !disconnected {
                self.runtime.channels.loop_events = Some(rx);
            }
        }

        let backlog_before = self.runtime.chat.stream_drain.pending_len();
        let oldest_age_ms = self.runtime.chat.stream_drain.oldest_age().as_millis() as u64;
        let mode_before = self.runtime.chat.stream_drain.mode();
        let batch_limit = self.runtime.chat.stream_drain.next_batch_limit();
        let mode_after = self.runtime.chat.stream_drain.mode();

        if backlog_before > 0 && mode_before != mode_after {
            tracing::debug!(
                pending_events = backlog_before,
                oldest_age_ms,
                mode = ?mode_after,
                batch_limit,
                "Switching stream drain mode"
            );
        }

        for _ in 0..batch_limit {
            let Some(event) = self.runtime.chat.stream_drain.pop_next() else {
                break;
            };
            processed_any = true;
            self.handle_loop_event(event);
        }

        if self.runtime.chat.has_stream_backlog() {
            let pending_after = self.runtime.chat.stream_drain.pending_len();
            let oldest_after_ms = self.runtime.chat.stream_drain.oldest_age().as_millis() as u64;
            let mode = self.runtime.chat.stream_drain.mode();
            let telemetry = self.runtime.chat.stream_drain.telemetry();

            if matches!(mode, StreamDrainMode::CatchUp) {
                tracing::trace!(
                    pending_events = pending_after,
                    oldest_age_ms = oldest_after_ms,
                    batch_limit,
                    coalesced_events = telemetry.coalesced_events,
                    dropped_events = telemetry.dropped_events,
                    "Continuing catch-up stream drain"
                );
            }
        }

        if disconnected
            && !self.runtime.chat.has_stream_backlog()
            && (self.runtime.chat.is_streaming || self.runtime.chat.is_executing_tools)
        {
            tracing::warn!("Orchestrator event channel disconnected before completion");
            self.stop_streaming();
            self.stop_tool_execution();
            self.runtime.channels.loop_events = None;
            self.runtime.channels.loop_input = None;
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Stream interrupted before the orchestrator finalized the turn.".to_string(),
            ));
            self.ui.scroll_system.scroll.request_scroll_to_bottom();
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
                    if let Some(call_idx) = self
                        .runtime
                        .pending_ask_user_calls
                        .iter()
                        .position(|c| c.id == tool_call_id)
                    {
                        let call = self.runtime.pending_ask_user_calls.remove(call_idx);
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
                if let Some(session_id) = self.runtime.current_session_id.clone() {
                    if let Some(ref pm) = self.services.plan_manager {
                        let had_active_plan = self.runtime.active_plan.is_some();
                        let stored_mode = self.ui.work_mode.into();
                        match pm.get_lifecycle_state(&session_id, stored_mode) {
                            Ok(lifecycle) => {
                                self.ui.work_mode = WorkMode::from(lifecycle.effective_work_mode);
                                if let Some(plan) = lifecycle.active_plan {
                                    self.set_plan(plan);
                                    if !self.ui.plan_sidebar.visible {
                                        self.ui.plan_sidebar.toggle();
                                    }
                                } else if had_active_plan {
                                    self.ui.plan_sidebar.start_collapse();
                                } else {
                                    self.clear_active_plan();
                                }
                            }
                            Err(err) => {
                                tracing::warn!("Failed to reload plan after update: {}", err);
                            }
                        }
                    }
                }
            }
            LoopEvent::PlanComplete {
                tool_call_id: _,
                title,
                task_count,
            } => {
                // Reload plan from DB and show confirmation
                if let Some(session_id) = self.runtime.current_session_id.clone() {
                    if let Some(ref pm) = self.services.plan_manager {
                        if let Ok(Some(plan)) = pm.get_active_plan(&session_id) {
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
            LoopEvent::AgentSleeping {
                duration_secs,
                reason,
            } => {
                tracing::info!(
                    duration_secs = duration_secs,
                    "Agent sleeping between ticks: {}",
                    reason
                );
            }

            // -- Turn lifecycle -------------------------------------------
            LoopEvent::TurnComplete { turn, has_more } => {
                self.runtime.agent_state.current_turn = turn;
                if !has_more {
                    self.stop_tool_execution();
                }
            }
            LoopEvent::TickInjected { tick_number } => {
                tracing::info!(tick_number = tick_number, "Injected autonomous tick");
            }
            LoopEvent::Usage {
                prompt_tokens,
                completion_tokens,
            } => {
                self.runtime.context_tokens_used = prompt_tokens + completion_tokens;
                self.save_session_token_count();
            }
            LoopEvent::SessionPinched {
                reason,
                source_session_id: _,
                new_session_id,
                estimated_tokens_before,
            } => {
                self.runtime.pending_pinched_session_id = Some(new_session_id.clone());
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "Pinch ({}) moved this run into session {} at ~{} tokens so work can continue with fresh context.",
                        reason, new_session_id, estimated_tokens_before
                    ),
                ));
            }
            LoopEvent::TitleGenerated { title } => {
                self.runtime.session_title = Some(title);
            }
            LoopEvent::Finished {
                session_id,
                stop_reason,
            } => {
                tracing::info!("Orchestrator finished for session {}", session_id);
                let stream_telemetry = self.runtime.chat.stream_drain.telemetry();
                let next_pinched_session = if stop_reason == LoopStopReason::Pinched {
                    self.runtime.pending_pinched_session_id.take()
                } else {
                    None
                };
                if next_pinched_session.is_none() {
                    self.reload_conversation_from_db();
                }
                self.stop_streaming();
                self.stop_tool_execution();
                self.runtime.channels.loop_events = None;
                self.runtime.channels.loop_input = None;
                self.runtime.pending_ask_user_calls.clear();
                self.push_stream_recovery_banner(stop_reason, stream_telemetry);
                if let Some(new_session_id) = next_pinched_session {
                    if let Err(error) = self.load_session(&new_session_id) {
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            format!(
                                "Pinch created session {}, but the TUI could not load it automatically: {}",
                                new_session_id, error
                            ),
                        ));
                    } else {
                        self.send_to_ai();
                    }
                }
            }
            LoopEvent::Error { error } => {
                self.handle_stream_error(error);
            }

            // -- Background agent lifecycle --------------------------------
            LoopEvent::AgentBackgroundStarted {
                delegated_run_id,
                agent_type,
                description,
            } => {
                tracing::info!(
                    delegated_run_id = %delegated_run_id,
                    agent_type = %agent_type,
                    "Background agent started: {}",
                    description,
                );
            }
            LoopEvent::AgentBackgroundCompleted {
                delegated_run_id,
                agent_type,
                success,
                summary,
            } => {
                tracing::info!(
                    delegated_run_id = %delegated_run_id,
                    agent_type = %agent_type,
                    success = success,
                    "Background agent completed: {}",
                    summary,
                );
            }

            // -- Mako autonomous agent events -----------------------------
            LoopEvent::UserMessage {
                title,
                message,
                level,
            } => {
                tracing::info!(
                    level = %level,
                    title = ?title,
                    "User message: {}",
                    message,
                );
            }
            LoopEvent::ClassifierDecision {
                tool_name,
                decision,
                reason,
                stage,
            } => {
                tracing::debug!(
                    tool_name = %tool_name,
                    decision = %decision,
                    stage = stage,
                    "Classifier decision: {}",
                    reason,
                );
            }
            LoopEvent::TeammateSpawned { name, role } => {
                tracing::info!(name = %name, role = %role, "Teammate spawned");
            }
            LoopEvent::TeammateTaskCompleted {
                name,
                task_id,
                result,
            } => {
                tracing::info!(
                    name = %name,
                    task_id = %task_id,
                    "Teammate task completed: {}",
                    result,
                );
            }
            LoopEvent::TeammateTaskFailed {
                name,
                task_id,
                error,
            } => {
                tracing::warn!(
                    name = %name,
                    task_id = %task_id,
                    "Teammate task failed: {}",
                    error,
                );
            }
            LoopEvent::TeammateCancelled { name } => {
                tracing::info!(name = %name, "Teammate cancelled");
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
                    let content: Vec<Content> = serde_json::from_str::<Vec<Content>>(&content_json)
                        .or_else(|_| {
                            serde_json::from_str::<Content>(&content_json).map(|c| vec![c])
                        })
                        .unwrap_or_else(|_| vec![Content::Text { text: content_json }]);
                    self.runtime.chat.conversation.push(ModelMessage {
                        role: storage_role_to_api_role(role.as_str()),
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

    fn push_stream_recovery_banner(
        &mut self,
        stop_reason: LoopStopReason,
        telemetry: StreamDrainTelemetry,
    ) {
        let detail = if telemetry.dropped_events > 0 || telemetry.coalesced_events > 0 {
            format!(
                " TUI catch-up coalesced {} chunk(s) and dropped {} low-priority delta(s).",
                telemetry.coalesced_events, telemetry.dropped_events
            )
        } else {
            String::new()
        };

        if let Some(session_id) = self.runtime.current_session_id.clone() {
            if let Some(recovery_state) = self.load_persisted_recovery_state(&session_id) {
                self.push_recovery_notice(
                    &recovery_state,
                    if detail.is_empty() {
                        None
                    } else {
                        Some(detail)
                    },
                );
                return;
            }
        }

        let fallback = match stop_reason {
            LoopStopReason::StreamIdleTimeout => Some(format!(
                "Stream interrupted: the provider stopped sending data before the idle timeout expired. Resume the turn manually if you want to continue.{}",
                detail
            )),
            LoopStopReason::ProviderError => Some(format!(
                "Stream interrupted by a provider error. Krusty stopped the turn instead of replaying it automatically.{}",
                detail
            )),
            _ => None,
        };

        if let Some(message) = fallback {
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), message));
        }
    }

    // =========================================================================
    // Text Handling
    // =========================================================================

    /// Handle text delta from AI response
    fn handle_text_delta(&mut self, delta: String) {
        // Mark all streaming blocks complete when AI starts responding
        self.complete_streaming_blocks();

        // Cache is cleared at the start of each new streaming session (start_streaming),
        // so a None cache means this is the first text delta of a new turn.
        self.append_streaming_assistant_delta(delta);

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
