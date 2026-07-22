//! AI streaming and tool execution handlers
//!
//! Handles sending messages to AI and executing tool calls.
//!
//! This module is split into focused submodules:
//! - `mod.rs`: Input handling and AI communication (via core orchestrator)
//! - `tool_execution.rs`: TUI-specific tool interception (plan tools, AskUser, blocks)

pub(crate) mod tool_execution;

use std::sync::Arc;

use crate::agent::{AgentEvent, OrchestratorServices, RunProvenance, RunSpecBuilder};
use crate::ai::client::config::AnthropicAdaptiveEffort;
use crate::ai::client::{CallOptions, CodexReasoningEffort};
use crate::ai::types::{Content, ContextManagement, ModelMessage, Role, ThinkingConfig};
use crate::paths;
use crate::storage::{ProjectSettings, SessionType};
use crate::tools::registry::ToolRequestPolicy;
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

        let project_settings = ProjectSettings::load(&self.runtime.working_dir);
        if let Err(error) = self.prepare_primary_run_model(&project_settings) {
            self.ui.input.insert_text(&text);
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), error.to_string()));
            return;
        }

        if !self.has_selected_model() {
            self.ui.input.insert_text(&text);
            self.runtime.chat.messages.push((
                "system".to_string(),
                "No model selected. Use /model to choose one.".to_string(),
            ));
            return;
        }

        if !self.is_authenticated() {
            self.ui.input.insert_text(&text);
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Not authenticated. Use /auth to set up API key.".to_string(),
            ));
            return;
        }

        if self.runtime.current_session_id.is_none() && self.create_session(&text).is_none() {
            return;
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
                        let display_name = format!(
                            "clipboard-{}.png",
                            clipboard_id.chars().take(8).collect::<String>()
                        );
                        let preview_path =
                            crate::tui::utils::clipboard::save_clipboard_image_preview(
                                width,
                                height,
                                &rgba_bytes,
                                clipboard_id,
                            )?;
                        self.runtime
                            .attached_files
                            .insert(display_name.clone(), preview_path);
                        content_blocks.push(loaded.content);
                        display_parts.push(format!("[Image: {}]", display_name));
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

        // A prior user interrupt cancels the shared token used by the main loop
        // and registered subagent tools. Reset it before every new turn so
        // later agent tool calls do not inherit an already-cancelled token.
        self.runtime.cancellation.reset();

        let project_settings = ProjectSettings::load(&self.runtime.working_dir);
        let selected_model_metadata = match self.prepare_primary_run_model(&project_settings) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.runtime
                    .chat
                    .messages
                    .push(("system".to_string(), error.to_string()));
                return;
            }
        };

        let client = match self.create_ai_client() {
            Some(c) => c,
            None => {
                let message = if self.has_selected_model() {
                    "No authentication available"
                } else {
                    "No model selected. Use /model to choose one."
                };
                self.runtime
                    .chat
                    .messages
                    .push(("system".to_string(), message.to_string()));
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

        // Build CallOptions (thinking, web tools, etc.)
        let has_active_plan = self
            .services
            .plan_manager
            .as_ref()
            .and_then(|manager| manager.get_active_plan(&session_id).ok())
            .flatten()
            .is_some();
        let tools = ToolRequestPolicy::code(
            self.runtime.permission_mode,
            self.ui.work_mode == crate::tui::app::WorkMode::Plan,
            has_active_plan,
            true,
            project_settings.disabled_tools.as_deref().unwrap_or(&[]),
        )
        .filter(self.services.cached_ai_tools.clone());
        // Keep the request controls on one live registry snapshot. Dynamic
        // catalogs can change wire semantics without changing the model slug.
        let reasoning_format = selected_model_metadata.reasoning_format;
        let reasoning_control = selected_model_metadata.reasoning_control;
        let fast_mode_format = selected_model_metadata.fast_mode;
        let can_use_thinking = self.runtime.thinking_level.is_enabled()
            && reasoning_control != Some(crate::ai::providers::ReasoningControl::OutputOnly);
        let thinking = can_use_thinking.then(ThinkingConfig::default);
        let codex_reasoning_effort =
            if reasoning_control == Some(crate::ai::providers::ReasoningControl::OpenAiEffort) {
                match self.runtime.thinking_level {
                    ThinkingLevel::Off => None,
                    ThinkingLevel::Minimal => Some(CodexReasoningEffort::Minimal),
                    ThinkingLevel::Low => Some(CodexReasoningEffort::Low),
                    ThinkingLevel::Medium => Some(CodexReasoningEffort::Medium),
                    ThinkingLevel::High => Some(CodexReasoningEffort::High),
                    ThinkingLevel::XHigh => Some(CodexReasoningEffort::XHigh),
                    ThinkingLevel::Max | ThinkingLevel::Ultra => Some(CodexReasoningEffort::Max),
                }
            } else {
                None
            };
        let anthropic_adaptive_effort = if reasoning_control
            == Some(crate::ai::providers::ReasoningControl::AnthropicAdaptive)
        {
            match self.runtime.thinking_level {
                ThinkingLevel::Off => None,
                ThinkingLevel::Minimal | ThinkingLevel::Low => Some(AnthropicAdaptiveEffort::Low),
                ThinkingLevel::Medium => Some(AnthropicAdaptiveEffort::Medium),
                ThinkingLevel::High => Some(AnthropicAdaptiveEffort::High),
                ThinkingLevel::XHigh => Some(AnthropicAdaptiveEffort::XHigh),
                ThinkingLevel::Max | ThinkingLevel::Ultra => Some(AnthropicAdaptiveEffort::Max),
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
            // Web tools remain available through deferred dispatch. Keeping
            // hosted web off the default Code request preserves the <=8 tool
            // cache-stable surface.
            web_search: None,
            web_fetch: None,
            session_id: Some(session_id.clone()),
            codex_reasoning_effort,
            codex_parallel_tool_calls: true,
            anthropic_adaptive_effort,
            reasoning_format,
            reasoning_control,
            fast_mode: self.runtime.fast_mode && fast_mode_format.is_some(),
            fast_mode_format,
            ..Default::default()
        };
        let mode_aware_code_tools = options.tools.is_some();

        // Determine if this is a new session (first user message → generate title)
        let is_new_session = self.runtime.chat.conversation.len() <= 1;

        // Resolve one canonical run contract before mutating live TUI state.
        let db_path = paths::config_dir().join("krusty.db");
        let ai_client = Arc::new(client);
        let (delegated_progress_tx, delegated_progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let run_spec = match RunSpecBuilder::new(
            RunProvenance::Tui,
            session_id,
            self.runtime.working_dir.clone(),
            SessionType::Code,
        )
        .project_dir(Some(self.runtime.working_dir.clone()))
        .permission_mode(self.runtime.permission_mode)
        .run_budget(self.runtime.agent_config.primary_run_budget_override())
        .stream_idle_timeout(self.runtime.agent_config.stream_idle_timeout())
        .initial_work_mode(self.ui.work_mode.into())
        .mode_aware_code_tools(mode_aware_code_tools)
        .generate_title(is_new_session)
        .delegated_progress_tx(Some(delegated_progress_tx))
        .call_options(options)
        .build(ai_client.as_ref())
        {
            Ok(run_spec) => run_spec,
            Err(error) => {
                tracing::error!(%error, "Failed to resolve TUI agent run");
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!("Cannot start agent run: {error}"),
                ));
                return;
            }
        };
        let services = OrchestratorServices {
            ai_client,
            tool_registry: self.services.tool_registry.clone(),
            process_registry: self.runtime.process_registry.clone(),
            db_path,
            skills_manager: self.services.skills_manager.clone(),
        };

        self.start_streaming();
        self.runtime.chat.streaming_assistant_idx = None;
        self.runtime.agent_state.start_turn();
        self.runtime.event_bus.emit(AgentEvent::TurnStart {
            turn: self.runtime.agent_state.current_turn,
            message_count: self.runtime.chat.conversation.len(),
        });
        self.runtime.channels.delegated_progress = Some(delegated_progress_rx);

        self.persist_current_work_mode();
        if let (Some(sm), Some(session_id)) = (
            &self.services.session_manager,
            self.runtime.current_session_id.as_deref(),
        ) {
            if let Err(error) =
                sm.update_session_permission_mode(session_id, self.runtime.permission_mode)
            {
                tracing::warn!("Failed to persist permission mode for session: {}", error);
            }
        }

        let conversation = self.runtime.chat.conversation.clone();
        let (event_rx, input_tx) = run_spec.start(services, conversation);

        // Store channels for the event loop to poll
        self.runtime.channels.loop_events = Some(event_rx);
        self.runtime.channels.loop_input = Some(input_tx);
    }
}
