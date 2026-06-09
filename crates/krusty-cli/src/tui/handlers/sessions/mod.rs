//! Session management handlers
//!
//! Handles creating, saving, loading sessions

mod display;
mod loading;
mod recovery;
#[cfg(test)]
mod tests;
mod title;

use crate::ai::types::{ModelMessage, Role};
use crate::storage::SessionManager;
use crate::tui::app::App;

pub(crate) fn storage_role_to_api_role(role: &str) -> Role {
    match role {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "system" => Role::System,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

impl App {
    /// Create a new session
    pub fn create_session(&mut self, first_message: &str) -> Option<String> {
        let Some(sm) = &self.services.session_manager else {
            return None;
        };

        // Use fallback title immediately for responsiveness
        let fallback_title = SessionManager::generate_title_from_content(first_message);
        let working_dir_str = self.runtime.working_dir.to_string_lossy().into_owned();

        let selected_model = self.runtime.current_model.trim();
        match sm.create_session(
            &fallback_title,
            (!selected_model.is_empty()).then_some(selected_model),
            Some(&working_dir_str),
        ) {
            Ok(id) => {
                if let Err(error) =
                    sm.update_session_permission_mode(&id, self.runtime.permission_mode)
                {
                    tracing::warn!(
                        "Failed to persist permission mode for new session: {}",
                        error
                    );
                }
                tracing::info!("Created new session: {}", id);
                self.runtime.current_session_id = Some(id.clone());
                self.runtime.session_title = Some(fallback_title);
                self.runtime.agent_state.reset();
                self.runtime.pending_clipboard_images.clear();
                self.runtime.attached_files.clear();

                // Clear any active plan when starting a new session
                self.clear_active_plan();
                self.persist_current_work_mode();

                // Spawn AI title generation in background
                self.spawn_title_generation(id.clone(), first_message.to_string());

                Some(id)
            }
            Err(e) => {
                tracing::warn!("Failed to create session: {}", e);
                None
            }
        }
    }
    /// Poll for AI-generated title updates
    pub fn poll_title_generation(&mut self) {
        let rx = match self.runtime.channels.title_update.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(update) => {
                self.runtime.channels.title_update = None;
                tracing::info!("AI generated title: {}", update.title);

                // Update in-memory title if this is the current session
                if self.runtime.current_session_id.as_ref() == Some(&update.session_id) {
                    self.runtime.session_title = Some(update.title.clone());
                }

                // Persist to database
                if let Some(sm) = &self.services.session_manager {
                    if let Err(e) = sm.update_session_title(&update.session_id, &update.title) {
                        tracing::warn!("Failed to update session title: {}", e);
                    }
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                // Still waiting
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                // Task failed/cancelled
                self.runtime.channels.title_update = None;
            }
        }
    }

    /// Save current token count to session
    pub fn save_session_token_count(&self) {
        let Some(sm) = &self.services.session_manager else {
            return;
        };
        let Some(session_id) = &self.runtime.current_session_id else {
            return;
        };

        if let Err(e) = sm.update_token_count(session_id, self.runtime.context_tokens_used) {
            tracing::warn!("Failed to update token count: {}", e);
        }
    }

    /// Save a message to the current session
    /// Content is serialized as JSON for full fidelity (supports tools, images, etc.)
    pub fn save_model_message(&self, message: &ModelMessage) {
        let Some(sm) = &self.services.session_manager else {
            tracing::warn!("Cannot save message: no session manager");
            return;
        };
        let Some(session_id) = &self.runtime.current_session_id else {
            tracing::warn!("Cannot save message: no current session");
            return;
        };

        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
        };

        // Serialize the content as JSON
        let content_json = match serde_json::to_string(&message.content) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("Failed to serialize message content: {}", e);
                return;
            }
        };

        tracing::info!(
            "Saving {} message to session {}: {}...",
            role,
            session_id,
            &content_json.chars().take(50).collect::<String>()
        );

        if let Err(e) = sm.save_message(session_id, role, &content_json) {
            tracing::warn!("Failed to save message: {}", e);
        }
    }
    /// Get sessions for a specific directory
    pub fn list_sessions_for_directory(&self, dir: &str) -> Vec<crate::storage::SessionInfo> {
        self.services
            .session_manager
            .as_ref()
            .and_then(|sm| sm.list_sessions(Some(dir)).ok())
            .unwrap_or_default()
    }

    /// Save all block UI states to the database
    pub fn save_block_ui_states(&self) {
        let Some(sm) = &self.services.session_manager else {
            return;
        };
        let Some(session_id) = &self.runtime.current_session_id else {
            return;
        };

        let states = self.ui.block_ui.export();
        for (block_id, collapsed, scroll_offset) in states {
            if let Err(e) = sm.save_block_ui_state(session_id, &block_id, collapsed, scroll_offset)
            {
                tracing::warn!("Failed to save block UI state for {}: {}", block_id, e);
            }
        }
        tracing::debug!("Saved block UI states for session {}", session_id);
    }

    /// Delete a session by ID
    pub fn delete_session(&mut self, session_id: &str) {
        let Some(sm) = &self.services.session_manager else {
            return;
        };

        if let Err(e) = sm.delete_session(session_id) {
            tracing::warn!("Failed to delete session: {}", e);
        } else {
            tracing::info!("Deleted session: {}", session_id);
            // If we deleted the current session, clear it
            if self.runtime.current_session_id.as_deref() == Some(session_id) {
                self.runtime.current_session_id = None;
                self.runtime.session_title = None;
            }
        }
    }
}
