//! AI streaming and tool execution handlers
//!
//! Handles sending messages to AI and executing tool calls.
//!
//! This module is split into focused submodules:
//! - `mod.rs`: Input handling and AI communication
//! - `tool_execution.rs`: Tool call execution and result processing
//! - `context_building.rs`: Context injection (diagnostics, plans, skills)

mod context_building;
mod tool_execution;

use tokio::sync::mpsc;

use crate::agent::{AgentEvent, InterruptReason};
use crate::ai::client::CallOptions;
use crate::ai::streaming::StreamPart;
use crate::ai::types::{
    Content, ContextManagement, ModelMessage, Role, ThinkingConfig, WebFetchConfig, WebSearchConfig,
};
use crate::tools::{load_from_clipboard_rgba, load_from_path, load_from_url};
use crate::tui::app::{App, View};
use crate::tui::input::{has_image_references, parse_input, InputSegment};

/// Maximum number of files allowed per message
const MAX_FILES_PER_MESSAGE: usize = 20;

/// Check if file count exceeds the maximum
fn check_file_limit(count: usize) -> anyhow::Result<()> {
    if count > MAX_FILES_PER_MESSAGE {
        anyhow::bail!("Too many files (max {} per message)", MAX_FILES_PER_MESSAGE);
    }
    Ok(())
}

impl App {
    /// Handle user input submission (message or command)
    pub fn handle_input_submit(&mut self, text: String) {
        // Check if this is a slash command vs a file path
        if text.starts_with('/') && !Self::looks_like_file_path(&text) {
            self.handle_slash_command(&text);
            return;
        }

        if self.ui.view == View::StartMenu {
            self.ui.view = View::Chat;
        }

        if !self.is_authenticated() {
            self.ui.input.insert_text(&text);
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Not authenticated. Use /auth to set up API key.".to_string(),
            ));
            return;
        }

        if self.runtime.current_session_id.is_none() {
            self.create_session(&text);
        }

        let (content_blocks, display_text) = match self.build_user_content(&text) {
            Ok(result) => result,
            Err(e) => {
                self.runtime
                    .chat
                    .messages
                    .push(("system".to_string(), format!("Error: {}", e)));
                return;
            }
        };

        self.runtime
            .chat
            .messages
            .push(("user".to_string(), display_text));
        let user_msg = ModelMessage {
            role: Role::User,
            content: content_blocks,
        };
        self.runtime.chat.conversation.push(user_msg.clone());
        self.save_model_message(&user_msg);
        self.send_to_ai();
    }

    /// Build user message content from input text
    /// Parses file references and loads images/documents
    fn build_user_content(&mut self, text: &str) -> anyhow::Result<(Vec<Content>, String)> {
        // Fast path: no file references
        if !has_image_references(text) {
            return Ok((
                vec![Content::Text {
                    text: text.to_string(),
                }],
                text.to_string(),
            ));
        }

        let segments = parse_input(text, &self.runtime.working_dir);
        let mut content_blocks = Vec::new();
        let mut display_parts = Vec::new();
        let mut file_count = 0;

        for segment in segments {
            match segment {
                InputSegment::Text(t) => {
                    if !t.is_empty() {
                        content_blocks.push(Content::Text { text: t.clone() });
                        display_parts.push(t);
                    }
                }
                InputSegment::ImagePath(path) => {
                    file_count += 1;
                    check_file_limit(file_count)?;
                    let loaded = load_from_path(&path)?;
                    let file_type = match &loaded.content {
                        Content::Document { .. } => "PDF",
                        _ => "Image",
                    };
                    // Track the file for preview lookup
                    self.runtime
                        .attached_files
                        .insert(loaded.display_name.clone(), path.clone());
                    display_parts.push(format!("[{}: {}]", file_type, loaded.display_name));
                    content_blocks.push(loaded.content);
                }
                InputSegment::ImageUrl(url) => {
                    file_count += 1;
                    check_file_limit(file_count)?;
                    let loaded = load_from_url(&url)?;
                    content_blocks.push(loaded.content);
                    display_parts.push(format!("[Image: {}]", loaded.display_name));
                }
                InputSegment::ClipboardImage(id) => {
                    // Extract clipboard id (format: "clipboard:uuid")
                    let clipboard_id = id.strip_prefix("clipboard:").unwrap_or(&id);
                    if let Some((width, height, rgba_bytes)) =
                        self.runtime.pending_clipboard_images.remove(clipboard_id)
                    {
                        file_count += 1;
                        check_file_limit(file_count)?;
                        let loaded = load_from_clipboard_rgba(width, height, &rgba_bytes)?;
                        content_blocks.push(loaded.content);
                        display_parts.push(format!("[Image: {}]", loaded.display_name));
                    } else {
                        // Clipboard image not found, treat as text
                        display_parts.push(format!("[{}]", id));
                        content_blocks.push(Content::Text {
                            text: format!("[{}]", id),
                        });
                    }
                }
            }
        }

        let display_text = display_parts.join("");
        Ok((content_blocks, display_text))
    }

    /// Check if text looks like a file path rather than a slash command
    /// Returns true for paths like /home/user/file.pdf, false for /help
    fn looks_like_file_path(text: &str) -> bool {
        // Get the first "word" (text before any space)
        let first_word = text.split_whitespace().next().unwrap_or(text);

        // If there's a second / in the path, it's likely a file path
        // /home/user = file path, /help = command
        if first_word.chars().skip(1).any(|c| c == '/') {
            return true;
        }

        // If it ends with a supported file extension, it's a file path
        let extensions = [".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp"];
        let lower = first_word.to_lowercase();
        extensions.iter().any(|ext| lower.ends_with(ext))
    }

    /// Send the current conversation to the AI and start streaming response
    pub fn send_to_ai(&mut self) {
        // Block sending while decision prompt is visible (waiting for user input)
        if self.ui.decision_prompt.visible {
            tracing::info!("send_to_ai blocked - waiting for user decision");
            return;
        }

        if self.is_busy() {
            tracing::warn!("send_to_ai called while already busy - skipping");
            return;
        }

        tracing::info!(
            "=== send_to_ai START === conversation_len={}",
            self.runtime.chat.conversation.len()
        );

        // Log conversation structure for debugging
        for (i, msg) in self.runtime.chat.conversation.iter().enumerate() {
            let content_summary: Vec<String> = msg
                .content
                .iter()
                .map(|c| match c {
                    Content::Text { text } => format!("Text({})", text.len()),
                    Content::ToolUse { id, name, .. } => format!("ToolUse({}, {})", name, id),
                    Content::ToolResult { tool_use_id, .. } => {
                        format!("ToolResult({})", tool_use_id)
                    }
                    Content::Image { .. } => "Image".to_string(),
                    Content::Document { .. } => "Document".to_string(),
                    Content::Thinking { thinking, .. } => format!("Thinking({})", thinking.len()),
                    Content::RedactedThinking { .. } => "RedactedThinking".to_string(),
                })
                .collect();
            tracing::debug!(
                "  conversation[{}] {:?}: {:?}",
                i,
                msg.role,
                content_summary
            );
        }

        // Check max turns limit
        if self
            .runtime
            .agent_config
            .exceeded_max_turns(self.runtime.agent_state.current_turn)
        {
            self.runtime.event_bus.emit(AgentEvent::Interrupt {
                turn: self.runtime.agent_state.current_turn,
                reason: InterruptReason::MaxTurnsReached,
            });
            self.runtime.chat.messages.push((
                "system".to_string(),
                format!(
                    "Max turns ({}) reached. Use /home to start a new session.",
                    self.runtime.agent_config.max_turns.unwrap_or(0)
                ),
            ));
            return;
        }

        let Some(ref _client) = self.runtime.ai_client else {
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), "No AI client configured".to_string()));
            return;
        };

        self.start_streaming();
        self.runtime.streaming.reset();

        self.runtime.agent_state.start_turn();
        self.runtime.event_bus.emit(AgentEvent::TurnStart {
            turn: self.runtime.agent_state.current_turn,
            message_count: self.runtime.chat.conversation.len(),
        });

        // Build context
        let plan_context = self.build_plan_context();
        let skills_context = self.build_skills_context();
        let project_context = self.build_project_context();

        // Log all context injection for monitoring
        if !plan_context.is_empty() {
            tracing::info!(chars = plan_context.len(), "Context: plan");
        }
        if !skills_context.is_empty() {
            tracing::info!(chars = skills_context.len(), "Context: skills");
        }
        if !project_context.is_empty() {
            tracing::info!(chars = project_context.len(), "Context: project");
        }
        let _has_thinking_conversation = self.runtime.thinking_enabled
            && self.runtime.chat.conversation.iter().any(|msg| {
                msg.role == Role::Assistant
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::Thinking { .. }))
            });

        let mut conversation = self.runtime.chat.conversation.clone();
        let mut system_insert_count = 0;

        // Inject project context FIRST
        if !project_context.is_empty() {
            conversation.insert(
                system_insert_count,
                ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text {
                        text: project_context,
                    }],
                },
            );
            system_insert_count += 1;
        }

        // Inject plan context
        if !plan_context.is_empty() {
            conversation.insert(
                system_insert_count,
                ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text { text: plan_context }],
                },
            );
            system_insert_count += 1;
        }

        // Inject skills context
        if !skills_context.is_empty() {
            conversation.insert(
                system_insert_count,
                ModelMessage {
                    role: Role::System,
                    content: vec![Content::Text {
                        text: skills_context,
                    }],
                },
            );
        }

        let client = match self.create_ai_client() {
            Some(c) => c,
            None => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    "No authentication available".to_string(),
                ));
                return;
            }
        };

        let tools = self.services.cached_ai_tools.clone();
        let tool_names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        tracing::info!("Sending {} tools to API: {:?}", tools.len(), tool_names);

        let can_use_thinking = self.runtime.thinking_enabled;
        let thinking = can_use_thinking.then(ThinkingConfig::default);

        let context_management = match (can_use_thinking, !tools.is_empty()) {
            (true, _) => Some(ContextManagement::default_for_thinking_and_tools()),
            (false, true) => Some(ContextManagement::default_tools_only()),
            (false, false) => None,
        };

        let options = CallOptions {
            tools: (!tools.is_empty()).then_some(tools),
            thinking,
            enable_caching: true,
            context_management,
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };

        self.runtime.cancellation.reset();
        let cancel_token = self.runtime.cancellation.child_token();

        let (tx, rx) = mpsc::unbounded_channel();
        self.runtime.streaming.start_stream(rx);

        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    let _ = tx.send(StreamPart::Error {
                        error: "Interrupted by user".to_string()
                    });
                }
                result = client.call_streaming(conversation, &options) => {
                    match result {
                        Ok(mut api_rx) => {
                            loop {
                                tokio::select! {
                                    _ = cancel_token.cancelled() => {
                                        let _ = tx.send(StreamPart::Error {
                                            error: "Interrupted by user".to_string()
                                        });
                                        break;
                                    }
                                    part = api_rx.recv() => {
                                        match part {
                                            Some(p) => {
                                                if tx.send(p).is_err() {
                                                    break;
                                                }
                                            }
                                            None => break,
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(StreamPart::Error { error: e.to_string() });
                        }
                    }
                }
            }
        });
    }
}
