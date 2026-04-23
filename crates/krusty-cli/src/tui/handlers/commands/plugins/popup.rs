use crate::tui::app::{App, Popup};

impl App {
    pub(super) fn open_plugins_browser(&mut self) {
        self.refresh_plugins_browser();
        self.ui.popup = Popup::PluginsBrowser;
    }

    pub(crate) fn refresh_plugins_browser(&mut self) {
        self.refresh_plugin_catalog(false);
    }

    pub(crate) fn toggle_selected_plugin_from_popup(&mut self) {
        let Some(plugin_id) = self
            .ui
            .popups
            .plugins
            .selected_plugin_id()
            .map(str::to_string)
        else {
            return;
        };

        let Some(descriptor) = crate::tui::plugins::plugin_descriptor_by_id(&plugin_id) else {
            self.ui.popups.plugins.set_status_message(Some(
                "Selected plugin no longer exists in the catalog.".to_string(),
            ));
            return;
        };

        let Some(manager) = self.services.plugin_manager.as_ref() else {
            self.ui
                .popups
                .plugins
                .set_status_message(Some("Plugin manager unavailable.".to_string()));
            return;
        };

        let target_enabled = !descriptor.enabled;
        match futures::executor::block_on(manager.set_plugin_enabled(&plugin_id, target_enabled)) {
            Ok(()) => {
                let action = if target_enabled {
                    "Enabled"
                } else {
                    "Disabled"
                };
                self.ui
                    .popups
                    .plugins
                    .set_status_message(Some(format!("{} plugin {}", action, plugin_id)));

                if !target_enabled
                    && self.ui.plugin_window.active_plugin_id.as_deref() == Some(plugin_id.as_str())
                {
                    self.ui.plugin_window.set_plugin(None);
                }

                self.refresh_plugin_catalog(true);
            }
            Err(err) => {
                self.ui
                    .popups
                    .plugins
                    .set_status_message(Some(format!("Failed to update {}: {}", plugin_id, err)));
            }
        }
    }
}
