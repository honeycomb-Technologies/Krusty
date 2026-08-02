use crate::tui::app::App;

impl App {
    pub(super) fn handle_extensions_command(&mut self, parts: &[&str]) {
        let Some(manager) = self.services.tool_registry.agent_extension_manager() else {
            self.extension_system_message("Agent extension host is not initialized.");
            return;
        };

        let action = parts.get(1).copied().unwrap_or("status");
        let result = match action {
            "status" => manager.project_trust_status().map(|status| {
                format!(
                    "Project extensions: {} for {}. Use /extensions trust or /extensions revoke.",
                    if status.trusted { "trusted" } else { "not trusted" },
                    status.project_path.display()
                )
            }),
            "trust" => futures::executor::block_on(manager.set_project_trusted_and_refresh(
                true,
                &self.services.tool_registry,
            ))
            .map(|status| {
                format!(
                    "Trusted project agent extensions for {}. Executable project code is now allowed subject to .mitsuro/settings.json restrictions.",
                    status.project_path.display()
                )
            }),
            "revoke" | "untrust" => futures::executor::block_on(
                manager.set_project_trusted_and_refresh(false, &self.services.tool_registry),
            )
            .map(|status| {
                format!(
                    "Revoked project agent-extension trust for {} and unloaded project extensions.",
                    status.project_path.display()
                )
            }),
            "reload" => futures::executor::block_on(
                manager.refresh_and_register(&self.services.tool_registry),
            )
            .map(|()| "Agent extensions reloaded.".to_string()),
            _ => Err(anyhow::anyhow!(
                "Usage: /extensions [status|trust|revoke|reload]"
            )),
        };

        match result {
            Ok(message) => {
                let commands = futures::executor::block_on(manager.commands());
                self.ui.autocomplete.set_extension_commands(
                    commands
                        .into_iter()
                        .map(|command| (command.name, command.description)),
                );
                self.services.cached_ai_tools =
                    futures::executor::block_on(self.services.tool_registry.get_ai_tools_all());
                self.extension_system_message(message);
            }
            Err(error) => {
                self.extension_system_message(format!("Agent extension operation failed: {error}"))
            }
        }
    }

    fn extension_system_message(&mut self, message: impl Into<String>) {
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), message.into()));
        self.ui.needs_redraw = true;
    }
}
