//! Stream event handlers
//!
//! Processes orchestrator loop events and updates application state.
//! The core orchestrator handles the agentic cycle (stream -> tools -> repeat)
//! and emits LoopEvents that this module translates to TUI visual state.

mod errors;
mod lifecycle;
mod recovery;
mod text;
mod thinking;
mod tools;
mod web;

use crate::agent::loop_events::LoopEvent;
use crate::tui::app::App;
use crate::tui::state::StreamDrainMode;

impl App {
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

    fn handle_loop_event(&mut self, event: LoopEvent) {
        match event {
            LoopEvent::TextDelta { delta } => self.handle_text_delta(delta),
            LoopEvent::TextDeltaWithCitations { delta, citations } => {
                self.handle_text_delta_with_citations(delta, citations)
            }
            LoopEvent::ThinkingDelta { thinking } => self.handle_streaming_thinking_delta(thinking),
            LoopEvent::ThinkingComplete {
                thinking: _,
                signature,
            } => self.handle_thinking_complete(signature),

            LoopEvent::ToolCallStart { id: _, name } => self.handle_tool_start(name),
            LoopEvent::ToolCallComplete {
                id,
                name,
                arguments,
            } => self.handle_tool_call_complete(id, name, arguments),
            LoopEvent::ToolExecuting { id: _, name: _ } => self.handle_tool_executing(),
            LoopEvent::ToolOutputDelta { id, delta } => self.handle_tool_output_delta(id, delta),
            LoopEvent::ToolResult {
                id,
                output,
                is_error,
            } => self.handle_tool_result(id, output, is_error),

            LoopEvent::ToolApprovalRequired {
                id,
                name,
                arguments: _,
            } => self.handle_tool_approval_required(id, name),
            LoopEvent::ToolApproved { id } => self.handle_tool_approved(id),
            LoopEvent::ToolDenied { id } => self.handle_tool_denied(id),
            LoopEvent::AwaitingInput {
                tool_call_id,
                tool_name,
            } => self.handle_awaiting_input(tool_call_id, tool_name),

            LoopEvent::ServerToolStart { id, name } => self.handle_server_tool_start(id, name),
            LoopEvent::ServerToolComplete { id, name } => {
                self.handle_server_tool_complete(id, name)
            }
            LoopEvent::WebSearchResults {
                tool_use_id,
                results,
            } => self.handle_web_search_results(tool_use_id, results),
            LoopEvent::WebFetchResult {
                tool_use_id,
                content,
            } => self.handle_web_fetch_result(tool_use_id, content),
            LoopEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => self.handle_server_tool_error(tool_use_id, error_code),

            LoopEvent::ModeChange { mode, reason } => self.handle_mode_change(mode, reason),
            LoopEvent::PlanUpdate { tasks } => self.handle_plan_update(tasks.len()),
            LoopEvent::PlanComplete {
                tool_call_id: _,
                title,
                task_count,
            } => self.handle_plan_complete(title, task_count),
            LoopEvent::AgentSleeping {
                duration_secs,
                reason,
            } => self.handle_agent_sleeping(duration_secs, reason),

            LoopEvent::TurnComplete { turn, has_more } => self.handle_turn_complete(turn, has_more),
            LoopEvent::TickInjected { tick_number } => self.handle_tick_injected(tick_number),
            LoopEvent::Usage {
                prompt_tokens,
                input_tokens,
                completion_tokens,
                reasoning_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                total_tokens,
            } => self.handle_usage(
                prompt_tokens,
                input_tokens,
                completion_tokens,
                reasoning_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                total_tokens,
            ),
            LoopEvent::SessionPinched {
                reason,
                source_session_id: _,
                new_session_id,
                estimated_tokens_before,
            } => self.handle_session_pinched(reason, new_session_id, estimated_tokens_before),
            LoopEvent::ContextCompactionStarted { reason } => {
                self.handle_compaction_started(reason)
            }
            LoopEvent::ContextCompacted {
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
                checkpoint_id: _,
                compaction_count: _,
            } => self.handle_context_compacted(
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
            ),
            LoopEvent::TitleGenerated { title } => self.handle_title_generated(title),
            LoopEvent::Finished {
                session_id,
                stop_reason,
            } => self.handle_finished(session_id, stop_reason),
            LoopEvent::Error { error } => self.handle_stream_error(error),

            LoopEvent::AgentBackgroundStarted {
                delegated_run_id,
                agent_type,
                description,
            } => self.handle_agent_background_started(delegated_run_id, agent_type, description),
            LoopEvent::AgentBackgroundCompleted {
                delegated_run_id,
                agent_type,
                success,
                summary,
            } => self.handle_agent_background_completed(
                delegated_run_id,
                agent_type,
                success,
                summary,
            ),

            LoopEvent::UserMessage {
                title,
                message,
                level,
            } => self.handle_user_message(title, message, level),
            LoopEvent::ClassifierDecision {
                tool_name,
                decision,
                reason,
                stage,
            } => self.handle_classifier_decision(tool_name, decision, reason, stage),
            LoopEvent::TeammateSpawned { name, role } => self.handle_teammate_spawned(name, role),
            LoopEvent::TeammateTaskCompleted {
                name,
                task_id,
                result,
            } => self.handle_teammate_task_completed(name, task_id, result),
            LoopEvent::TeammateTaskFailed {
                name,
                task_id,
                error,
            } => self.handle_teammate_task_failed(name, task_id, error),
            LoopEvent::TeammateCancelled { name } => self.handle_teammate_cancelled(name),
        }
    }
}
