//! Popup keyboard event handlers
//!
//! Handles keyboard input for all popup dialogs.
//! Each popup type has its own module for focused, testable handlers.

mod auth;
mod file_preview;
mod hooks;
mod mcp;
mod pinch;
mod plugins;
mod process;
mod skills;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::tui::app::{App, Popup};

impl App {
    /// Handle keyboard events when a popup is open
    pub fn handle_popup_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match &self.ui.popup {
            Popup::Help => match code {
                KeyCode::Esc => self.ui.popup = Popup::None,
                KeyCode::Tab => self.ui.popups.help.next_tab(),
                _ => {}
            },
            Popup::ThemeSelect => {
                match code {
                    KeyCode::Esc => {
                        // Restore original theme on cancel
                        self.restore_original_theme();
                        self.ui.popup = Popup::None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.ui.popups.theme.prev();
                        // Live preview on navigation
                        if let Some(name) = self.ui.popups.theme.get_selected_theme_name() {
                            self.preview_theme(&name);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.ui.popups.theme.next();
                        // Live preview on navigation
                        if let Some(name) = self.ui.popups.theme.get_selected_theme_name() {
                            self.preview_theme(&name);
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(name) = self.ui.popups.theme.get_selected_theme_name() {
                            self.set_theme(&name); // Apply AND save
                            self.ui.popup = Popup::None;
                        }
                    }
                    _ => {}
                }
            }
            Popup::ModelSelect => {
                self.handle_model_select_key(code);
            }
            Popup::SessionList => {
                self.handle_session_list_key(code);
            }
            Popup::Auth => {
                self.handle_auth_popup_key(code, modifiers);
            }
            Popup::ProcessList => {
                self.handle_process_popup_key(code);
            }
            Popup::PluginsBrowser => {
                self.handle_plugins_popup_key(code);
            }
            Popup::Pinch => {
                self.handle_pinch_popup_key(code, modifiers);
            }
            Popup::FilePreview => {
                self.handle_file_preview_popup_key(code);
            }
            Popup::SkillsBrowser => {
                self.handle_skills_popup_key(code);
            }
            Popup::McpBrowser => {
                self.handle_mcp_popup_key(code);
            }
            Popup::Hooks => {
                self.handle_hooks_popup_key(code);
            }
            Popup::None => {}
        }
    }

    /// Handle model select popup keys
    fn handle_model_select_key(&mut self, code: KeyCode) {
        if self.ui.popups.model.search_active {
            match code {
                KeyCode::Esc => self.ui.popups.model.toggle_search(),
                KeyCode::Enter => {
                    if self.ui.popups.model.has_selectable_results() {
                        self.ui.popups.model.close_search();
                    } else {
                        self.confirm_model_selection();
                    }
                }
                KeyCode::Backspace => self.ui.popups.model.backspace_search(),
                KeyCode::Char(c) => self.ui.popups.model.add_search_char(c),
                _ => {}
            }
        } else {
            match code {
                KeyCode::Esc => self.ui.popup = Popup::None,
                KeyCode::Up | KeyCode::Char('k') => self.ui.popups.model.prev(),
                KeyCode::Down | KeyCode::Char('j') => self.ui.popups.model.next(),
                KeyCode::Char('i') | KeyCode::Char('/') => self.ui.popups.model.toggle_search(),
                KeyCode::Enter => self.confirm_model_selection(),
                _ => {}
            }
        }
    }

    /// Confirm model selection
    fn confirm_model_selection(&mut self) {
        let selection = self
            .ui
            .popups
            .model
            .get_selected_metadata()
            .cloned()
            .map(|metadata| (metadata, false))
            .or_else(|| {
                self.ui
                    .popups
                    .model
                    .custom_model_metadata(self.runtime.active_provider)
                    .map(|metadata| (metadata, true))
            });

        if let Some((metadata, is_custom)) = selection {
            // Check if current context exceeds new model's limit
            if self.runtime.context_tokens_used > metadata.context_window {
                let used_k = self.runtime.context_tokens_used as f64 / 1000.0;
                let max_k = metadata.context_window as f64 / 1000.0;
                self.ui.popups.model.set_error(format!(
                    "Context too large ({:.0}k) for {} ({:.0}k max). Clear conversation or choose a larger model.",
                    used_k, metadata.display_name, max_k
                ));
            } else {
                self.apply_model_selection(metadata, is_custom);
                self.ui.popup = Popup::None;
            }
        }
    }

    /// Handle session list popup keys
    fn handle_session_list_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.ui.popup = Popup::None,
            KeyCode::Up | KeyCode::Char('k') => self.ui.popups.session.prev(),
            KeyCode::Down | KeyCode::Char('j') => self.ui.popups.session.next(),
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(session) = self.ui.popups.session.delete_selected() {
                    self.delete_session(&session.id);
                }
            }
            KeyCode::Enter => {
                if let Some(session) = self.ui.popups.session.get_selected_session() {
                    let session_id = session.id.clone();
                    self.save_block_ui_states();
                    if let Err(e) = self.load_session(&session_id) {
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            format!("Failed to load session: {}", e),
                        ));
                    } else {
                        self.ui.pending_view_change = Some(crate::tui::app::View::Chat);
                    }
                    self.ui.popup = Popup::None;
                }
            }
            _ => {}
        }
    }
}
