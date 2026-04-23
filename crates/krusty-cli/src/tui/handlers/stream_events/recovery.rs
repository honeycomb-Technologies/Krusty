use crate::agent::loop_events::LoopStopReason;
use crate::ai::types::{Content, ModelMessage};
use crate::tui::app::App;
use crate::tui::handlers::sessions::storage_role_to_api_role;
use crate::tui::state::StreamDrainTelemetry;

impl App {
    /// Reload the conversation from the database.
    ///
    /// Called when the orchestrator finishes to sync the TUI's in-memory
    /// conversation with what the orchestrator saved to the DB.
    pub(super) fn reload_conversation_from_db(&mut self) {
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

    pub(super) fn push_stream_recovery_banner(
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
}
