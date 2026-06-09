use super::*;
use krusty_core::ai::client::supports_openai_xhigh_reasoning;

impl App {
    /// Get max context window size for current model
    pub fn max_context_tokens(&self) -> usize {
        // First check dynamic ModelRegistry (OpenRouter models live here)
        // Use try_get_model() to avoid blocking during rendering
        if let Some(metadata) = self
            .services
            .model_registry
            .try_get_model(&self.runtime.current_model)
        {
            return metadata.context_window;
        }

        // Fall back to static provider config (Anthropic, Z.ai, etc.)
        if let Some(provider) = crate::ai::providers::get_provider(self.runtime.active_provider) {
            if let Some(model) = provider
                .models
                .iter()
                .find(|m| m.id == self.runtime.current_model)
            {
                return model.context_window;
            }
        }

        resolve_context_window(
            self.runtime.active_provider,
            &self.runtime.current_model,
            detect_api_format(self.runtime.active_provider, &self.runtime.current_model),
        )
    }

    /// Whether Tab should cycle OpenAI xhigh-capable thinking levels.
    pub fn is_openai_xhigh_thinking_mode(&self) -> bool {
        self.runtime.active_provider == ProviderId::OpenAI
            && supports_openai_xhigh_reasoning(&self.runtime.current_model)
    }

    /// Whether Tab should cycle Anthropic Opus 4.6 thinking levels.
    pub fn is_anthropic_opus_thinking_mode(&self) -> bool {
        self.runtime.active_provider == ProviderId::Anthropic
            && (self.runtime.current_model.contains("opus-4-6")
                || self.runtime.current_model.contains("opus-4.6"))
    }

    /// Whether Tab should cycle Grok Build/Composer thinking effort levels.
    pub fn is_grok_thinking_mode(&self) -> bool {
        self.runtime.active_provider == ProviderId::Grok
            && (self.runtime.current_model == "grok-build"
                || self.runtime.current_model.starts_with("grok-composer-"))
    }

    /// Whether this model supports multi-level thinking cycling.
    pub fn has_multi_level_thinking(&self) -> bool {
        self.is_openai_xhigh_thinking_mode()
            || self.is_anthropic_opus_thinking_mode()
            || self.is_grok_thinking_mode()
    }

    /// Handle Tab thinking toggle/cycle.
    pub fn cycle_thinking_level(&mut self) {
        self.runtime.thinking_level =
            if self.is_openai_xhigh_thinking_mode() || self.is_grok_thinking_mode() {
                self.runtime.thinking_level.cycle_codex()
            } else if self.is_anthropic_opus_thinking_mode() {
                self.runtime.thinking_level.cycle_anthropic()
            } else {
                self.runtime.thinking_level.toggle_basic()
            };
        tracing::info!(
            model = %self.runtime.current_model,
            multi_level = self.has_multi_level_thinking(),
            thinking_level = self.runtime.thinking_level.label(),
            "Updated thinking level"
        );
    }

    /// Input border color based on thinking intensity.
    pub fn thinking_border_color(&self) -> Color {
        match self.runtime.thinking_level {
            ThinkingLevel::Off => self.ui.theme.border_color,
            ThinkingLevel::Low => self.ui.theme.mode_view_color,
            ThinkingLevel::Medium => self.ui.theme.accent_color,
            ThinkingLevel::High => self.ui.theme.warning_color,
            ThinkingLevel::XHigh => self.ui.theme.error_color,
        }
    }

    fn persist_work_mode(&self, mode: WorkMode) {
        let Some(session_id) = self.runtime.current_session_id.as_deref() else {
            return;
        };
        let Some(session_manager) = self.services.session_manager.as_ref() else {
            return;
        };
        let storage_mode: crate::storage::WorkMode = mode.into();
        if let Err(err) = session_manager.update_session_work_mode(session_id, storage_mode) {
            tracing::warn!(
                session_id = %session_id,
                mode = %storage_mode,
                "Failed to persist TUI work mode: {}",
                err
            );
        }
    }

    /// Persist the current TUI work mode to session storage when possible.
    pub fn persist_current_work_mode(&self) {
        self.persist_work_mode(self.ui.work_mode);
    }

    /// Apply a work mode in the UI and persist it to session storage.
    pub fn set_work_mode(&mut self, mode: WorkMode) {
        self.ui.work_mode = mode;
        self.persist_work_mode(mode);
    }

    /// Clear the active plan without mutating session work mode.
    pub fn clear_active_plan(&mut self) {
        self.runtime.active_plan = None;
        self.ui.plan_sidebar.reset();
    }

    /// Clear the active plan and return to build mode.
    pub fn clear_plan(&mut self) {
        self.clear_active_plan();
        self.set_work_mode(WorkMode::Build);
    }

    /// Set the active plan without changing work mode
    ///
    /// Callers are responsible for setting the appropriate WorkMode:
    /// - New plan from AI: set WorkMode::Plan
    /// - Session resume: use canonical lifecycle resolution
    pub fn set_plan(&mut self, plan: PlanFile) {
        self.runtime.active_plan = Some(plan);
    }

    /// Trigger auto-pinch if pending and conditions are right
    ///
    /// Called from main loop. When AI is busy (autonomous work), bypasses the popup
    /// entirely and runs pinch in the background. When idle, shows the popup for
    /// manual interaction.
    pub fn trigger_pending_auto_pinch(&mut self) {
        if !self.runtime.pending_auto_pinch {
            return;
        }

        // Don't trigger if still busy with streaming or tools
        if self.runtime.chat.is_streaming || self.runtime.chat.is_executing_tools {
            return;
        }

        // Don't trigger if already in a popup or auto-pinch is running
        if self.ui.popup != crate::tui::app::Popup::None || self.runtime.auto_pinch_in_progress {
            return;
        }

        // Don't trigger if no session
        if self.runtime.current_session_id.is_none() {
            self.runtime.pending_auto_pinch = false;
            self.runtime.pending_auto_pinch_reason = None;
            return;
        }

        self.runtime.pending_auto_pinch = false;
        let reason = self
            .runtime
            .pending_auto_pinch_reason
            .take()
            .unwrap_or_else(|| {
                "The current thread is no longer healthy enough to keep running in place."
                    .to_string()
            });

        // Calculate usage percent
        let max_tokens = self.max_context_tokens();
        let usage_percent = if max_tokens > 0 {
            ((self.runtime.context_tokens_used as f64 / max_tokens as f64) * 100.0) as u8
        } else {
            0
        };

        // Show system message explaining why
        self.runtime.chat.messages.push((
            "system".to_string(),
            format!(
                "{} Starting pinch fallback at {}% capacity ({} / {} tokens) so the conversation can continue with fresh context.",
                reason, usage_percent, self.runtime.context_tokens_used, max_tokens
            ),
        ));

        // Check if conversation has pending AI work (multi-turn tool loop).
        // If the last message is a tool result or assistant message with tool calls,
        // the AI was mid-flow — bypass popup and auto-pinch silently.
        let was_autonomous = self.runtime.chat.conversation.last().is_some_and(|msg| {
            msg.role == crate::ai::types::Role::User
                && msg
                    .content
                    .iter()
                    .any(|c| matches!(c, crate::ai::types::Content::ToolResult { .. }))
        });

        if was_autonomous {
            // AI was working autonomously — bypass popup
            tracing::info!("Auto-pinch: AI was autonomous, bypassing popup");
            self.start_auto_pinch();
        } else {
            // User is interactive — show popup as before
            let top_files = self.get_top_files_preview(5);
            self.ui.popups.pinch.start(usage_percent, top_files);
            self.ui.popup = crate::tui::app::Popup::Pinch;
        }
    }

    /// Show a toast notification
    pub fn show_toast(&mut self, toast: crate::tui::components::Toast) {
        self.ui.toasts.push(toast);
    }

    /// Get plan info for toolbar display
    pub fn get_plan_info(&self) -> Option<crate::tui::components::PlanInfo<'_>> {
        self.runtime.active_plan.as_ref().map(|plan| {
            let (completed, total) = plan.progress();
            crate::tui::components::PlanInfo {
                _title: &plan.title,
                completed,
                total,
            }
        })
    }

    // =========================================================================
    // Processing State Helpers
    // =========================================================================

    /// Start streaming from AI - sets is_streaming flag
    pub fn start_streaming(&mut self) {
        self.runtime
            .chat
            .start_streaming_with_policy(self.current_stream_drain_policy());
    }

    /// Stop streaming from AI - clears is_streaming flag and related caches
    pub fn stop_streaming(&mut self) {
        let telemetry = self.runtime.chat.stream_drain.telemetry();
        if telemetry.dropped_events > 0
            || telemetry.coalesced_events > 0
            || telemetry.mode_switches > 0
        {
            tracing::info!(
                model = %self.runtime.current_model,
                provider = %self.runtime.active_provider,
                enqueued_events = telemetry.enqueued_events,
                dequeued_events = telemetry.dequeued_events,
                coalesced_events = telemetry.coalesced_events,
                dropped_events = telemetry.dropped_events,
                mode_switches = telemetry.mode_switches,
                peak_pending = telemetry.peak_pending,
                peak_oldest_age_ms = telemetry.peak_oldest_age.as_millis() as u64,
                "Stream drain telemetry"
            );
        }
        self.runtime.chat.stop_streaming();
    }

    /// Start tool execution - sets is_executing_tools flag
    pub fn start_tool_execution(&mut self) {
        self.runtime.chat.start_tool_execution();
    }

    /// Stop tool execution - clears is_executing_tools flag
    pub fn stop_tool_execution(&mut self) {
        self.runtime.chat.stop_tool_execution();
    }

    fn current_stream_drain_policy(&self) -> crate::ai::model_profile::StreamDrainPolicy {
        let api_format =
            detect_api_format(self.runtime.active_provider, &self.runtime.current_model);
        ModelProfile::resolve(
            self.runtime.active_provider,
            api_format,
            &self.runtime.current_model,
        )
        .stream_drain_policy()
    }

    /// Apply any pending view change (called at end of event loop iteration)
    pub fn apply_pending_view_change(&mut self) {
        if let Some(view) = self.ui.pending_view_change.take() {
            self.ui.view = view;
        }
    }

    /// Check if busy (streaming OR executing tools)
    pub fn is_busy(&self) -> bool {
        self.runtime.chat.is_busy()
    }

    /// Start editing the session title
    pub fn start_title_edit(&mut self) {
        if self.ui.view == View::Chat {
            self.runtime
                .title_editor
                .start(self.runtime.session_title.as_deref());
        }
    }

    /// Cancel title editing and revert
    pub fn cancel_title_edit(&mut self) {
        self.runtime.title_editor.cancel();
    }

    /// Save the edited title
    pub fn save_title_edit(&mut self) {
        if let Some(new_title) = self.runtime.title_editor.finish() {
            self.runtime.session_title = Some(new_title.clone());

            // Save to database
            if let (Some(manager), Some(session_id)) = (
                &self.services.session_manager,
                &self.runtime.current_session_id,
            ) {
                let _ = manager.update_session_title(session_id, &new_title);
            }
        }
    }
}
