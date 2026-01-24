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

        if self.view == View::StartMenu {
            self.view = View::Chat;
        }

        if !self.is_authenticated() {
            self.input.insert_text(&text);
            self.messages.push((
                "system".to_string(),
                "Not authenticated. Use /auth to set up API key.".to_string(),
            ));
            return;
        }

        if self.current_session_id.is_none() {
            self.create_session(&text);
        }

        let (content_blocks, display_text) = match self.build_user_content(&text) {
            Ok(result) => result,
            Err(e) => {
                self.messages
                    .push(("system".to_string(), format!("Error: {}", e)));
                return;
            }
        };

        self.messages.push(("user".to_string(), display_text));
        let user_msg = ModelMessage {
            role: Role::User,
            content: content_blocks,
        };
        self.conversation.push(user_msg.clone());
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

        let segments = parse_input(text, &self.working_dir);
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
                    self.attached_files
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
                        self.pending_clipboard_images.remove(clipboard_id)
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
        if self.is_busy() {
            tracing::warn!("send_to_ai called while already busy - skipping");
            return;
        }

        tracing::info!(
            "=== send_to_ai START === conversation_len={}",
            self.conversation.len()
        );

        // Log conversation structure for debugging
        for (i, msg) in self.conversation.iter().enumerate() {
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
            .agent_config
            .exceeded_max_turns(self.agent_state.current_turn)
        {
            self.event_bus.emit(AgentEvent::Interrupt {
                turn: self.agent_state.current_turn,
                reason: InterruptReason::MaxTurnsReached,
            });
            self.messages.push((
                "system".to_string(),
                format!(
                    "Max turns ({}) reached. Use /home to start a new session.",
                    self.agent_config.max_turns.unwrap_or(0)
                ),
            ));
            return;
        }

        let Some(ref _client) = self.ai_client else {
            self.messages
                .push(("system".to_string(), "No AI client configured".to_string()));
            return;
        };

        self.start_streaming();
        self.streaming.reset();

        self.agent_state.start_turn();
        self.event_bus.emit(AgentEvent::TurnStart {
            turn: self.agent_state.current_turn,
            message_count: self.conversation.len(),
        });

        // Build context
        let diagnostics_context = self.build_diagnostics_context();
        let plan_context = self.build_plan_context();
        let skills_context = self.build_skills_context();
        let project_context = self.build_project_context();

        if !plan_context.is_empty() {
            tracing::info!(
                "Injecting plan context ({} chars) into conversation",
                plan_context.len()
            );
        }
        if !skills_context.is_empty() {
            tracing::debug!(
                "Injecting skills context ({} chars) into conversation",
                skills_context.len()
            );
        }
        if !project_context.is_empty() {
            tracing::info!(
                "Injecting project context ({} chars) into conversation",
                project_context.len()
            );
        }

        let has_thinking_conversation = self.thinking_enabled
            && self.conversation.iter().any(|msg| {
                msg.role == Role::Assistant
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::Thinking { .. }))
            });

        let mut conversation = self.conversation.clone();
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

        // Inject diagnostics as user/assistant pair
        if !diagnostics_context.is_empty() && !has_thinking_conversation {
            let system_count = conversation
                .iter()
                .take_while(|m| m.role == Role::System)
                .count();
            let insert_pos = system_count;
            conversation.insert(
                insert_pos,
                ModelMessage {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: diagnostics_context,
                    }],
                },
            );
            conversation.insert(
                insert_pos + 1,
                ModelMessage {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: "I understand the current diagnostics. I'll take them into account."
                            .to_string(),
                    }],
                },
            );
        }

        let client = match self.create_ai_client() {
            Some(c) => c,
            None => {
                self.messages.push((
                    "system".to_string(),
                    "No authentication available".to_string(),
                ));
                return;
            }
        };

        let tools = self.services.cached_ai_tools.clone();
        let tool_names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        tracing::info!("Sending {} tools to API: {:?}", tools.len(), tool_names);

        let can_use_thinking = self.thinking_enabled;
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

        self.cancellation.reset();
        let cancel_token = self.cancellation.child_token();

        let (tx, rx) = mpsc::unbounded_channel();
        self.streaming.start_stream(rx);

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
