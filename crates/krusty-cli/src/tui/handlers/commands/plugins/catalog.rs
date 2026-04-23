use crate::tui::app::App;

impl App {
    pub(crate) fn refresh_plugin_catalog(&mut self, notify_active_updates: bool) -> bool {
        let Some(manager) = self.services.plugin_manager.as_ref() else {
            crate::tui::plugins::set_installed_plugins(Vec::new());
            self.runtime.plugin_versions.clear();
            self.ui.popups.plugins.set_plugins(Vec::new());
            return false;
        };

        let installed = match futures::executor::block_on(manager.list_installed_plugins()) {
            Ok(plugins) => plugins,
            Err(err) => {
                self.ui
                    .popups
                    .plugins
                    .set_status_message(Some(format!("Failed to refresh plugin catalog: {}", err)));
                return false;
            }
        };

        let descriptors: Vec<_> = installed
            .iter()
            .map(crate::tui::plugins::InstalledPluginDescriptor::from_installed)
            .collect();
        crate::tui::plugins::set_installed_plugins(descriptors);
        self.ui.popups.plugins.set_plugins(installed);

        let previous_versions = self.runtime.plugin_versions.clone();
        self.runtime.plugin_versions = crate::tui::plugins::installed_plugin_version_map();

        let changed = self.runtime.plugin_versions != previous_versions;

        if notify_active_updates {
            if let Some(active_id) = self.ui.plugin_window.active_plugin_id.clone() {
                if let (Some(previous), Some(current)) = (
                    previous_versions.get(&active_id),
                    self.runtime.plugin_versions.get(&active_id),
                ) {
                    if previous != current {
                        self.show_toast(crate::tui::components::Toast::info(format!(
                            "Plugin update ready: {} {} -> {}",
                            active_id, previous, current
                        )));
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            format!(
                                "Update detected for active plugin '{}'. Run `/plugins reload {}` to apply.",
                                active_id, active_id
                            ),
                        ));
                    }
                }
            }
        }

        if let Some(active_id) = self.ui.plugin_window.active_plugin_id.clone() {
            if crate::tui::plugins::get_plugin_by_id(&active_id).is_none() {
                self.ui.plugin_window.set_plugin(None);
            }
        }

        changed
    }
}
