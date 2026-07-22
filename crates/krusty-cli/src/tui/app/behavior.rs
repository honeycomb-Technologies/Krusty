use super::*;
use krusty_core::ai::providers::ReasoningControl;

impl App {
    /// Get max context window size for current model
    pub fn max_context_tokens(&self) -> usize {
        // Use the exact provider/auth/transport row; bare slugs may be shared.
        if let Some(metadata) = self.selected_model_metadata() {
            return metadata.context_window;
        }

        self.runtime.current_model_key.as_ref().map_or_else(
            || {
                resolve_context_window(
                    self.runtime.active_provider,
                    &self.runtime.current_model,
                    detect_api_format(self.runtime.active_provider, &self.runtime.current_model),
                )
            },
            |key| resolve_context_window(key.provider, &key.model_id, key.api_format),
        )
    }

    pub fn selectable_thinking_levels(&self) -> Vec<ThinkingLevel> {
        let Some(metadata) = self.selected_model_metadata() else {
            return vec![ThinkingLevel::Off];
        };
        if metadata.reasoning_control == Some(ReasoningControl::OutputOnly) {
            return vec![ThinkingLevel::Off];
        }

        let mut levels = metadata
            .supported_reasoning_levels
            .iter()
            .copied()
            .map(ThinkingLevel::from_reasoning_effort)
            .filter(|level| *level != ThinkingLevel::Ultra)
            .collect::<Vec<_>>();
        levels.dedup();
        let fallback = metadata
            .default_reasoning_level
            .map(ThinkingLevel::from_reasoning_effort)
            .filter(|level| !matches!(level, ThinkingLevel::Off | ThinkingLevel::Ultra))
            .unwrap_or(ThinkingLevel::Medium);
        if levels.is_empty() {
            return if metadata.supports_thinking {
                if metadata.reasoning_is_mandatory {
                    vec![fallback]
                } else {
                    vec![ThinkingLevel::Off, fallback]
                }
            } else {
                vec![ThinkingLevel::Off]
            };
        }
        if metadata.reasoning_is_mandatory {
            levels.retain(|level| *level != ThinkingLevel::Off);
            if levels.is_empty() {
                levels.push(fallback);
            }
        } else if !levels.contains(&ThinkingLevel::Off) {
            levels.insert(0, ThinkingLevel::Off);
        }
        levels
    }

    /// Whether this model supports multi-level thinking cycling.
    pub fn has_multi_level_thinking(&self) -> bool {
        self.selectable_thinking_levels().len() > 2
    }

    /// Handle Tab thinking toggle/cycle.
    pub fn cycle_thinking_level(&mut self) {
        let levels = self.selectable_thinking_levels();
        let next_index = levels
            .iter()
            .position(|level| *level == self.runtime.thinking_level)
            .map_or(0, |index| (index + 1) % levels.len());
        self.runtime.thinking_level = levels[next_index];
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
            ThinkingLevel::Minimal => self.ui.theme.mode_view_color,
            ThinkingLevel::Low => self.ui.theme.mode_view_color,
            ThinkingLevel::Medium => self.ui.theme.accent_color,
            ThinkingLevel::High => self.ui.theme.warning_color,
            ThinkingLevel::XHigh => self.ui.theme.error_color,
            ThinkingLevel::Max | ThinkingLevel::Ultra => self.ui.theme.error_color,
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

    fn current_stream_drain_policy(&self) -> crate::ai::transport_policy::StreamDrainPolicy {
        let (provider, api_format) = self.runtime.current_model_key.as_ref().map_or_else(
            || {
                (
                    self.runtime.active_provider,
                    detect_api_format(self.runtime.active_provider, &self.runtime.current_model),
                )
            },
            |key| (key.provider, key.api_format),
        );
        StreamTransportPolicy::resolve(provider, api_format).drain
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
