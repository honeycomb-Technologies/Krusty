//! Keyboard event handlers
//!
//! Main keyboard input handling. Popup-specific key handlers are in popup_keys.rs.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::agent::{AgentEvent, InterruptReason};
use crate::tui::app::{App, Popup, View};
use crate::tui::input::InputAction;
use crate::tui::utils::TitleAction;

impl App {
    /// Main keyboard event dispatcher
    pub fn handle_key(&mut self, key_event: KeyEvent) {
        let code = key_event.code;
        let modifiers = key_event.modifiers;
        let is_press =
            key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat;

        // Handle title editing mode first (ignore Release events)
        if self.runtime.title_editor.is_editing {
            if is_press {
                match self.runtime.title_editor.handle_key(code, modifiers) {
                    TitleAction::Save => self.save_title_edit(),
                    TitleAction::Cancel => self.cancel_title_edit(),
                    TitleAction::Continue => {}
                }
            }
            return;
        }

        // Handle popups first (ignore Release events)
        if self.ui.popup != Popup::None {
            if is_press {
                self.handle_popup_key(code, modifiers);
            }
            return;
        }

        // Handle plugin window focus - route keys to plugin
        if self.ui.plugin_window.focused {
            // Delete unfocuses the plugin window (Esc is used by plugin menus)
            if code == KeyCode::Delete {
                self.ui.plugin_window.unfocus();
                return;
            }
            // Ctrl+Q still quits
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('q') {
                self.runtime.should_quit = true;
                return;
            }
            // Ctrl+P toggles plugin window visibility
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('p') {
                // Load preferred plugin from preferences on first open
                let preferred = self
                    .services
                    .preferences
                    .as_ref()
                    .and_then(|p| p.get_active_plugin());
                self.ui.plugin_window.toggle(preferred.as_deref());
                return;
            }
            // Forward all other keys to the plugin (pass full event for key release detection)
            let area = self.ui.plugin_window.last_area;
            if let Some(plugin) = self.ui.plugin_window.active_plugin_mut() {
                use crate::tui::plugins::PluginEventResult;
                let event = crossterm::event::Event::Key(key_event);
                if let Some(area) = area {
                    match plugin.handle_event(&event, area) {
                        PluginEventResult::Consumed => return,
                        PluginEventResult::Ignored => {}
                    }
                }
            }
            return;
        }

        // Forward keys to focused terminal (except Esc and Ctrl+Q)
        if let Some(idx) = self.runtime.blocks.focused_terminal {
            // ESC unfocuses terminal
            if code == KeyCode::Esc {
                self.runtime.blocks.clear_all_terminal_focus();
                return;
            }
            // Ctrl+Q still quits
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('q') {
                self.runtime.should_quit = true;
                return;
            }
            // Forward all other keys to the terminal
            if let Some(tp) = self.runtime.blocks.terminal.get_mut(idx) {
                let _ = tp.handle_key(key_event);
            }
            return;
        }

        // For regular input handling, ignore Release events (only plugins/terminals need them)
        if !is_press {
            return;
        }

        // Handle autocomplete navigation
        if self.ui.autocomplete.visible {
            match code {
                KeyCode::Tab | KeyCode::Down => {
                    self.ui.autocomplete.next();
                    return;
                }
                KeyCode::Up => {
                    self.ui.autocomplete.prev();
                    return;
                }
                // Only plain Enter selects autocomplete - Shift+Enter should insert newline
                KeyCode::Enter if modifiers.is_empty() => {
                    if let Some(cmd) = self.ui.autocomplete.get_selected() {
                        self.handle_slash_command(cmd.primary);
                        self.ui.input.clear();
                        self.ui.autocomplete.hide();
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.ui.autocomplete.hide();
                    return;
                }
                _ => {}
            }
        }

        // Handle file search navigation - fully isolate arrow keys
        if self.ui.file_search.visible {
            use crate::tui::input::file_search::FileSearchMode;

            match code {
                KeyCode::Down => {
                    self.ui.file_search.next();
                    return;
                }
                KeyCode::Up => {
                    self.ui.file_search.prev();
                    return;
                }
                KeyCode::Right => {
                    if self.ui.file_search.mode == FileSearchMode::Tree {
                        self.ui.file_search.enter_dir();
                    }
                    // Always consume right arrow in file search
                    return;
                }
                KeyCode::Left => {
                    if self.ui.file_search.mode == FileSearchMode::Tree {
                        self.ui.file_search.go_up();
                    }
                    // Always consume left arrow in file search
                    return;
                }
                // Ctrl+F toggles between fuzzy and tree mode
                KeyCode::Char('f') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.ui.file_search.toggle_mode();
                    return;
                }
                KeyCode::Enter if modifiers.is_empty() => {
                    if let Some(path) = self.ui.file_search.get_selected() {
                        let path = path.to_string();
                        // Insert @path into input, replacing the @query
                        self.insert_file_reference(&path);
                        self.ui.file_search.hide();
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.ui.file_search.hide();
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+Q to quit
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('q') {
            self.runtime.should_quit = true;
            return;
        }

        // Ctrl+B to open process list
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('b') {
            self.refresh_process_popup();
            self.ui.popup = Popup::ProcessList;
            return;
        }

        // Ctrl+T to toggle plan/tasks sidebar (only if we have an active plan)
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('t') {
            if self.runtime.active_plan.is_some() {
                self.ui.plan_sidebar.toggle();
            }
            return;
        }

        // Ctrl+P to toggle plugin window
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('p') {
            let preferred = self
                .services
                .preferences
                .as_ref()
                .and_then(|p| p.get_active_plugin());
            self.ui.plugin_window.toggle(preferred.as_deref());
            return;
        }

        // Ctrl+G to toggle work mode (BUILD/PLAN)
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('g') {
            let old_mode = self.ui.work_mode;
            self.ui.work_mode = self.ui.work_mode.toggle();
            tracing::info!(from = ?old_mode, to = ?self.ui.work_mode, "Work mode toggled via Ctrl+G");
            return;
        }

        match self.ui.view {
            View::StartMenu => self.handle_start_menu_key(code, modifiers),
            View::Chat => self.handle_chat_key(code, modifiers),
        }
    }

    /// Handle bracketed paste events (routes to focused terminal, popup, or main input)
    pub fn handle_paste(&mut self, text: String) {
        use crate::tui::popups::auth::AuthState;

        // Forward paste to focused terminal
        if let Some(idx) = self.runtime.blocks.focused_terminal {
            if let Some(tp) = self.runtime.blocks.terminal.get_mut(idx) {
                let _ = tp.write(text.as_bytes());
            }
            return;
        }

        // Route paste to auth popup if active and in input state
        if let Popup::Auth = &self.ui.popup {
            if let AuthState::ApiKeyInput { .. } = &self.ui.popups.auth.state {
                for c in text.trim().chars() {
                    self.ui.popups.auth.add_api_key_char(c);
                }
                return;
            }
        }

        // Route paste to pinch popup if in input state
        if let Popup::Pinch = &self.ui.popup {
            use crate::tui::popups::pinch::PinchStage;
            match &self.ui.popups.pinch.stage {
                PinchStage::PreservationInput { .. } | PinchStage::DirectionInput { .. } => {
                    for c in text.chars() {
                        self.ui.popups.pinch.add_char(c);
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
            self.ui.input.insert_text(&format!("[{}]", trimmed));
        } else {
            self.ui.input.insert_text(&text);
        }
        self.update_autocomplete();
    }

    /// Handle start menu keyboard events
    pub fn handle_start_menu_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Tab - toggle extended thinking mode (when not in autocomplete)
        if code == KeyCode::Tab && !self.ui.autocomplete.visible {
            self.runtime.thinking_enabled = !self.runtime.thinking_enabled;
            tracing::info!(
                "Extended thinking {}",
                if self.runtime.thinking_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            return;
        }

        match self.ui.input.handle_key(code, modifiers) {
            InputAction::Submit(text) => {
                if !text.is_empty() {
                    self.ui.input.clear();
                    self.ui.autocomplete.hide();
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
                self.runtime
                    .pending_clipboard_images
                    .insert(placeholder_id, (width, height, rgba_bytes));
                self.update_autocomplete();
            }
            InputAction::Continue | InputAction::ContentChanged => {
                self.update_autocomplete();
                // Escape clears input on start menu
                if code == KeyCode::Esc && !self.ui.autocomplete.visible {
                    self.ui.input.clear();
                }
            }
        }
    }

    /// Handle chat view keyboard events
    pub fn handle_chat_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // IMPORTANT: Handle decision prompt FIRST (before global Esc handler)
        // This ensures Esc in custom input mode cancels typing, not the whole conversation
        if self.ui.decision_prompt.visible && self.handle_decision_prompt_key(code, modifiers) {
            return;
        }
        // Fall through to input for custom response typing

        // Esc interrupts AI processing (use /home to return to start menu)
        // Only if decision prompt is NOT visible (handled above)
        if code == KeyCode::Esc && !self.ui.autocomplete.visible && !self.ui.decision_prompt.visible
        {
            if self.is_busy() {
                // Cancel the background task
                self.runtime.cancellation.cancel();

                // Emit interrupt event
                self.runtime.event_bus.emit(AgentEvent::Interrupt {
                    turn: self.runtime.agent_state.current_turn,
                    reason: InterruptReason::UserRequested,
                });

                // Update state
                self.runtime.agent_state.interrupt();
                self.runtime.streaming.reset();
                self.stop_streaming();
                self.stop_tool_execution();
                self.runtime
                    .chat
                    .messages
                    .push(("system".to_string(), "Interrupted.".to_string()));
            }
            return;
        }

        // Tab - toggle extended thinking mode (when not in autocomplete)
        // Can toggle during streaming - takes effect after current stream completes
        if code == KeyCode::Tab && !self.ui.autocomplete.visible {
            self.runtime.thinking_enabled = !self.runtime.thinking_enabled;
            tracing::info!(
                "Extended thinking {}",
                if self.runtime.thinking_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            return;
        }

        // Plan sidebar scrolling with Shift+PageUp/PageDown or Shift+Up/Down
        if self.ui.plan_sidebar.visible && modifiers.contains(KeyModifiers::SHIFT) {
            let visible_height = 20; // Approximate visible height
            match code {
                KeyCode::PageUp => {
                    self.ui.plan_sidebar.page_up(visible_height);
                    return;
                }
                KeyCode::PageDown => {
                    self.ui.plan_sidebar.page_down(visible_height);
                    return;
                }
                KeyCode::Up => {
                    self.ui.plan_sidebar.scroll_up();
                    return;
                }
                KeyCode::Down => {
                    self.ui.plan_sidebar.scroll_down(visible_height);
                    return;
                }
                _ => {}
            }
        }

        // PageUp - show older content (decrease offset toward 0/top)
        if code == KeyCode::PageUp {
            self.ui.scroll_system.scroll.scroll_up(5);
            return;
        }
        // PageDown - show newer content (increase offset toward MAX/bottom)
        if code == KeyCode::PageDown {
            self.ui.scroll_system.scroll.scroll_down(5);
            return;
        }

        match self.ui.input.handle_key(code, modifiers) {
            InputAction::Submit(text) => {
                // Check if we're in decision prompt custom input mode
                if self.ui.decision_prompt.visible && self.ui.decision_prompt.custom_input_mode {
                    if !text.is_empty() {
                        let all_done = self.ui.decision_prompt.submit_custom(text);
                        self.ui.input.clear();
                        if all_done {
                            self.handle_decision_prompt_complete();
                        }
                    }
                } else if !text.is_empty() {
                    if self.is_busy() {
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            "Please wait for the current response to complete.".to_string(),
                        ));
                    } else {
                        self.ui.input.clear();
                        self.ui.autocomplete.hide();
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
                self.runtime
                    .pending_clipboard_images
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
        let content = self.ui.input.content();
        // Only show autocomplete for slash commands, not file paths
        // /help = show autocomplete, /home/user/file.pdf = don't show
        if let Some(query) = content.strip_prefix('/') {
            // Check if it looks like a file path (has another / or ends with file extension)
            let is_file_path = query.contains('/')
                || [".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp"]
                    .iter()
                    .any(|ext| query.to_lowercase().ends_with(ext));

            if is_file_path {
                self.ui.autocomplete.hide();
            } else if self.ui.autocomplete.visible {
                self.ui.autocomplete.update(query);
            } else {
                self.ui.autocomplete.show(query);
            }
        } else {
            self.ui.autocomplete.hide();
        }

        // Also update file search
        self.update_file_search();
    }

    /// Update file search based on input (triggered by @)
    pub fn update_file_search(&mut self) {
        let content = self.ui.input.content();

        // Find the last @ in the content
        if let Some(at_pos) = content.rfind('@') {
            // Get the query after @
            let query = &content[at_pos + 1..];

            // Don't trigger if @ is followed by a space or newline (completed reference)
            if query.starts_with(' ') || query.starts_with('\n') {
                self.ui.file_search.hide();
                return;
            }

            // Don't show file search if we're in slash command mode
            if content.starts_with('/') && !self.ui.autocomplete.visible {
                self.ui.file_search.hide();
                return;
            }

            if self.ui.file_search.visible {
                self.ui.file_search.update(query);
            } else {
                self.ui.file_search.show(query);
            }
        } else {
            self.ui.file_search.hide();
        }
    }

    /// Insert a file reference into the input, replacing the @query
    pub fn insert_file_reference(&mut self, path: &str) {
        let content = self.ui.input.content().to_string();

        // Find the last @ and replace from there
        if let Some(at_pos) = content.rfind('@') {
            // Build new content: everything before @ + [path] + space
            // Use brackets so the path is recognized by image_parser and click detection
            let before = &content[..at_pos];
            let new_content = format!("{}[{}] ", before, path);

            self.ui.input.clear();
            self.ui.input.insert_text(&new_content);
        }
    }

    /// Refresh process popup with current process list (non-blocking)
    pub fn refresh_process_popup(&mut self) {
        if let Some(processes) = self.runtime.process_registry.try_list() {
            self.ui.popups.process.update(processes);
        }
    }

    /// Handle keyboard input for decision prompt
    /// Returns true if the key was handled (don't pass to input)
    fn handle_decision_prompt_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        // In custom input mode, only Escape cancels
        if self.ui.decision_prompt.custom_input_mode {
            if code == KeyCode::Esc {
                self.ui.decision_prompt.exit_custom_mode();
                self.ui.input.clear();
                return true;
            }
            // Let Enter be handled by input (submits custom text)
            return false;
        }

        match code {
            // Number keys select option directly
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let num = (c as usize) - ('0' as usize);
                if self.ui.decision_prompt.select_by_number(num) {
                    let all_done = self.ui.decision_prompt.confirm_selection();
                    if all_done {
                        self.handle_decision_prompt_complete();
                    }
                }
                true
            }
            // Arrow navigation
            KeyCode::Up => {
                self.ui.decision_prompt.prev_option();
                true
            }
            KeyCode::Down => {
                self.ui.decision_prompt.next_option();
                true
            }
            // Page navigation for many options
            KeyCode::PageUp => {
                self.ui.decision_prompt.page_up();
                true
            }
            KeyCode::PageDown => {
                self.ui.decision_prompt.page_down();
                true
            }
            // Enter confirms current selection
            KeyCode::Enter if modifiers.is_empty() => {
                let all_done = self.ui.decision_prompt.confirm_selection();
                if all_done {
                    self.handle_decision_prompt_complete();
                }
                true
            }
            // Space toggles for multi-select
            KeyCode::Char(' ') => {
                self.ui.decision_prompt.toggle_current();
                true
            }
            // Backspace goes back to previous question
            KeyCode::Backspace if self.ui.decision_prompt.current_index > 0 => {
                self.ui.decision_prompt.go_back();
                true
            }
            // Escape goes back or dismisses
            KeyCode::Esc => {
                if !self.ui.decision_prompt.go_back() {
                    // No previous question - close the prompt
                    self.ui.decision_prompt.hide();
                }
                true
            }
            // Any other character starts custom input mode
            KeyCode::Char(_) => {
                self.ui.decision_prompt.enter_custom_mode();
                false // Let it pass through to input
            }
            _ => false,
        }
    }

    /// Handle completion of decision prompt (all questions answered)
    pub(crate) fn handle_decision_prompt_complete(&mut self) {
        use crate::tui::components::PromptType;

        let prompt_type = self.ui.decision_prompt.prompt_type.clone();
        let answers = self.ui.decision_prompt.answers.clone();
        let tool_use_id = self.ui.decision_prompt.tool_use_id.clone();

        self.ui.decision_prompt.hide();

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
        use crate::ai::types::{ModelMessage, Role};
        use crate::tui::components::PromptAnswer;

        // Flush any pending tool results that were deferred while prompt was visible
        let pending_results = std::mem::take(&mut self.runtime.pending_tool_results);
        if !pending_results.is_empty() {
            tracing::info!(
                "Flushing {} pending tool results after plan confirmation",
                pending_results.len()
            );
            let msg = ModelMessage {
                role: Role::User,
                content: pending_results,
            };
            self.runtime.chat.conversation.push(msg.clone());
            self.save_model_message(&msg);
        }

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
                        if let Some(ref plan) = self.runtime.active_plan {
                            let title = plan.title.clone();
                            self.runtime.chat.messages.push((
                                "system".to_string(),
                                format!("Plan '{}' abandoned.", title),
                            ));
                        }
                        self.clear_plan();
                        // Don't continue AI conversation - user abandoned
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
        let questions = &self.ui.decision_prompt.questions;

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

        // Include any pending tool results that were deferred while prompt was visible
        let mut content = std::mem::take(&mut self.runtime.pending_tool_results);
        content.push(tool_result);

        // Add to conversation as user message (tool results are sent as user role)
        let msg = ModelMessage {
            role: Role::User,
            content,
        };

        tracing::info!(
            tool_use_id = %tool_use_id,
            answer_count = answers.len(),
            pending_results = msg.content.len() - 1,
            "Sending AskUserQuestion tool result"
        );

        self.runtime.chat.conversation.push(msg.clone());
        self.save_model_message(&msg);

        // Continue AI conversation
        self.send_to_ai();
    }
}
