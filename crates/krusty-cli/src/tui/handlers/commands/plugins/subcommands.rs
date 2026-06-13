use crate::tui::app::App;

impl App {
    pub(in crate::tui::handlers::commands) fn handle_plugins_command(&mut self, parts: &[&str]) {
        let Some(manager) = self.services.plugin_manager.as_ref() else {
            self.runtime.chat.messages.push((
                "system".to_string(),
                "Plugin manager is unavailable in this build.".to_string(),
            ));
            return;
        };

        match parts.get(1).copied() {
            None => self.open_plugins_browser(),
            Some("list") => {
                let installed = futures::executor::block_on(manager.list_installed_plugins());
                match installed {
                    Ok(plugins) if plugins.is_empty() => {
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            "No plugins installed. Use /plugins install <npm:package|package-dir|manifest-path-or-url>."
                                .to_string(),
                        ));
                    }
                    Ok(plugins) => {
                        let mut message = String::from(
                            "Installed plugins:
",
                        );
                        for plugin in plugins {
                            let state = if plugin.enabled {
                                "enabled"
                            } else {
                                "disabled"
                            };
                            message.push_str(&format!(
                                "  • {}@{} ({}) [{}]
",
                                plugin.id, plugin.version, plugin.publisher, state
                            ));
                        }
                        self.runtime
                            .chat
                            .messages
                            .push(("system".to_string(), message));
                    }
                    Err(err) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Failed to list installed plugins: {}", err),
                    )),
                }
            }
            Some("catalog") | Some("search") => {
                match futures::executor::block_on(manager.list_catalog_plugins()) {
                    Ok(plugins) if plugins.is_empty() => self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Plugin directory is empty. Add a catalog with /plugins add-source <catalog-url> [name]."
                            .to_string(),
                    )),
                    Ok(plugins) => {
                        let mut message = String::from("Available plugins:\n");
                        for plugin in plugins {
                            message.push_str(&format!(
                                "  • {}@{} ({}) [{}] -> {}\n",
                                plugin.id,
                                plugin.version,
                                plugin.publisher,
                                match plugin.runtime {
                                    crate::plugins::PluginRuntime::Native => "native",
                                    crate::plugins::PluginRuntime::Wasm => "wasm",
                                    crate::plugins::PluginRuntime::Js => "js",
                                },
                                plugin.package
                            ));
                        }
                        self.runtime
                            .chat
                            .messages
                            .push(("system".to_string(), message));
                    }
                    Err(err) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Failed to list plugin directory: {}", err),
                    )),
                }
            }
            Some("install") => {
                let Some(manifest_ref) = parts.get(2) else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Usage: /plugins install <npm:package|package-dir|manifest-path-or-url>"
                            .to_string(),
                    ));
                    return;
                };

                match futures::executor::block_on(manager.install_from_ref(manifest_ref)) {
                    Ok(plugins) => {
                        self.refresh_plugin_catalog(true);
                        let names = plugins
                            .iter()
                            .map(|plugin| format!("{}@{}", plugin.id, plugin.version))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.show_toast(crate::tui::components::Toast::success(format!(
                            "Installed plugin{} {}",
                            if plugins.len() == 1 { "" } else { "s" },
                            names
                        )));
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            format!("Installed {} from {}", names, manifest_ref),
                        ));
                    }
                    Err(err) => self
                        .runtime
                        .chat
                        .messages
                        .push(("system".to_string(), format!("Install failed: {}", err))),
                }
            }
            Some("enable") | Some("disable") => {
                let enable = matches!(parts.get(1), Some(&"enable"));
                let Some(plugin_id) = parts.get(2) else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Usage: /plugins {} <plugin-id>", parts[1]),
                    ));
                    return;
                };
                match futures::executor::block_on(manager.set_plugin_enabled(plugin_id, enable)) {
                    Ok(()) => {
                        self.refresh_plugin_catalog(true);
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            format!(
                                "{} plugin {}",
                                if enable { "Enabled" } else { "Disabled" },
                                plugin_id
                            ),
                        ));
                    }
                    Err(err) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Failed to change plugin state: {}", err),
                    )),
                }
            }
            Some("reload") => {
                let Some(plugin_id) = parts.get(2) else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Usage: /plugins reload <plugin-id>".to_string(),
                    ));
                    return;
                };

                match futures::executor::block_on(manager.reload_plugin(plugin_id)) {
                    Ok(()) => {
                        self.refresh_plugin_catalog(true);
                        if self.ui.plugin_window.active_plugin_id.as_deref() == Some(*plugin_id) {
                            let plugin = crate::tui::plugins::get_plugin_by_id(plugin_id);
                            self.ui.plugin_window.set_plugin(plugin);
                        }
                        self.runtime.chat.messages.push((
                            "system".to_string(),
                            format!("Reloaded plugin shell for {}", plugin_id),
                        ));
                    }
                    Err(err) => self
                        .runtime
                        .chat
                        .messages
                        .push(("system".to_string(), format!("Reload failed: {}", err))),
                }
            }
            Some("add-source") => {
                let Some(source_ref) = parts.get(2) else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Usage: /plugins add-source <catalog-url-or-path> [name]".to_string(),
                    ));
                    return;
                };
                let name = parts.get(3).copied();
                match futures::executor::block_on(manager.add_source(name, source_ref)) {
                    Ok(source) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Added source '{}' -> {}", source.name, source.manifest_url),
                    )),
                    Err(err) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Failed to add source: {}", err),
                    )),
                }
            }
            Some("sources") => match futures::executor::block_on(manager.list_sources()) {
                Ok(sources) if sources.is_empty() => self.runtime.chat.messages.push((
                    "system".to_string(),
                    "No plugin sources configured.".to_string(),
                )),
                Ok(sources) => {
                    let mut message = String::from(
                        "Plugin sources:
",
                    );
                    for source in sources {
                        message.push_str(&format!(
                            "  • {} -> {}
",
                            source.name, source.manifest_url
                        ));
                    }
                    self.runtime
                        .chat
                        .messages
                        .push(("system".to_string(), message));
                }
                Err(err) => self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!("Failed to list sources: {}", err),
                )),
            },
            Some("allow-publisher") => {
                let Some(publisher) = parts.get(2) else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Usage: /plugins allow-publisher <publisher>".to_string(),
                    ));
                    return;
                };

                match futures::executor::block_on(manager.add_allowed_publisher(publisher)) {
                    Ok(()) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Allowlisted plugin publisher '{}'", publisher),
                    )),
                    Err(err) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Failed to allow publisher: {}", err),
                    )),
                }
            }
            Some("add-key") => {
                let (Some(key_id), Some(key_value)) = (parts.get(2), parts.get(3)) else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Usage: /plugins add-key <key-id> <public-key-base64>".to_string(),
                    ));
                    return;
                };

                match futures::executor::block_on(manager.add_trusted_key(key_id, key_value)) {
                    Ok(()) => self.runtime.chat.messages.push((
                        "system".to_string(),
                        format!("Added trusted key '{}'", key_id),
                    )),
                    Err(err) => self
                        .runtime
                        .chat
                        .messages
                        .push(("system".to_string(), format!("Failed to add key: {}", err))),
                }
            }
            Some("refresh") | Some("update") => {
                if self.refresh_plugin_catalog(true) {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Plugin catalog refreshed.".to_string(),
                    ));
                } else {
                    self.runtime.chat.messages.push((
                        "system".to_string(),
                        "Plugin catalog is already up to date.".to_string(),
                    ));
                }
            }
            Some(other) => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "Unknown /plugins subcommand '{}'. Available: list, catalog, install, enable, disable, reload, add-source, sources, allow-publisher, add-key, refresh",
                        other
                    ),
                ));
            }
        }
    }
}
