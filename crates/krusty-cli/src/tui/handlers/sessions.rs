//! Session management handlers
//!
//! Handles creating, saving, loading sessions

use anyhow::Result;

use crate::ai::client::AiClient;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::SessionManager;
use crate::tui::app::{App, WorkMode};
use crate::tui::blocks::{
    BashBlock, EditBlock, ReadBlock, ThinkingBlock, ToolResultBlock, WriteBlock,
};
use crate::tui::state::{hash_content, BlockManager};
use crate::tui::utils::TitleUpdate;

impl App {
    /// Create a new session
    pub fn create_session(&mut self, first_message: &str) -> Option<String> {
        let Some(sm) = &self.services.session_manager else {
            return None;
        };

        // Use fallback title immediately for responsiveness
        let fallback_title = SessionManager::generate_title_from_content(first_message);
        let working_dir_str = self.working_dir.to_string_lossy().into_owned();

        match sm.create_session(
            &fallback_title,
            Some(&self.current_model),
            Some(&working_dir_str),
        ) {
            Ok(id) => {
                tracing::info!("Created new session: {}", id);
                self.current_session_id = Some(id.clone());
                self.session_title = Some(fallback_title);

                // Clear any active plan when starting a new session
                self.clear_plan();

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

    /// Spawn background task to generate AI title
    fn spawn_title_generation(&mut self, session_id: String, first_message: String) {
        // Need an AI client to generate title
        let client = match self.create_title_client() {
            Some(c) => c,
            None => {
                tracing::debug!("No AI client available for title generation");
                return;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.channels.title_update = Some(rx);

        tokio::spawn(async move {
            let title = crate::ai::generate_title(&client, &first_message).await;
            let _ = tx.send(TitleUpdate { session_id, title });
        });
    }

    /// Create AI client for title generation
    fn create_title_client(&self) -> Option<AiClient> {
        self.create_ai_client()
    }

    /// Poll for AI-generated title updates
    pub fn poll_title_generation(&mut self) {
        let rx = match self.channels.title_update.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(update) => {
                self.channels.title_update = None;
                tracing::info!("AI generated title: {}", update.title);

                // Update in-memory title if this is the current session
                if self.current_session_id.as_ref() == Some(&update.session_id) {
                    self.session_title = Some(update.title.clone());
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
                self.channels.title_update = None;
            }
        }
    }

    /// Save current token count to session
    pub fn save_session_token_count(&self) {
        let Some(sm) = &self.services.session_manager else {
            return;
        };
        let Some(session_id) = &self.current_session_id else {
            return;
        };

        if let Err(e) = sm.update_token_count(session_id, self.context_tokens_used) {
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
        let Some(session_id) = &self.current_session_id else {
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

    /// Load a session by ID
    pub fn load_session(&mut self, session_id: &str) -> Result<()> {
        tracing::info!("Loading session: {}", session_id);

        // Load all data from database upfront to avoid borrow conflicts
        let (messages, session_info, ui_states) = {
            let sm = self
                .services
                .session_manager
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No session manager"))?;

            let messages = sm.load_session_messages(session_id)?;
            let session_info = sm.get_session(session_id).ok().flatten();
            let ui_states = sm.load_block_ui_states(session_id);

            (messages, session_info, ui_states)
        };

        tracing::info!("Loaded {} raw messages from database", messages.len());

        // Set session info
        self.session_title = session_info.as_ref().map(|i| i.title.clone());
        let stored_token_count = session_info.as_ref().and_then(|i| i.token_count);

        // Clear current state
        self.chat.messages.clear();
        self.chat.conversation.clear();
        self.blocks = BlockManager::new();
        self.block_ui.clear();
        self.tool_results.clear();
        self.chat.streaming_assistant_idx = None;
        self.current_session_id = Some(session_id.to_string());

        // Load plan for this session (strict 1:1 linkage, no working_dir fallback)
        match self.services.plan_manager.get_plan(session_id) {
            Ok(Some(plan)) => {
                let (completed, total) = plan.progress();
                tracing::info!(
                    "Loaded plan '{}' for session ({}/{})",
                    plan.title,
                    completed,
                    total
                );
                // Resume in Build mode if work has started, Plan mode otherwise
                let resume_mode = if completed > 0 || plan.has_in_progress_tasks() {
                    WorkMode::Build
                } else {
                    WorkMode::Plan
                };
                self.set_plan(plan);
                self.ui.work_mode = resume_mode;
                if !self.plan_sidebar.visible {
                    self.plan_sidebar.toggle();
                }
            }
            Ok(None) => {
                tracing::debug!("No active plan found for session {}", session_id);
                self.clear_plan();
            }
            Err(e) => {
                tracing::warn!("Failed to find active plan: {}", e);
                self.clear_plan();
            }
        }

        // Rebuild conversation from database
        for (role, content_json) in messages {
            tracing::debug!(
                "Processing message - role: {}, content preview: {}...",
                role,
                &content_json.chars().take(50).collect::<String>()
            );

            let api_role = match role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => Role::User,
            };

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

            self.chat.conversation.push(ModelMessage {
                role: api_role,
                content,
            });
        }

        // Fix orphaned tool calls (tool_use without tool_result)
        // This happens when a session is interrupted mid-tool-execution
        self.fix_orphaned_tool_calls();

        // Build caches and display from conversation
        self.build_tool_results_cache();
        self.build_display_from_conversation();

        // Restore persisted block UI states (collapsed/scroll positions)
        if !ui_states.is_empty() {
            tracing::debug!("Restoring {} block UI states", ui_states.len());
            let states: Vec<(String, bool, u16)> = ui_states
                .into_iter()
                .map(|s| (s.block_id, s.collapsed, s.scroll_offset))
                .collect();
            self.block_ui.import(states);
        }

        // Use stored token count if available, otherwise estimate
        self.context_tokens_used = stored_token_count
            .unwrap_or_else(|| Self::estimate_conversation_tokens(&self.chat.conversation));

        tracing::info!(
            "Loaded session {} with {} messages, {} blocks, ~{} tokens",
            session_id,
            self.chat.messages.len(),
            self.blocks.thinking.len()
                + self.blocks.bash.len()
                + self.blocks.read.len()
                + self.blocks.edit.len()
                + self.blocks.write.len(),
            self.context_tokens_used
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
        let Some(session_id) = &self.current_session_id else {
            return;
        };

        let states = self.block_ui.export();
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
            if self.current_session_id.as_deref() == Some(session_id) {
                self.current_session_id = None;
                self.session_title = None;
            }
        }
    }

    /// Build tool results cache from conversation
    fn build_tool_results_cache(&mut self) {
        self.tool_results.clear();

        for msg in &self.chat.conversation {
            for content in &msg.content {
                if let Content::ToolResult {
                    tool_use_id,
                    output,
                    is_error,
                } = content
                {
                    let tool_name = self.find_tool_name_for_id(tool_use_id);
                    let output_str = match output {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    tracing::info!(
                        "Caching tool result: {} (tool={}) is_error={:?}",
                        tool_use_id,
                        tool_name,
                        is_error
                    );
                    self.tool_results.insert_raw(
                        tool_use_id.clone(),
                        &tool_name,
                        &output_str,
                        is_error.unwrap_or(false),
                    );
                }
            }
        }
    }

    /// Find the tool name for a given tool_use_id by searching conversation
    fn find_tool_name_for_id(&self, tool_use_id: &str) -> String {
        for msg in &self.chat.conversation {
            for content in &msg.content {
                if let Content::ToolUse { id, name, .. } = content {
                    if id == tool_use_id {
                        return name.clone();
                    }
                }
            }
        }
        "unknown".to_string()
    }

    /// Build display messages and blocks from conversation
    ///
    /// Messages array format: (role, content) where role determines rendering:
    /// - "user" / "assistant" / "system" → text message
    /// - "thinking" → ThinkingBlock at current thinking index
    /// - "bash" → BashBlock at current bash index
    /// - "read" / "edit" / "write" → respective block types
    /// - "tool_result" → ToolResultBlock (grep/glob/unknown tools)
    fn build_display_from_conversation(&mut self) {
        self.chat.messages.clear();
        self.chat.streaming_assistant_idx = None;
        self.blocks = BlockManager::new();

        for msg in &self.chat.conversation {
            let base_role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            };

            for content in &msg.content {
                match content {
                    Content::Text { text } => {
                        // Skip filler messages (single "." used for API alternation)
                        if text == "." && msg.content.len() == 1 {
                            tracing::debug!("Skipping filler message in display");
                            continue;
                        }
                        // Text messages use the API role
                        self.chat
                            .messages
                            .push((base_role.to_string(), text.clone()));
                    }

                    Content::Thinking {
                        thinking,
                        signature,
                    } => {
                        // Thinking gets its own message entry with "thinking" role
                        self.chat
                            .messages
                            .push(("thinking".to_string(), String::new()));

                        let mut block = ThinkingBlock::new();
                        block.append(thinking);
                        if !signature.is_empty() {
                            block.set_signature(signature.clone());
                        }
                        block.complete();
                        block.set_collapsed(true);

                        let block_id = if signature.is_empty() {
                            hash_content(thinking)
                        } else {
                            signature.clone()
                        };
                        self.block_ui.set_collapsed(&block_id, true);
                        self.blocks.thinking.push(block);
                    }

                    Content::RedactedThinking { .. } => {
                        // Redacted thinking - create a placeholder thinking block
                        self.chat
                            .messages
                            .push(("thinking".to_string(), String::new()));

                        let mut block = ThinkingBlock::new();
                        block.append("[Redacted]");
                        block.complete();
                        block.set_collapsed(true);
                        self.blocks.thinking.push(block);
                    }

                    Content::ToolUse { id, name, input } => {
                        // Each tool use gets its own message entry with tool-specific role
                        match name.to_lowercase().as_str() {
                            "bash" => {
                                self.chat.messages.push(("bash".to_string(), id.clone()));

                                let command = input
                                    .get("command")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let mut block = BashBlock::with_tool_id(command, id.clone());
                                if let Some(result) = self.tool_results.get(id) {
                                    block.append(&result.output);
                                    block.complete(result.exit_code);
                                }
                                block.set_collapsed(false);
                                self.block_ui.set_collapsed(id, false);
                                self.blocks.bash.push(block);
                            }

                            "read" => {
                                self.chat.messages.push(("read".to_string(), id.clone()));

                                let file_path = input
                                    .get("file_path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let mut block = ReadBlock::new(id.clone(), file_path);
                                if let Some(result) = self.tool_results.get(id) {
                                    let line_count = result.output.lines().count();
                                    block.set_content(
                                        result.output.clone(),
                                        line_count,
                                        line_count,
                                    );
                                    block.complete();
                                }
                                block.set_collapsed(true);
                                self.block_ui.set_collapsed(id, true);
                                self.blocks.read.push(block);
                            }

                            "edit" => {
                                let result = self.tool_results.get(id);
                                let is_error = result.map(|r| r.is_error).unwrap_or(false);

                                if is_error {
                                    // Failed edit - show as tool_result with error
                                    self.chat
                                        .messages
                                        .push(("tool_result".to_string(), id.clone()));

                                    let mut block = ToolResultBlock::new(
                                        id.clone(),
                                        "edit".to_string(),
                                        input
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    );
                                    if let Some(r) = result {
                                        block.set_results(&r.output);
                                    }
                                    block.complete();
                                    block.set_collapsed(false);
                                    self.block_ui.set_collapsed(id, false);
                                    self.blocks.tool_result.push(block);
                                } else {
                                    // Successful edit - show as edit block
                                    self.chat.messages.push(("edit".to_string(), id.clone()));

                                    let file_path = input
                                        .get("file_path")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let old_string = input
                                        .get("old_string")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let new_string = input
                                        .get("new_string")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    let mut block = EditBlock::new_pending(file_path.clone());
                                    block.set_tool_use_id(id.clone());
                                    block.set_diff_data(file_path, old_string, new_string, 1);
                                    block.complete();
                                    block.set_collapsed(false);
                                    self.block_ui.set_collapsed(id, false);
                                    self.blocks.edit.push(block);
                                }
                            }

                            "write" => {
                                let result = self.tool_results.get(id);
                                let is_error = result.map(|r| r.is_error).unwrap_or(false);
                                tracing::info!(
                                    "Write tool {} - has_result={}, is_error={}",
                                    id,
                                    result.is_some(),
                                    is_error
                                );

                                if is_error {
                                    // Failed write - show as tool_result with error
                                    self.chat
                                        .messages
                                        .push(("tool_result".to_string(), id.clone()));

                                    let mut block = ToolResultBlock::new(
                                        id.clone(),
                                        "write".to_string(),
                                        input
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    );
                                    if let Some(r) = result {
                                        block.set_results(&r.output);
                                    }
                                    block.complete();
                                    block.set_collapsed(false); // Show errors expanded
                                    self.block_ui.set_collapsed(id, false);
                                    self.blocks.tool_result.push(block);
                                } else {
                                    // Successful write - show as write block
                                    self.chat.messages.push(("write".to_string(), id.clone()));

                                    let file_path = input
                                        .get("file_path")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let file_content = input
                                        .get("content")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    let mut block = WriteBlock::new_pending(file_path.clone());
                                    block.set_tool_use_id(id.clone());
                                    block.set_content(file_path, file_content);
                                    block.complete();
                                    block.set_collapsed(true);
                                    self.block_ui.set_collapsed(id, true);
                                    self.blocks.write.push(block);
                                }
                            }

                            "grep" | "glob" => {
                                self.chat
                                    .messages
                                    .push(("tool_result".to_string(), id.clone()));

                                let pattern = input
                                    .get("pattern")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let mut block =
                                    ToolResultBlock::new(id.clone(), name.clone(), pattern);
                                if let Some(result) = self.tool_results.get(id) {
                                    block.set_results(&result.output);
                                    block.complete();
                                }
                                block.set_collapsed(true);
                                self.block_ui.set_collapsed(id, true);
                                self.blocks.tool_result.push(block);
                            }

                            // Silent tools - don't create any visual element
                            "task_complete" | "enter_plan_mode" | "todowrite" => {
                                // These tools are intentionally silent and should not
                                // create any UI blocks when rebuilding from conversation
                            }

                            _ => {
                                // Unknown tools go to tool_result
                                self.chat
                                    .messages
                                    .push(("tool_result".to_string(), id.clone()));

                                let mut block =
                                    ToolResultBlock::new(id.clone(), name.clone(), String::new());
                                if let Some(result) = self.tool_results.get(id) {
                                    block.set_results(&result.output);
                                    block.complete();
                                }
                                block.set_collapsed(true);
                                self.block_ui.set_collapsed(id, true);
                                self.blocks.tool_result.push(block);
                            }
                        }
                    }

                    Content::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                    } => {
                        // Check if this result has a matching ToolUse in the conversation
                        let has_matching_tool_use = self.chat.conversation.iter().any(|m| {
                            m.content.iter().any(
                                |c| matches!(c, Content::ToolUse { id, .. } if id == tool_use_id),
                            )
                        });

                        if !has_matching_tool_use {
                            // Orphan ToolResult - create a visible block so it's not lost
                            tracing::warn!(
                                "Found orphan ToolResult without matching ToolUse: {}",
                                tool_use_id
                            );

                            self.chat
                                .messages
                                .push(("tool_result".to_string(), tool_use_id.clone()));

                            let output_str = match output {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };

                            let mut block = ToolResultBlock::new(
                                tool_use_id.clone(),
                                "unknown".to_string(),
                                String::new(),
                            );
                            block.set_results(&output_str);
                            if is_error.unwrap_or(false) {
                                block.set_collapsed(false);
                            } else {
                                block.set_collapsed(true);
                            }
                            block.complete();
                            self.block_ui
                                .set_collapsed(tool_use_id, is_error.unwrap_or(false));
                            self.blocks.tool_result.push(block);
                        }
                        // Otherwise: handled via the cache when creating ToolUse blocks
                    }

                    Content::Image { .. } => {
                        // Images displayed as text for now
                        self.chat
                            .messages
                            .push((base_role.to_string(), "[Image]".to_string()));
                    }

                    Content::Document { .. } => {
                        // Documents (PDFs) displayed as text for now
                        self.chat
                            .messages
                            .push((base_role.to_string(), "[PDF]".to_string()));
                    }
                }
            }
        }
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

        for msg in &self.chat.conversation {
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
            self.chat.conversation.push(ModelMessage {
                role: Role::User,
                content: placeholder_results,
            });
        }
    }
}
