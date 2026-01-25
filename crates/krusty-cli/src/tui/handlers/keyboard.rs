//! Keyboard event handlers
//!
//! Main keyboard input handling. Popup-specific key handlers are in popup_keys.rs.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::agent::{AgentEvent, InterruptReason};
use crate::tui::app::{App, Popup, View};
use crate::tui::input::InputAction;
use crate::tui::utils::TitleAction;

impl App {
    /// Main keyboard event dispatcher
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Handle title editing mode first
        if self.title_editor.is_editing {
            match self.title_editor.handle_key(code, modifiers) {
                TitleAction::Save => self.save_title_edit(),
                TitleAction::Cancel => self.cancel_title_edit(),
                TitleAction::Continue => {}
            }
            return;
        }

        // Handle popups first
        if self.ui.popup != Popup::None {
            self.handle_popup_key(code, modifiers);
            return;
        }

        // Forward keys to focused terminal (except Esc and Ctrl+Q)
        if let Some(idx) = self.blocks.focused_terminal {
            // ESC unfocuses terminal
            if code == KeyCode::Esc {
                self.blocks.clear_all_terminal_focus();
                return;
            }
            // Ctrl+Q still quits
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('q') {
                self.should_quit = true;
                return;
            }
            // Forward all other keys to the terminal
            if let Some(tp) = self.blocks.terminal.get_mut(idx) {
                let key_event = crossterm::event::KeyEvent::new(code, modifiers);
                let _ = tp.handle_key(key_event);
            }
            return;
        }

        // Handle autocomplete navigation
        if self.autocomplete.visible {
            match code {
                KeyCode::Tab | KeyCode::Down => {
                    self.autocomplete.next();
                    return;
                }
                KeyCode::Up => {
                    self.autocomplete.prev();
                    return;
                }
                // Only plain Enter selects autocomplete - Shift+Enter should insert newline
                KeyCode::Enter if modifiers.is_empty() => {
                    if let Some(cmd) = self.autocomplete.get_selected() {
                        self.handle_slash_command(cmd.primary);
                        self.input.clear();
                        self.autocomplete.hide();
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.autocomplete.hide();
                    return;
                }
                _ => {}
            }
        }

        // Handle file search navigation - fully isolate arrow keys
        if self.file_search.visible {
            use crate::tui::input::file_search::FileSearchMode;

            match code {
                KeyCode::Down => {
                    self.file_search.next();
                    return;
                }
                KeyCode::Up => {
                    self.file_search.prev();
                    return;
                }
                KeyCode::Right => {
                    if self.file_search.mode == FileSearchMode::Tree {
                        self.file_search.enter_dir();
                    }
                    // Always consume right arrow in file search
                    return;
                }
                KeyCode::Left => {
                    if self.file_search.mode == FileSearchMode::Tree {
                        self.file_search.go_up();
                    }
                    // Always consume left arrow in file search
                    return;
                }
                // Ctrl+F toggles between fuzzy and tree mode
                KeyCode::Char('f') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.file_search.toggle_mode();
                    return;
                }
                KeyCode::Enter if modifiers.is_empty() => {
                    if let Some(path) = self.file_search.get_selected() {
                        let path = path.to_string();
                        // Insert @path into input, replacing the @query
                        self.insert_file_reference(&path);
                        self.file_search.hide();
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.file_search.hide();
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Q to quit
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }

        // Ctrl+B to toggle work mode (BUILD/PLAN)
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('b') {
            let old_mode = self.ui.work_mode;
            self.ui.work_mode = self.ui.work_mode.toggle();
            tracing::info!(from = ?old_mode, to = ?self.ui.work_mode, "Work mode toggled via Ctrl+B");
            return;
        }

        // Ctrl+T to toggle plan/tasks sidebar (only if we have an active plan)
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('t') {
            if self.active_plan.is_some() {
                self.plan_sidebar.toggle();
            }
            return;
        }

        // Ctrl+P to open process list
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('p') {
            self.refresh_process_popup();
            self.ui.popup = Popup::ProcessList;
            return;
        }

        match self.ui.view {
            View::StartMenu => self.handle_start_menu_key(code, modifiers),
            View::Chat => self.handle_chat_key(code, modifiers),
        }
    }

    /// Handle bracketed paste events (routes to popup or main input)
    pub fn handle_paste(&mut self, text: String) {
        use crate::tui::popups::auth::AuthState;

        // Route paste to auth popup if active and in input state
        if let Popup::Auth = &self.ui.popup {
            if let AuthState::ApiKeyInput { .. } = &self.popups.auth.state {
                for c in text.trim().chars() {
                    self.popups.auth.add_api_key_char(c);
                }
                return;
            }
        }

        // Route paste to pinch popup if in input state
        if let Popup::Pinch = &self.ui.popup {
            use crate::tui::popups::pinch::PinchStage;
            match &self.popups.pinch.stage {
                PinchStage::PreservationInput { .. } | PinchStage::DirectionInput { .. } => {
                    for c in text.chars() {
                        self.popups.pinch.add_char(c);
                    }
                    return;
                }
                _ => {}
            }
        }

        // Default: paste to main input
        // Auto-wrap file paths with brackets for preview support
        let trimmed = text.trim();
        let path = std::path::Path::new(trimmed);
        if path.exists() && crate::tools::is_supported_file(path) {
            self.input.insert_text(&format!("[{}]", trimmed));
        } else {
            self.input.insert_text(&text);
        }
        self.update_autocomplete();
    }

    /// Handle start menu keyboard events
    pub fn handle_start_menu_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Tab - toggle extended thinking mode (when not in autocomplete)
        if code == KeyCode::Tab && !self.autocomplete.visible {
            self.thinking_enabled = !self.thinking_enabled;
            tracing::info!(
                "Extended thinking {}",
                if self.thinking_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            return;
        }

        match self.input.handle_key(code, modifiers) {
            InputAction::Submit(text) => {
                if !text.is_empty() {
                    self.input.clear();
                    self.autocomplete.hide();
                    self.handle_input_submit(text);
                }
            }
            InputAction::ImagePasted {
                width,
                height,
                rgba_bytes,
                placeholder_id,
            } => {
                // Store clipboard image for later resolution
                self.pending_clipboard_images
                    .insert(placeholder_id, (width, height, rgba_bytes));
                self.update_autocomplete();
            }
            InputAction::Continue | InputAction::ContentChanged => {
                self.update_autocomplete();
                // Escape clears input on start menu
                if code == KeyCode::Esc && !self.autocomplete.visible {
                    self.input.clear();
                }
            }
        }
    }

    /// Handle chat view keyboard events
    pub fn handle_chat_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // IMPORTANT: Handle decision prompt FIRST (before global Esc handler)
        // This ensures Esc in custom input mode cancels typing, not the whole conversation
        if self.decision_prompt.visible && self.handle_decision_prompt_key(code, modifiers) {
            return;
        }
        // Fall through to input for custom response typing

        // Esc interrupts AI processing (use /home to return to start menu)
        // Only if decision prompt is NOT visible (handled above)
        if code == KeyCode::Esc && !self.autocomplete.visible && !self.decision_prompt.visible {
            if self.is_busy() {
                // Cancel the background task
                self.cancellation.cancel();

                // Emit interrupt event
                self.event_bus.emit(AgentEvent::Interrupt {
                    turn: self.agent_state.current_turn,
                    reason: InterruptReason::UserRequested,
                });

                // Update state
                self.agent_state.interrupt();
                self.streaming.reset();
                self.stop_streaming();
                self.stop_tool_execution();
                self.chat
                    .messages
                    .push(("system".to_string(), "Interrupted.".to_string()));
            }
            return;
        }

        // Tab - toggle extended thinking mode (when not in autocomplete)
        // Can toggle during streaming - takes effect after current stream completes
        if code == KeyCode::Tab && !self.autocomplete.visible {
            self.thinking_enabled = !self.thinking_enabled;
            tracing::info!(
                "Extended thinking {}",
                if self.thinking_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            return;
        }

        // Plan sidebar scrolling with Shift+PageUp/PageDown or Shift+Up/Down
        if self.plan_sidebar.visible && modifiers.contains(KeyModifiers::SHIFT) {
            let visible_height = 20; // Approximate visible height
            match code {
                KeyCode::PageUp => {
                    self.plan_sidebar.page_up(visible_height);
                    return;
                }
                KeyCode::PageDown => {
                    self.plan_sidebar.page_down(visible_height);
                    return;
                }
                KeyCode::Up => {
                    self.plan_sidebar.scroll_up();
                    return;
                }
                KeyCode::Down => {
                    self.plan_sidebar.scroll_down(visible_height);
                    return;
                }
                _ => {}
            }
        }

        // PageUp - show older content (decrease offset toward 0/top)
        if code == KeyCode::PageUp {
            self.scroll_system.scroll.scroll_up(5);
            return;
        }
        // PageDown - show newer content (increase offset toward MAX/bottom)
        if code == KeyCode::PageDown {
            self.scroll_system.scroll.scroll_down(5);
            return;
        }

        match self.input.handle_key(code, modifiers) {
            InputAction::Submit(text) => {
                // Check if we're in decision prompt custom input mode
                if self.decision_prompt.visible && self.decision_prompt.custom_input_mode {
                    if !text.is_empty() {
                        let all_done = self.decision_prompt.submit_custom(text);
                        self.input.clear();
                        if all_done {
                            self.handle_decision_prompt_complete();
                        }
                    }
                } else if !text.is_empty() {
                    if self.is_busy() {
                        self.chat.messages.push((
                            "system".to_string(),
                            "Please wait for the current response to complete.".to_string(),
                        ));
                    } else {
                        self.input.clear();
                        self.autocomplete.hide();
                        self.handle_input_submit(text);
                    }
                }
            }
            InputAction::ImagePasted {
                width,
                height,
                rgba_bytes,
                placeholder_id,
            } => {
                // Store clipboard image for later resolution
                self.pending_clipboard_images
                    .insert(placeholder_id, (width, height, rgba_bytes));
                self.update_autocomplete();
            }
            InputAction::Continue | InputAction::ContentChanged => {
                self.update_autocomplete();
            }
        }
    }

    /// Update autocomplete suggestions based on input
    pub fn update_autocomplete(&mut self) {
        let content = self.input.content();
        // Only show autocomplete for slash commands, not file paths
        // /help = show autocomplete, /home/user/file.pdf = don't show
        if let Some(query) = content.strip_prefix('/') {
            // Check if it looks like a file path (has another / or ends with file extension)
            let is_file_path = query.contains('/')
                || [".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp"]
                    .iter()
                    .any(|ext| query.to_lowercase().ends_with(ext));

            if is_file_path {
                self.autocomplete.hide();
            } else if self.autocomplete.visible {
                self.autocomplete.update(query);
            } else {
                self.autocomplete.show(query);
            }
        } else {
            self.autocomplete.hide();
        }

        // Also update file search
        self.update_file_search();
    }

    /// Update file search based on input (triggered by @)
    pub fn update_file_search(&mut self) {
        let content = self.input.content();

        // Find the last @ in the content
        if let Some(at_pos) = content.rfind('@') {
            // Get the query after @
            let query = &content[at_pos + 1..];

            // Don't trigger if @ is followed by a space or newline (completed reference)
            if query.starts_with(' ') || query.starts_with('\n') {
                self.file_search.hide();
                return;
            }

            // Don't show file search if we're in slash command mode
            if content.starts_with('/') && !self.autocomplete.visible {
                self.file_search.hide();
                return;
            }

            if self.file_search.visible {
                self.file_search.update(query);
            } else {
                self.file_search.show(query);
            }
        } else {
            self.file_search.hide();
        }
    }

    /// Insert a file reference into the input, replacing the @query
    pub fn insert_file_reference(&mut self, path: &str) {
        let content = self.input.content().to_string();

        // Find the last @ and replace from there
        if let Some(at_pos) = content.rfind('@') {
            // Build new content: everything before @ + [path] + space
            // Use brackets so the path is recognized by image_parser and click detection
            let before = &content[..at_pos];
            let new_content = format!("{}[{}] ", before, path);

            self.input.clear();
            self.input.insert_text(&new_content);
        }
    }

    /// Refresh process popup with current process list (non-blocking)
    pub fn refresh_process_popup(&mut self) {
        if let Some(processes) = self.process_registry.try_list() {
            self.popups.process.update(processes);
        }
    }

    /// Handle keyboard input for decision prompt
    /// Returns true if the key was handled (don't pass to input)
    fn handle_decision_prompt_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        // In custom input mode, only Escape cancels
        if self.decision_prompt.custom_input_mode {
            if code == KeyCode::Esc {
                self.decision_prompt.exit_custom_mode();
                self.input.clear();
                return true;
            }
            // Let Enter be handled by input (submits custom text)
            return false;
        }

        match code {
            // Number keys select option directly
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let num = (c as usize) - ('0' as usize);
                if self.decision_prompt.select_by_number(num) {
                    let all_done = self.decision_prompt.confirm_selection();
                    if all_done {
                        self.handle_decision_prompt_complete();
                    }
                }
                true
            }
            // Arrow navigation
            KeyCode::Up => {
                self.decision_prompt.prev_option();
                true
            }
            KeyCode::Down => {
                self.decision_prompt.next_option();
                true
            }
            // Page navigation for many options
            KeyCode::PageUp => {
                self.decision_prompt.page_up();
                true
            }
            KeyCode::PageDown => {
                self.decision_prompt.page_down();
                true
            }
            // Enter confirms current selection
            KeyCode::Enter if modifiers.is_empty() => {
                let all_done = self.decision_prompt.confirm_selection();
                if all_done {
                    self.handle_decision_prompt_complete();
                }
                true
            }
            // Space toggles for multi-select
            KeyCode::Char(' ') => {
                self.decision_prompt.toggle_current();
                true
            }
            // Backspace goes back to previous question
            KeyCode::Backspace if self.decision_prompt.current_index > 0 => {
                self.decision_prompt.go_back();
                true
            }
            // Escape goes back or dismisses
            KeyCode::Esc => {
                if !self.decision_prompt.go_back() {
                    // No previous question - close the prompt
                    self.decision_prompt.hide();
                }
                true
            }
            // Any other character starts custom input mode
            KeyCode::Char(_) => {
                self.decision_prompt.enter_custom_mode();
                false // Let it pass through to input
            }
            _ => false,
        }
    }

    /// Handle completion of decision prompt (all questions answered)
    pub(crate) fn handle_decision_prompt_complete(&mut self) {
        use crate::tui::components::PromptType;

        let prompt_type = self.decision_prompt.prompt_type.clone();
        let answers = self.decision_prompt.answers.clone();
        let tool_use_id = self.decision_prompt.tool_use_id.clone();

        self.decision_prompt.hide();

        match prompt_type {
            PromptType::PlanConfirm => {
                self.handle_plan_confirm_answer(&answers);
            }
            PromptType::AskUserQuestion => {
                if let Some(id) = tool_use_id {
                    self.handle_ask_user_answer(id, &answers);
                }
            }
        }
    }

    /// Handle plan confirmation answer
    fn handle_plan_confirm_answer(&mut self, answers: &[crate::tui::components::PromptAnswer]) {
        use crate::tui::components::PromptAnswer;

        match answers.first() {
            Some(PromptAnswer::Selected(idx)) => {
                match idx {
                    0 => {
                        // Execute - switch to BUILD mode and auto-start
                        self.ui.work_mode = crate::tui::app::WorkMode::Build;

                        // Auto-send execute message to Claude
                        let execute_msg =
                            "Begin executing the plan, starting with Task 1.1".to_string();
                        self.handle_input_submit(execute_msg);
                    }
                    1 => {
                        // Abandon - clear plan
                        if let Some(ref plan) = self.active_plan {
                            let title = plan.title.clone();
                            self.chat.messages.push((
                                "system".to_string(),
                                format!("Plan '{}' abandoned.", title),
                            ));
                        }
                        self.active_plan = None;
                        self.plan_sidebar.reset();
                        self.ui.work_mode = crate::tui::app::WorkMode::Build;
                    }
                    _ => {}
                }
            }
            Some(PromptAnswer::Custom(text)) => {
                // Custom text = modification instructions, send to Claude (stays in PLAN mode)
                self.handle_input_submit(text.clone());
            }
            _ => {}
        }
    }

    /// Handle AskUserQuestion tool answer
    fn handle_ask_user_answer(
        &mut self,
        tool_use_id: String,
        answers: &[crate::tui::components::PromptAnswer],
    ) {
        use crate::ai::types::{Content, ModelMessage, Role};
        use crate::tui::components::PromptAnswer;

        // Build answers object matching the questions
        let mut answers_json = serde_json::Map::new();
        let questions = &self.decision_prompt.questions;

        for (i, answer) in answers.iter().enumerate() {
            let question = questions.get(i);
            let key = question
                .map(|q| q.header.clone())
                .unwrap_or_else(|| format!("question_{}", i + 1));

            let value = match answer {
                PromptAnswer::Selected(idx) => {
                    // Get the label of the selected option
                    question
                        .and_then(|q| q.options.get(*idx))
                        .map(|opt| serde_json::Value::String(opt.label.clone()))
                        .unwrap_or_else(|| serde_json::Value::String(format!("Option {}", idx + 1)))
                }
                PromptAnswer::MultiSelected(indices) => {
                    // Get labels of all selected options
                    let labels: Vec<serde_json::Value> = indices
                        .iter()
                        .filter_map(|idx| {
                            question
                                .and_then(|q| q.options.get(*idx))
                                .map(|opt| serde_json::Value::String(opt.label.clone()))
                        })
                        .collect();
                    serde_json::Value::Array(labels)
                }
                PromptAnswer::Custom(text) => serde_json::Value::String(text.clone()),
            };

            answers_json.insert(key, value);
        }

        // Create tool result - output must be a string, not an object
        let result_json = serde_json::json!({ "answers": answers_json });
        let tool_result = Content::ToolResult {
            tool_use_id: tool_use_id.clone(),
            output: serde_json::Value::String(result_json.to_string()),
            is_error: None,
        };

        // Add to conversation as user message (tool results are sent as user role)
        let msg = ModelMessage {
            role: Role::User,
            content: vec![tool_result],
        };

        tracing::info!(
            tool_use_id = %tool_use_id,
            answer_count = answers.len(),
            "Sending AskUserQuestion tool result"
        );

        self.chat.conversation.push(msg.clone());
        self.save_model_message(&msg);

        // Continue AI conversation
        self.send_to_ai();
    }
}
