//! Slash command handler
//!
//! Handles /command parsing and execution.

mod extensions;
mod init;
mod plan;
mod plugins;
mod skills;
mod ui;

pub use init::generate_krab_from_exploration;

use crate::tui::app::App;

impl App {
    /// Handle slash commands.
    pub fn handle_slash_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let command = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();

        if command.starts_with("/skill:") {
            self.handle_explicit_skill_command(cmd);
            return;
        }

        match command.as_str() {
            "/home" => self.open_home_view(),
            "/load" => self.open_session_list_popup(),
            "/model" => self.open_model_popup(),
            "/fast" => match self.toggle_fast_mode() {
                Some(enabled) => {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!(
                            "Fast mode {} for {}.",
                            if enabled { "enabled" } else { "disabled" },
                            self.runtime.current_model
                        ),
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
            "/extensions" => self.handle_extensions_command(&parts),
            "/mcp" => self.open_mcp_browser(),
            "/hooks" => self.open_hooks_popup(),
            "/permissions" | "/perm" => self.show_permission_select(),
            "/update" => self.start_manual_update_check(),
            _ => {
                self.run_agent_extension_command(&command, &parts[1..]);
            }
        }
    }

    fn run_agent_extension_command(&mut self, command: &str, arguments: &[&str]) {
        let Some(manager) = self.services.tool_registry.agent_extension_manager() else {
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), format!("Unknown command: {command}")));
            return;
        };

        let command_name = command.trim_start_matches('/').to_string();
        let argument = arguments.join(" ");
        let context = krusty_core::extensions::ExtensionCallContext::for_turn(
            self.runtime.working_dir.clone(),
            Some(self.runtime.working_dir.clone()),
            self.runtime.current_session_id.clone(),
            Some(self.runtime.current_model.clone()),
            format!("{:?}", self.runtime.permission_mode).to_ascii_lowercase(),
            matches!(self.ui.work_mode, crate::tui::app::WorkMode::Plan),
        );
        let tx = self.runtime.channels.extension_command_sender();
        let display_command = format!("/{command_name}");
        self.runtime.chat.messages.push((
            "system".to_string(),
            format!("Running agent extension command {display_command}…"),
        ));
        tokio::spawn(async move {
            let result = manager
                .execute_command(&command_name, &argument, &context)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(crate::tui::utils::ExtensionCommandUpdate {
                command: display_command,
                result,
            });
        });
    }

    pub(crate) fn poll_agent_extension_commands(&mut self) {
        let mut updates = Vec::new();
        if let Some(receiver) = self.runtime.channels.extension_commands.as_mut() {
            loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
        }
        for update in updates {
            let message = match update.result {
                Ok(serde_json::Value::String(output)) => output,
                Ok(output) => {
                    serde_json::to_string_pretty(&output).unwrap_or_else(|_| output.to_string())
                }
                Err(error) => format!("{} failed: {}", update.command, error),
            };
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), message));
            self.ui.needs_redraw = true;
        }
    }
}
