use anyhow::{bail, Result};

use crate::plugins::{
    PluginInstallOptions, PluginPermissionSet, PluginPermissionStatus, PluginSourceTrust,
    PluginUpdateReport,
};
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
            Some("list") => match futures::executor::block_on(manager.list_installed_plugins()) {
                Ok(plugins) if plugins.is_empty() => self.push_plugin_message(
                    "No plugins installed. Use /plugins install <npm:package|package-dir|manifest-path-or-url>.",
                ),
                Ok(plugins) => {
                    let mut message = String::from("Installed plugins:\n");
                    for plugin in plugins {
                        let state = if plugin.enabled { "enabled" } else { "disabled" };
                        let pin = if plugin.pinned { "pinned" } else { "updatable" };
                        let trust = source_trust_label(plugin.source_trust);
                        message.push_str(&format!(
                            "  • {}@{} ({}) [{}, {}, {}]{}\n",
                            plugin.id,
                            plugin.version,
                            plugin.publisher,
                            state,
                            pin,
                            trust,
                            if plugin.requested_permissions.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    " permissions={}",
                                    permission_set_label(&plugin.requested_permissions)
                                )
                            }
                        ));
                    }
                    self.push_plugin_message(message);
                }
                Err(error) => self.push_plugin_message(format!(
                    "Failed to list installed plugins: {}",
                    error
                )),
            },
            Some("catalog") | Some("search") => {
                match futures::executor::block_on(manager.list_catalog_plugins()) {
                    Ok(plugins) if plugins.is_empty() => self.push_plugin_message(
                        "Plugin directory is empty. Add a catalog with /plugins add-source <catalog-url> [name].",
                    ),
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
                        self.push_plugin_message(message);
                    }
                    Err(error) => self.push_plugin_message(format!(
                        "Failed to list plugin directory: {}",
                        error
                    )),
                }
            }
            Some("install") => {
                let Some(plugin_ref) = parts.get(2) else {
                    self.push_plugin_message(
                        "Usage: /plugins install <ref> [--allow-scripts] [--pin|--unpin]",
                    );
                    return;
                };
                let options = match parse_install_options(&parts[3..]) {
                    Ok(options) => options,
                    Err(error) => {
                        self.push_plugin_message(format!("Invalid install options: {}", error));
                        return;
                    }
                };

                match futures::executor::block_on(
                    manager.install_from_ref_with_options(plugin_ref, options),
                ) {
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
                        let script_notice = if options.allow_package_scripts {
                            " Package script execution was explicitly allowed."
                        } else {
                            " Package scripts were blocked."
                        };
                        self.push_plugin_message(format!(
                            "Installed {} from {}.{}",
                            names, plugin_ref, script_notice
                        ));
                    }
                    Err(error) => self.push_plugin_message(format!("Install failed: {}", error)),
                }
            }
            Some("uninstall") | Some("remove") => {
                let Some(plugin_id) = parts.get(2) else {
                    self.push_plugin_message("Usage: /plugins uninstall <plugin-id>");
                    return;
                };
                match futures::executor::block_on(manager.uninstall_plugin(plugin_id)) {
                    Ok(()) => {
                        if self.ui.plugin_window.active_plugin_id.as_deref() == Some(*plugin_id) {
                            self.ui.plugin_window.set_plugin(None);
                        }
                        self.refresh_plugin_catalog(false);
                        self.push_plugin_message(format!("Uninstalled plugin {}", plugin_id));
                    }
                    Err(error) => {
                        self.push_plugin_message(format!("Uninstall failed: {}", error))
                    }
                }
            }
            Some("enable") | Some("disable") => {
                let enable = matches!(parts.get(1), Some(&"enable"));
                let Some(plugin_id) = parts.get(2) else {
                    self.push_plugin_message(format!("Usage: /plugins {} <plugin-id>", parts[1]));
                    return;
                };
                match futures::executor::block_on(manager.set_plugin_enabled(plugin_id, enable)) {
                    Ok(()) => {
                        self.refresh_plugin_catalog(true);
                        self.push_plugin_message(format!(
                            "{} plugin {}",
                            if enable { "Enabled" } else { "Disabled" },
                            plugin_id
                        ));
                    }
                    Err(error) => self
                        .push_plugin_message(format!("Failed to change plugin state: {}", error)),
                }
            }
            Some("pin") | Some("unpin") => {
                let pinned = matches!(parts.get(1), Some(&"pin"));
                let Some(plugin_id) = parts.get(2) else {
                    self.push_plugin_message(format!("Usage: /plugins {} <plugin-id>", parts[1]));
                    return;
                };
                match futures::executor::block_on(manager.set_plugin_pinned(plugin_id, pinned)) {
                    Ok(()) => {
                        self.refresh_plugin_catalog(false);
                        self.push_plugin_message(format!(
                            "Plugin {} is now {}",
                            plugin_id,
                            if pinned { "pinned" } else { "eligible for updates" }
                        ));
                    }
                    Err(error) => {
                        self.push_plugin_message(format!("Failed to change pin state: {}", error))
                    }
                }
            }
            Some("update") => {
                let include_pinned = parts.contains(&"--include-pinned");
                let target = parts
                    .get(2)
                    .copied()
                    .filter(|value| !value.starts_with("--") && *value != "all");
                let result = match target {
                    Some(plugin_id) => futures::executor::block_on(
                        manager.update_plugin(plugin_id, include_pinned),
                    ),
                    None => futures::executor::block_on(
                        manager.update_all_plugins(include_pinned),
                    ),
                };
                match result {
                    Ok(report) => {
                        self.refresh_plugin_catalog(true);
                        self.push_plugin_message(format_update_report(&report));
                    }
                    Err(error) => self.push_plugin_message(format!("Plugin update failed: {}", error)),
                }
            }
            Some("reconcile") => {
                let update = parts.contains(&"--update");
                match futures::executor::block_on(manager.reconcile_plugins(update)) {
                    Ok(report) => {
                        self.refresh_plugin_catalog(true);
                        let mut message = format!(
                            "Plugin reconciliation complete: {} valid, {} invalid, {} orphan snapshot(s) removed.",
                            report.valid_plugins.len(),
                            report.invalid_plugins.len(),
                            report.removed_orphan_roots.len()
                        );
                        for (id, error) in report.invalid_plugins {
                            message.push_str(&format!("\n  • {}: {}", id, error));
                        }
                        if update {
                            message.push('\n');
                            message.push_str(&format_update_report(&report.updates));
                        }
                        self.push_plugin_message(message);
                    }
                    Err(error) => self
                        .push_plugin_message(format!("Plugin reconciliation failed: {}", error)),
                }
            }
            Some("permissions") => {
                let Some(plugin_id) = parts.get(2) else {
                    self.push_plugin_message("Usage: /plugins permissions <plugin-id>");
                    return;
                };
                match futures::executor::block_on(manager.permission_status(plugin_id)) {
                    Ok(status) => self.push_plugin_message(format_permission_status(&status)),
                    Err(error) => self
                        .push_plugin_message(format!("Failed to read plugin permissions: {}", error)),
                }
            }
            Some("grant") => {
                let (Some(plugin_id), Some(grants)) = (parts.get(2), parts.get(3)) else {
                    self.push_plugin_message(
                        "Usage: /plugins grant <plugin-id> <all|none|fs-read,fs-write,network,process>",
                    );
                    return;
                };
                let result = if *grants == "all" {
                    futures::executor::block_on(manager.grant_all_plugin_permissions(plugin_id))
                } else {
                    match parse_permission_set(grants) {
                        Ok(granted) => futures::executor::block_on(
                            manager.grant_plugin_permissions(plugin_id, granted),
                        ),
                        Err(error) => {
                            self.push_plugin_message(format!("Invalid permission set: {}", error));
                            return;
                        }
                    }
                };
                match result {
                    Ok(status) => {
                        self.refresh_plugin_catalog(true);
                        self.push_plugin_message(format_permission_status(&status));
                    }
                    Err(error) => self
                        .push_plugin_message(format!("Failed to grant plugin permissions: {}", error)),
                }
            }
            Some("revoke-permissions") => {
                let Some(plugin_id) = parts.get(2) else {
                    self.push_plugin_message("Usage: /plugins revoke-permissions <plugin-id>");
                    return;
                };
                match futures::executor::block_on(manager.revoke_plugin_permissions(plugin_id)) {
                    Ok(()) => {
                        self.refresh_plugin_catalog(true);
                        self.push_plugin_message(format!(
                            "Revoked all permissions for {}",
                            plugin_id
                        ));
                    }
                    Err(error) => self.push_plugin_message(format!(
                        "Failed to revoke plugin permissions: {}",
                        error
                    )),
                }
            }
            Some("reload") => {
                let Some(plugin_id) = parts.get(2) else {
                    self.push_plugin_message("Usage: /plugins reload <plugin-id>");
                    return;
                };
                match futures::executor::block_on(manager.reload_plugin(plugin_id)) {
                    Ok(()) => {
                        self.refresh_plugin_catalog(true);
                        if self.ui.plugin_window.active_plugin_id.as_deref() == Some(*plugin_id) {
                            let plugin = crate::tui::plugins::get_plugin_by_id(plugin_id);
                            self.ui.plugin_window.set_plugin(plugin);
                        }
                        self.push_plugin_message(format!("Reloaded plugin shell for {}", plugin_id));
                    }
                    Err(error) => self.push_plugin_message(format!("Reload failed: {}", error)),
                }
            }
            Some("add-source") => {
                let Some(source_ref) = parts.get(2) else {
                    self.push_plugin_message(
                        "Usage: /plugins add-source <https-catalog-url|local-path> [name]",
                    );
                    return;
                };
                let name = parts.get(3).copied();
                match futures::executor::block_on(manager.add_source(name, source_ref)) {
                    Ok(source) => self.push_plugin_message(format!(
                        "Added source '{}' -> {}",
                        source.name, source.manifest_url
                    )),
                    Err(error) => {
                        self.push_plugin_message(format!("Failed to add source: {}", error))
                    }
                }
            }
            Some("remove-source") => {
                let Some(name) = parts.get(2) else {
                    self.push_plugin_message("Usage: /plugins remove-source <name>");
                    return;
                };
                match futures::executor::block_on(manager.remove_source(name)) {
                    Ok(()) => self.push_plugin_message(format!("Removed plugin source '{}'", name)),
                    Err(error) => {
                        self.push_plugin_message(format!("Failed to remove source: {}", error))
                    }
                }
            }
            Some("sources") => match futures::executor::block_on(manager.list_sources()) {
                Ok(sources) if sources.is_empty() => {
                    self.push_plugin_message("No plugin sources configured.")
                }
                Ok(sources) => {
                    let mut message = String::from("Plugin sources:\n");
                    for source in sources {
                        message.push_str(&format!(
                            "  • {} -> {}\n",
                            source.name, source.manifest_url
                        ));
                    }
                    self.push_plugin_message(message);
                }
                Err(error) => {
                    self.push_plugin_message(format!("Failed to list sources: {}", error))
                }
            },
            Some("allow-publisher") => {
                let Some(publisher) = parts.get(2) else {
                    self.push_plugin_message("Usage: /plugins allow-publisher <publisher>");
                    return;
                };
                match futures::executor::block_on(manager.add_allowed_publisher(publisher)) {
                    Ok(()) => self.push_plugin_message(format!(
                        "Allowlisted plugin publisher '{}'",
                        publisher
                    )),
                    Err(error) => self
                        .push_plugin_message(format!("Failed to allow publisher: {}", error)),
                }
            }
            Some("add-key") => {
                let (Some(key_id), Some(key_value), Some(publisher)) =
                    (parts.get(2), parts.get(3), parts.get(4))
                else {
                    self.push_plugin_message(
                        "Usage: /plugins add-key <key-id> <public-key-base64> <publisher>",
                    );
                    return;
                };
                match futures::executor::block_on(manager.add_trusted_key_for_publisher(
                    publisher,
                    key_id,
                    key_value,
                )) {
                    Ok(()) => self.push_plugin_message(format!(
                        "Added trusted key '{}' for publisher '{}'",
                        key_id, publisher
                    )),
                    Err(error) => self.push_plugin_message(format!("Failed to add key: {}", error)),
                }
            }
            Some("refresh") => {
                let changed = self.refresh_plugin_catalog(true);
                self.push_plugin_message(if changed {
                    "Plugin catalog refreshed."
                } else {
                    "Plugin catalog is already up to date."
                });
            }
            Some("help") => self.push_plugin_message(plugin_help()),
            Some(other) => self.push_plugin_message(format!(
                "Unknown /plugins subcommand '{}'. {}",
                other,
                plugin_help()
            )),
        }
    }

    fn push_plugin_message(&mut self, message: impl Into<String>) {
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), message.into()));
    }
}

fn parse_install_options(parts: &[&str]) -> Result<PluginInstallOptions> {
    let mut options = PluginInstallOptions::default();
    for option in parts {
        match *option {
            "--allow-scripts" => options.allow_package_scripts = true,
            "--pin" => options.pinned = Some(true),
            "--unpin" => options.pinned = Some(false),
            other => bail!("unknown option '{}'", other),
        }
    }
    Ok(options)
}

fn parse_permission_set(value: &str) -> Result<PluginPermissionSet> {
    let mut permissions = PluginPermissionSet::default();
    if value == "none" || value.is_empty() {
        return Ok(permissions);
    }
    for permission in value.split(',') {
        match permission {
            "fs-read" => permissions.fs_read = true,
            "fs-write" => permissions.fs_write = true,
            "network" => permissions.network = true,
            "process" => permissions.process = true,
            other => bail!("unknown permission '{}'", other),
        }
    }
    Ok(permissions)
}

fn permission_set_label(permissions: &PluginPermissionSet) -> String {
    let mut values = Vec::new();
    if permissions.fs_read {
        values.push("fs-read");
    }
    if permissions.fs_write {
        values.push("fs-write");
    }
    if permissions.network {
        values.push("network");
    }
    if permissions.process {
        values.push("process");
    }
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(",")
    }
}

fn format_permission_status(status: &PluginPermissionStatus) -> String {
    let mut message = format!(
        "Plugin {}@{} permissions: requested={}; granted={}; decision={}",
        status.plugin_id,
        status.plugin_version,
        permission_set_label(&status.requested),
        permission_set_label(&status.granted),
        if status.grant_is_current {
            "current"
        } else {
            "missing or stale (privileged access denied)"
        }
    );
    if status.requested.process {
        message.push_str(
            "; warning: process authorizes trusted native/JS/shell code with full user OS authority",
        );
    }
    message
}

fn source_trust_label(trust: PluginSourceTrust) -> &'static str {
    match trust {
        PluginSourceTrust::SignedPublisher => "signed",
        PluginSourceTrust::NpmUnsigned => "npm/unsigned",
        PluginSourceTrust::LocalUnsigned => "local/unsigned",
        PluginSourceTrust::LegacyUnknown => "legacy/unverified",
    }
}

fn format_update_report(report: &PluginUpdateReport) -> String {
    let mut message = format!(
        "Plugin update complete: {} updated, {} unchanged, {} removed, {} pinned/skipped.",
        report.updated.len(),
        report.unchanged.len(),
        report.removed.len(),
        report.skipped_pinned.len()
    );
    for update in &report.updated {
        message.push_str(&format!(
            "\n  • {}: {} -> {}",
            update.id, update.previous_version, update.current_version
        ));
    }
    if !report.removed.is_empty() {
        message.push_str(&format!(
            "\n  Removed from updated package: {}",
            report.removed.join(", ")
        ));
    }
    if !report.skipped_pinned.is_empty() {
        message.push_str(&format!(
            "\n  Pinned: {} (use --include-pinned to override)",
            report.skipped_pinned.join(", ")
        ));
    }
    message
}

fn plugin_help() -> &'static str {
    "Available: list, catalog, install, uninstall, enable, disable, pin, unpin, update, reconcile, permissions, grant, revoke-permissions, reload, add-source, remove-source, sources, allow-publisher, add-key (<key> <base64> <publisher>), refresh"
}
