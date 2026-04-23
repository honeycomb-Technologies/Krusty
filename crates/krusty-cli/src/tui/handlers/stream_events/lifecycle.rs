use crate::agent::loop_events::LoopStopReason;
use crate::tui::app::{App, WorkMode};

impl App {
    pub(super) fn handle_mode_change(&mut self, mode: String, reason: Option<String>) {
        let new_mode = match mode.as_str() {
            "plan" | "Plan" => WorkMode::Plan,
            _ => WorkMode::Build,
        };
        self.ui.work_mode = new_mode;
        if let Some(reason) = reason {
            tracing::info!("Mode changed to {:?}: {}", new_mode, reason);
        }
    }

    pub(super) fn handle_plan_update(&mut self, task_count: usize) {
        tracing::info!("Plan updated: {} tasks", task_count);
        if let Some(session_id) = self.runtime.current_session_id.clone() {
            if let Some(ref plan_manager) = self.services.plan_manager {
                let had_active_plan = self.runtime.active_plan.is_some();
                let stored_mode = self.ui.work_mode.into();
                match plan_manager.get_lifecycle_state(&session_id, stored_mode) {
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

    pub(super) fn handle_plan_complete(&mut self, title: String, task_count: usize) {
        if let Some(session_id) = self.runtime.current_session_id.clone() {
            if let Some(ref plan_manager) = self.services.plan_manager {
                if let Ok(Some(plan)) = plan_manager.get_active_plan(&session_id) {
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

    pub(super) fn handle_agent_sleeping(&mut self, duration_secs: u64, reason: String) {
        tracing::info!(
            duration_secs = duration_secs,
            "Agent sleeping between ticks: {}",
            reason
        );
    }

    pub(super) fn handle_turn_complete(&mut self, turn: usize, has_more: bool) {
        self.runtime.agent_state.current_turn = turn;
        if !has_more {
            self.stop_tool_execution();
        }
    }

    pub(super) fn handle_tick_injected(&mut self, tick_number: usize) {
        tracing::info!(tick_number = tick_number, "Injected autonomous tick");
    }

    pub(super) fn handle_usage(&mut self, prompt_tokens: usize, completion_tokens: usize) {
        self.runtime.context_tokens_used = prompt_tokens + completion_tokens;
        self.save_session_token_count();
    }

    pub(super) fn handle_session_pinched(
        &mut self,
        reason: String,
        new_session_id: String,
        estimated_tokens_before: usize,
    ) {
        self.runtime.pending_pinched_session_id = Some(new_session_id.clone());
        self.runtime.chat.messages.push((
            "system".to_string(),
            format!(
                "Pinch ({}) moved this run into session {} at ~{} tokens so work can continue with fresh context.",
                reason, new_session_id, estimated_tokens_before
            ),
        ));
    }

    pub(super) fn handle_title_generated(&mut self, title: String) {
        self.runtime.session_title = Some(title);
    }

    pub(super) fn handle_finished(&mut self, session_id: String, stop_reason: LoopStopReason) {
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

    pub(super) fn handle_agent_background_started(
        &mut self,
        delegated_run_id: String,
        agent_type: String,
        description: String,
    ) {
        tracing::info!(
            delegated_run_id = %delegated_run_id,
            agent_type = %agent_type,
            "Background agent started: {}",
            description,
        );
    }

    pub(super) fn handle_agent_background_completed(
        &mut self,
        delegated_run_id: String,
        agent_type: String,
        success: bool,
        summary: String,
    ) {
        tracing::info!(
            delegated_run_id = %delegated_run_id,
            agent_type = %agent_type,
            success = success,
            "Background agent completed: {}",
            summary,
        );
    }

    pub(super) fn handle_user_message(
        &mut self,
        title: Option<String>,
        message: String,
        level: String,
    ) {
        tracing::info!(
            level = %level,
            title = ?title,
            "User message: {}",
            message,
        );
    }

    pub(super) fn handle_classifier_decision(
        &mut self,
        tool_name: String,
        decision: String,
        reason: String,
        stage: u8,
    ) {
        tracing::debug!(
            tool_name = %tool_name,
            decision = %decision,
            stage = stage,
            "Classifier decision: {}",
            reason,
        );
    }

    pub(super) fn handle_teammate_spawned(&mut self, name: String, role: String) {
        tracing::info!(name = %name, role = %role, "Teammate spawned");
    }

    pub(super) fn handle_teammate_task_completed(
        &mut self,
        name: String,
        task_id: String,
        result: String,
    ) {
        tracing::info!(
            name = %name,
            task_id = %task_id,
            "Teammate task completed: {}",
            result,
        );
    }

    pub(super) fn handle_teammate_task_failed(
        &mut self,
        name: String,
        task_id: String,
        error: String,
    ) {
        tracing::warn!(
            name = %name,
            task_id = %task_id,
            "Teammate task failed: {}",
            error,
        );
    }

    pub(super) fn handle_teammate_cancelled(&mut self, name: String) {
        tracing::info!(name = %name, "Teammate cancelled");
    }
}
