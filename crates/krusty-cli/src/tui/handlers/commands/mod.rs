//! Slash command handler
//!
//! Handles /command parsing and execution.

mod init;
mod plan;
mod plugins;
mod ui;

pub use init::generate_krab_from_exploration;

use crate::tui::app::App;

impl App {
    /// Handle slash commands.
    pub fn handle_slash_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let command = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();

        match command.as_str() {
            "/home" => self.open_home_view(),
            "/load" => self.open_session_list_popup(),
            "/model" => self.open_model_popup(),
            "/fast" => match self.toggle_fast_model() {
                Some(model_id) => {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Fast mode toggled: {}", model_id),
                    ));
                }
                None => {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!(
                            "Fast mode is not available for {}.",
                            self.runtime.current_model
                        ),
                    ));
                }
            },
            "/auth" => self.open_auth_popup(),
            "/ps" | "/processes" => {
                self.refresh_process_popup();
                self.ui.popup = crate::tui::app::Popup::ProcessList;
            }
            "/theme" => {
                self.ui.popups.theme.open(&self.ui.theme_name);
                self.ui.popup = crate::tui::app::Popup::ThemeSelect;
            }
            "/clear" => self.clear_chat_view(),
            "/cmd" => self.ui.popup = crate::tui::app::Popup::Help,
            "/init" => self.handle_init_command(),
            "/pinch" => self.handle_pinch_command(),
            "/terminal" | "/term" | "/shell" => {
                self.handle_terminal_command(parts.get(1).copied());
            }
            "/plan" => self.handle_plan_command(parts.get(1).copied()),
            "/skills" => self.open_skills_browser(),
            "/plugins" => self.handle_plugins_command(&parts),
            "/mcp" => self.open_mcp_browser(),
            "/hooks" => self.open_hooks_popup(),
            "/permissions" | "/perm" => self.show_permission_select(),
            "/update" => self.start_update_check(),
            _ => {
                self.runtime
                    .chat
                    .messages
                    .push(("system".to_string(), format!("Unknown command: {}", cmd)));
            }
        }
    }
}
