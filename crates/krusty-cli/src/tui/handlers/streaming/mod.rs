//! AI streaming and tool execution handlers
//!
//! Handles sending messages to AI and executing tool calls.
//!
//! This module is split into focused submodules:
//! - `mod.rs`: Input handling and AI communication (via core orchestrator)
//! - `tool_execution.rs`: TUI-specific tool interception (plan tools, AskUser, blocks)

pub(crate) mod tool_execution;

use std::sync::Arc;

use crate::agent::{AgentEvent, InterruptReason, OrchestratorConfig, OrchestratorServices};
use crate::ai::client::config::AnthropicAdaptiveEffort;
use crate::ai::client::{CallOptions, CodexReasoningEffort};
use crate::ai::types::{
    Content, ContextManagement, ModelMessage, Role, ThinkingConfig, WebFetchConfig, WebSearchConfig,
};
use crate::paths;
use crate::tools::{load_from_clipboard_rgba, load_from_path, load_from_url};
use crate::tui::app::{App, ThinkingLevel, View};
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

    /// Send the current conversation to the AI via the core orchestrator.
    ///
    /// The orchestrator runs the entire agentic loop (AI call → tools → repeat)
    /// as a spawned task. Events are consumed by `process_loop_events()` in the
    /// main event loop.
    pub fn send_to_ai(&mut self) {
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

        let Some(session_id) = self.runtime.current_session_id.clone() else {
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), "No active session".to_string()));
            return;
        };

        self.start_streaming();
        self.runtime.chat.streaming_assistant_idx = None;

        self.runtime.agent_state.start_turn();
        self.runtime.event_bus.emit(AgentEvent::TurnStart {
            turn: self.runtime.agent_state.current_turn,
            message_count: self.runtime.chat.conversation.len(),
        });

        // Build CallOptions (thinking, web tools, etc.)
        let tools = self.services.cached_ai_tools.clone();
        let can_use_thinking = self.runtime.thinking_level.is_enabled();
        let thinking = can_use_thinking.then(ThinkingConfig::default);
        let codex_reasoning_effort = if self.is_codex_thinking_mode() {
            match self.runtime.thinking_level {
                ThinkingLevel::Off => None,
                ThinkingLevel::Low => Some(CodexReasoningEffort::Low),
                ThinkingLevel::Medium => Some(CodexReasoningEffort::Medium),
                ThinkingLevel::High => Some(CodexReasoningEffort::High),
                ThinkingLevel::XHigh => Some(CodexReasoningEffort::XHigh),
            }
        } else {
            None
        };
        let anthropic_adaptive_effort = if self.is_anthropic_opus_thinking_mode() {
            match self.runtime.thinking_level {
                ThinkingLevel::Off => None,
                ThinkingLevel::Low => Some(AnthropicAdaptiveEffort::Low),
                ThinkingLevel::Medium => Some(AnthropicAdaptiveEffort::Medium),
                ThinkingLevel::High | ThinkingLevel::XHigh => Some(AnthropicAdaptiveEffort::High),
            }
        } else {
            None
        };
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
            session_id: Some(session_id.clone()),
            codex_reasoning_effort,
            codex_parallel_tool_calls: true,
            anthropic_adaptive_effort,
            ..Default::default()
        };

        // Determine if this is a new session (first user message → generate title)
        let is_new_session = self.runtime.chat.conversation.len() <= 1;

        // Create orchestrator services and config
        let db_path = paths::config_dir().join("krusty.db");
        let services = OrchestratorServices {
            ai_client: Arc::new(client),
            tool_registry: self.services.tool_registry.clone(),
            process_registry: self.runtime.process_registry.clone(),
            db_path,
            skills_manager: self.services.skills_manager.clone(),
        };

        let config = OrchestratorConfig {
            session_id,
            working_dir: self.runtime.working_dir.clone(),
            permission_mode: self.runtime.permission_mode,
            max_iterations: self.runtime.agent_config.max_turns.unwrap_or(50).min(50),
            user_id: None,
            initial_work_mode: self.ui.work_mode.into(),
            generate_title: is_new_session,
        };

        let conversation = self.runtime.chat.conversation.clone();
        let orchestrator = crate::agent::AgenticOrchestrator::new(services, config);
        let (event_rx, input_tx) = orchestrator.run(conversation, options);

        // Store channels for the event loop to poll
        self.runtime.channels.loop_events = Some(event_rx);
        self.runtime.channels.loop_input = Some(input_tx);
    }
}
