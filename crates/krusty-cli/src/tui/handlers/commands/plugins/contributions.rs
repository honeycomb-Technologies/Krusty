use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::agent::PackageHookConfig;
use crate::plugins::PluginPermission;
use crate::tui::app::App;
use krusty_core::mcp::{McpConnectionAuthority, McpPackageConfig};

impl App {
    /// Reconcile every runtime contribution from the current enabled plugin
    /// snapshot. A stable fingerprint makes the periodic catalog poll cheap.
    pub(crate) fn reload_plugin_contributions(&mut self) -> bool {
        let Some(plugin_manager) = self.services.plugin_manager.clone() else {
            return false;
        };
        let installed = match futures::executor::block_on(plugin_manager.list_installed_plugins()) {
            Ok(installed) => installed,
            Err(error) => {
                tracing::warn!(error = %error, "Failed to resolve plugin contributions");
                return false;
            }
        };

        let mut hasher = DefaultHasher::new();
        let mut skill_roots = Vec::new();
        let mut executable_roots = Vec::new();
        let mut package_hook_configs = Vec::new();
        let mut mcp_paths = Vec::new();
        for plugin in &installed {
            plugin.id.hash(&mut hasher);
            plugin.version.hash(&mut hasher);
            plugin.enabled.hash(&mut hasher);
            plugin.manifest_path.hash(&mut hasher);
            if !plugin.enabled {
                continue;
            }

            for path in &plugin.skill_paths {
                skill_roots.push((plugin.id.clone(), path.clone()));
            }

            if !plugin.agent_extension_paths.is_empty() || !plugin.hook_paths.is_empty() {
                let permission = futures::executor::block_on(
                    plugin_manager
                        .ensure_installed_plugin_permission(plugin, PluginPermission::Process),
                );
                permission.is_ok().hash(&mut hasher);
                if permission.is_ok() {
                    executable_roots.extend(plugin.agent_extension_paths.iter().cloned());
                    for path in &plugin.hook_paths {
                        path.hash(&mut hasher);
                        package_hook_configs.push(PackageHookConfig::new(
                            &plugin.id,
                            path,
                            &plugin.install_path,
                        ));
                    }
                }
            }

            if let Some(path) = &plugin.mcp_servers_path {
                match futures::executor::block_on(
                    plugin_manager.permission_status_for_installed(plugin),
                ) {
                    Ok(status) => {
                        status.grant_is_current.hash(&mut hasher);
                        status.granted.fs_read.hash(&mut hasher);
                        status.granted.fs_write.hash(&mut hasher);
                        status.granted.network.hash(&mut hasher);
                        status.granted.process.hash(&mut hasher);
                        let authority = McpConnectionAuthority::new(
                            status.granted.process,
                            status.granted.network,
                        );
                        if status.grant_is_current && !authority.is_empty() {
                            mcp_paths.push(McpPackageConfig::new(path.clone(), authority));
                        }
                    }
                    Err(error) => {
                        tracing::warn!(plugin_id = %plugin.id, error = %error, "Failed to resolve plugin MCP permissions");
                        false.hash(&mut hasher);
                    }
                }
            }
        }

        let fingerprint = hasher.finish();
        if self.runtime.plugin_contribution_fingerprint == Some(fingerprint) {
            return false;
        }

        let skills_manager = self.services.skills_manager.clone();
        let extension_manager = self.services.tool_registry.agent_extension_manager();
        let user_hook_manager = self.services.user_hook_manager.clone();
        let mcp_manager = self.services.mcp_manager.clone();
        let tool_registry = self.services.tool_registry.clone();
        let mcp_status_tx = self.services.mcp_status_tx.clone();
        let reload = futures::executor::block_on(async move {
            let hook_report = user_hook_manager
                .write()
                .await
                .replace_package_hooks(package_hook_configs)?;
            skills_manager.write().await.set_package_roots(skill_roots);

            let extension_commands = if let Some(extension_manager) = extension_manager {
                extension_manager
                    .set_package_roots_and_refresh(executable_roots, &tool_registry)
                    .await?;
                extension_manager.commands().await
            } else {
                Vec::new()
            };

            mcp_manager.set_package_configs(mcp_paths).await;
            let load_result = mcp_manager.load_config().await;
            // Always remove wrappers from the previous authorization snapshot.
            // A failed parse clears the manager fail-closed and must also clear
            // the advertised tool surface.
            tool_registry.unregister_by_prefix("mcp__").await;
            load_result?;
            mcp_manager.connect_all().await?;
            krusty_core::mcp::tool::register_mcp_tools(mcp_manager.clone(), &tool_registry).await;
            Ok::<_, anyhow::Error>((extension_commands, hook_report))
        });

        match reload {
            Ok((extension_commands, hook_report)) => {
                self.runtime.plugin_contribution_fingerprint = Some(fingerprint);
                self.ui.autocomplete.set_extension_commands(
                    extension_commands
                        .into_iter()
                        .map(|command| (command.name, command.description)),
                );
                self.services.cached_ai_tools =
                    futures::executor::block_on(self.services.tool_registry.get_ai_tools_all());
                let _ = mcp_status_tx.send(crate::tui::utils::McpStatusUpdate {
                    success: true,
                    message: format!(
                        "Plugin contributions reloaded ({} package hooks)",
                        hook_report.hook_count
                    ),
                });
                true
            }
            Err(error) => {
                tracing::warn!(error = %error, "Failed to reload plugin contributions");
                let _ = mcp_status_tx.send(crate::tui::utils::McpStatusUpdate {
                    success: false,
                    message: format!("Plugin contribution reload failed: {error}"),
                });
                false
            }
        }
    }
}
