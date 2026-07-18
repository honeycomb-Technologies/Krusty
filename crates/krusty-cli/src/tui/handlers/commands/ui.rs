use crate::tui::app::{App, Popup, View};

impl App {
    pub(super) fn open_home_view(&mut self) {
        self.runtime.current_session_id = None;
        self.runtime.chat.messages.clear();
        self.runtime.chat.streaming_assistant_idx = None;
        self.runtime.chat.conversation.clear();
        self.runtime.agent_state.reset();
        self.clear_plan();
        self.ui.view = View::StartMenu;
    }

    pub(super) fn open_session_list_popup(&mut self) {
        let current_dir = self.runtime.working_dir.to_string_lossy().into_owned();
        self.ui.popups.session.set_current_directory(&current_dir);

        let sessions: Vec<_> = self
            .list_sessions_for_directory(&current_dir)
            .into_iter()
            .map(|s| crate::tui::popups::session_list::SessionInfo {
                id: s.id,
                title: s.title,
                updated_at: s.updated_at.format("%Y-%m-%d %H:%M").to_string(),
            })
            .collect();

        self.ui.popups.session.set_sessions(sessions);
        self.ui.popup = Popup::SessionList;
    }

    pub(super) fn open_model_popup(&mut self) {
        let configured = self.configured_providers();

        if let Some((recent_models, models_by_provider)) = self
            .services
            .model_registry
            .try_get_organized_models(&configured)
        {
            let models_vec: Vec<_> = crate::ai::providers::ProviderId::all()
                .iter()
                .filter_map(|id| {
                    models_by_provider
                        .get(id)
                        .map(|models| (*id, models.clone()))
                })
                .collect();

            self.ui.popups.model.set_models(recent_models, models_vec);
        }

        self.ui.popup = Popup::ModelSelect;

        // Refresh every dynamic provider with credentials so the picker matches
        // the server model list, not only the currently active provider.
        self.refresh_stale_dynamic_model_catalogs();
    }

    pub(super) fn open_auth_popup(&mut self) {
        self.ui.popups.auth.reset();
        self.ui
            .popups
            .auth
            .set_configured_providers(self.configured_providers());
        self.ui.popup = Popup::Auth;
    }

    pub(super) fn clear_chat_view(&mut self) {
        self.runtime.chat.messages.clear();
        self.runtime.chat.streaming_assistant_idx = None;
        self.runtime.blocks = crate::tui::state::BlockManager::new();
    }

    /// Handle /pinch command — compact the current session in place.
    pub(super) fn handle_pinch_command(&mut self) {
        self.start_manual_compaction(false);
    }

    /// Handle /terminal command - spawn an interactive PTY terminal.
    pub(super) fn handle_terminal_command(&mut self, shell: Option<&str>) {
        let shell_cmd = shell.unwrap_or("bash");

        match crate::tui::blocks::TerminalPane::spawn(shell_cmd, 24, 80) {
            Ok(mut pane) => {
                let process_id = format!("terminal-{}", uuid::Uuid::new_v4());
                let process_id_clone = process_id.clone();
                let pid = pane.get_child_pid();
                pane.set_process_id(process_id.clone());

                let registry = self.runtime.process_registry.clone();
                let working_dir = self.runtime.working_dir.clone();
                let cmd = shell_cmd.to_string();
                tokio::spawn(async move {
                    registry
                        .register_external(
                            process_id,
                            format!("terminal: {}", cmd),
                            Some("Interactive PTY terminal".to_string()),
                            pid,
                            working_dir,
                        )
                        .await;
                });

                self.runtime.blocks.terminal.push(pane);
                self.runtime
                    .chat
                    .messages
                    .push(("terminal".to_string(), process_id_clone));
                self.ui.scroll_system.scroll.request_scroll_to_bottom();
            }
            Err(e) => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!("Failed to spawn terminal: {}", e),
                ));
            }
        }
    }

    /// Open skills browser popup.
    pub(super) fn open_skills_browser(&mut self) {
        let skills = match self.services.skills_manager.try_write() {
            Ok(mut guard) => guard.list_skills(),
            Err(_) => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    "Skills manager is busy, try again.".to_string(),
                ));
                return;
            }
        };

        self.ui.popups.skills.set_skills(skills);
        self.ui.popup = Popup::SkillsBrowser;
    }

    /// Refresh skills in the browser.
    pub fn refresh_skills_browser(&mut self) {
        let selected = self
            .ui
            .popups
            .skills
            .selected_skill()
            .map(|skill| skill.name.clone());
        let skills = match self.services.skills_manager.try_write() {
            Ok(mut guard) => {
                guard.refresh();
                guard.list_skills()
            }
            Err(_) => return,
        };
        self.ui
            .popups
            .skills
            .set_skills_preserving(skills, selected.as_deref());
    }

    /// Open MCP server browser popup.
    pub(super) fn open_mcp_browser(&mut self) {
        self.refresh_mcp_popup();
        self.ui.popup = Popup::McpBrowser;
    }

    /// Refresh MCP servers in the browser popup.
    pub fn refresh_mcp_popup(&mut self) {
        let mcp = self.services.mcp_manager.clone();
        let servers = futures::executor::block_on(mcp.list_servers());
        self.ui.popups.mcp.update(servers);
    }

    /// Show permission mode selection popup.
    pub(super) fn show_permission_select(&mut self) {
        let is_supervised = self.runtime.permission_mode
            == krusty_core::tools::registry::PermissionMode::Supervised;
        self.ui
            .decision_prompt
            .show_permission_select(is_supervised);
    }

    /// Open hooks configuration popup.
    pub(super) fn open_hooks_popup(&mut self) {
        let hooks: Vec<_> = futures::executor::block_on(async {
            self.services
                .user_hook_manager
                .read()
                .await
                .hooks()
                .to_vec()
        });
        self.ui.popups.hooks.set_hooks(hooks);
        self.ui.popup = Popup::Hooks;
    }
}
