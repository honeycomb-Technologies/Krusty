use anyhow::Result;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::tui::app::{App, WorkMode};
use crate::tui::state::BlockManager;

use super::storage_role_to_api_role;

impl App {
    /// Load a session by ID
    pub fn load_session(&mut self, session_id: &str) -> Result<()> {
        tracing::info!("Loading session: {}", session_id);

        // Load all data from database upfront to avoid borrow conflicts
        let (messages, session_info, ui_states, recovery_state) = {
            let sm = self
                .services
                .session_manager
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No session manager"))?;

            let messages = sm.load_session_messages(session_id)?;
            let session_info = sm.get_session(session_id).ok().flatten();
            let ui_states = sm.load_block_ui_states(session_id);
            let recovery_state = sm.load_recovery_state(session_id).ok().flatten();

            (messages, session_info, ui_states, recovery_state)
        };

        tracing::info!("Loaded {} raw messages from database", messages.len());

        // Set session info
        self.runtime.session_title = session_info.as_ref().map(|i| i.title.clone());
        if let Some(info) = session_info.as_ref() {
            self.runtime.permission_mode = info.permission_mode;
        }
        let stored_token_count = session_info.as_ref().and_then(|i| i.token_count);
        if let Some(model) = session_info
            .as_ref()
            .and_then(|info| info.model.as_deref())
            .map(str::trim)
            .filter(|model| !model.is_empty())
        {
            self.runtime.current_model = model.to_string();
            self.persist_current_model_selection();
            self.sync_active_provider_to_current_model();
            let _ = futures::executor::block_on(self.try_load_auth());
        }

        // Clear current state
        self.runtime.chat.messages.clear();
        self.runtime.chat.conversation.clear();
        self.runtime.blocks = BlockManager::new();
        self.ui.block_ui.clear();
        self.runtime.tool_results.clear();
        self.runtime.chat.streaming_assistant_idx = None;
        self.runtime.pending_clipboard_images.clear();
        self.runtime.attached_files.clear();
        self.runtime.live_tool_calls.clear();
        self.runtime.current_session_id = Some(session_id.to_string());
        self.runtime.pending_pinched_session_id = None;
        self.runtime.agent_state.reset();

        // Load plan for this session (strict 1:1 linkage, no working_dir fallback)
        let stored_mode = session_info
            .as_ref()
            .map(|info| info.work_mode)
            .unwrap_or_default();

        if let Some(ref pm) = self.services.plan_manager {
            match pm.get_lifecycle_state(session_id, stored_mode) {
                Ok(lifecycle) => {
                    self.ui.work_mode = WorkMode::from(lifecycle.effective_work_mode);

                    if let Some(plan) = lifecycle.active_plan {
                        let (completed, total) = plan.progress();
                        tracing::info!(
                            "Loaded active plan '{}' for session ({}/{})",
                            plan.title,
                            completed,
                            total
                        );
                        self.set_plan(plan);
                        if !self.ui.plan_sidebar.visible {
                            self.ui.plan_sidebar.toggle();
                        }
                    } else {
                        tracing::debug!("No active plan found for session {}", session_id);
                        self.clear_active_plan();
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to resolve plan lifecycle: {}", e);
                    self.clear_active_plan();
                    self.ui.work_mode = WorkMode::from(stored_mode);
                }
            }
        } else {
            self.clear_active_plan();
            self.ui.work_mode = WorkMode::from(stored_mode);
        }

        // Rebuild conversation from database
        for (role, content_json) in messages {
            tracing::debug!(
                "Processing message - role: {}, content preview: {}...",
                role,
                &content_json.chars().take(50).collect::<String>()
            );

            // Multi-tier deserialization for robust session loading:
            // 1. Try Vec<Content> (current format)
            // 2. Try single Content object (alternate format)
            // 3. Fallback to plain text (legacy format)
            let content: Vec<Content> = serde_json::from_str::<Vec<Content>>(&content_json)
                .inspect(|c| {
                    tracing::debug!("Deserialized as JSON array with {} items", c.len());
                })
                .or_else(|_| {
                    serde_json::from_str::<Content>(&content_json).map(|c| {
                        tracing::debug!("Deserialized as single Content object");
                        vec![c]
                    })
                })
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        "Failed to parse content JSON ({}), treating as plain text. Preview: {}...",
                        e,
                        &content_json.chars().take(100).collect::<String>()
                    );
                    vec![Content::Text {
                        text: content_json.clone(),
                    }]
                });

            self.runtime.chat.conversation.push(ModelMessage {
                role: storage_role_to_api_role(role.as_str()),
                content,
            });
        }

        // Fix orphaned tool calls (tool_use without tool_result)
        // This happens when a session is interrupted mid-tool-execution
        self.fix_orphaned_tool_calls();

        // Build caches and display from conversation
        self.build_tool_results_cache();
        self.build_display_from_conversation();
        if let Some(recovery_state) = recovery_state.as_ref() {
            self.push_recovery_notice(recovery_state, None);
        }

        // Restore persisted block UI states (collapsed/scroll positions)
        if !ui_states.is_empty() {
            tracing::debug!("Restoring {} block UI states", ui_states.len());
            let states: Vec<(String, bool, u16)> = ui_states
                .into_iter()
                .map(|s| (s.block_id, s.collapsed, s.scroll_offset))
                .collect();
            self.ui.block_ui.import(states);
        }

        // Use stored token count if available, otherwise estimate
        self.runtime.context_tokens_used = stored_token_count
            .unwrap_or_else(|| Self::estimate_conversation_tokens(&self.runtime.chat.conversation));

        tracing::info!(
            "Loaded session {} with {} messages, {} blocks, ~{} tokens",
            session_id,
            self.runtime.chat.messages.len(),
            self.runtime.blocks.thinking.len()
                + self.runtime.blocks.bash.len()
                + self.runtime.blocks.read.len()
                + self.runtime.blocks.edit.len()
                + self.runtime.blocks.write.len(),
            self.runtime.context_tokens_used
        );
        Ok(())
    }

    /// Estimate token count for a conversation (rough approximation: ~4 chars per token)
    /// Used as fallback for legacy sessions without stored token count
    fn estimate_conversation_tokens(conversation: &[ModelMessage]) -> usize {
        let total_chars: usize = conversation
            .iter()
            .flat_map(|msg| &msg.content)
            .map(|content| match content {
                Content::Text { text } => text.len(),
                Content::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
                Content::ToolResult { output, .. } => output.to_string().len(),
                Content::Image { .. } => 1000, // Images use significant tokens
                Content::Document { .. } => 5000, // PDFs use significant tokens
                Content::Thinking { thinking, .. } => thinking.len(),
                Content::RedactedThinking { .. } => 100, // Redacted thinking placeholder
            })
            .sum();

        // Rough estimate: ~4 characters per token
        total_chars / 4
    }
    /// Fix orphaned tool calls by injecting placeholder results
    ///
    /// When a session is interrupted mid-tool-execution, there may be ToolUse
    /// content without corresponding ToolResult. This causes API errors like
    /// "No tool output found for function call". This function detects and
    /// patches these orphans by inserting placeholder results.
    fn fix_orphaned_tool_calls(&mut self) {
        use std::collections::HashSet;

        // Collect all tool_use IDs and tool_result IDs
        let mut tool_use_ids: HashSet<String> = HashSet::new();
        let mut tool_result_ids: HashSet<String> = HashSet::new();

        for msg in &self.runtime.chat.conversation {
            for content in &msg.content {
                match content {
                    Content::ToolUse { id, .. } => {
                        tool_use_ids.insert(id.clone());
                    }
                    Content::ToolResult { tool_use_id, .. } => {
                        tool_result_ids.insert(tool_use_id.clone());
                    }
                    _ => {}
                }
            }
        }

        // Find orphaned tool calls
        let orphaned: Vec<String> = tool_use_ids.difference(&tool_result_ids).cloned().collect();

        if orphaned.is_empty() {
            return;
        }

        tracing::warn!(
            "Found {} orphaned tool calls without results, injecting placeholders: {:?}",
            orphaned.len(),
            orphaned
        );

        // Create placeholder tool results for each orphan
        let placeholder_results: Vec<Content> = orphaned
            .into_iter()
            .map(|id| Content::ToolResult {
                tool_use_id: id,
                output: serde_json::Value::String(
                    "[Session interrupted - tool execution was cancelled]".to_string(),
                ),
                is_error: Some(true),
            })
            .collect();

        // Append as a user message with tool results (Anthropic style)
        if !placeholder_results.is_empty() {
            self.runtime.chat.conversation.push(ModelMessage {
                role: Role::User,
                content: placeholder_results,
            });
        }
    }
}
