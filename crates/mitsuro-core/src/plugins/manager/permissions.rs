use anyhow::{bail, Context, Result};

use super::storage::{
    load_lockfile, load_permissions, read_installed_from_lock_entry, save_permissions,
};
use super::PluginManager;
use crate::plugins::{
    PluginPermission, PluginPermissionGrant, PluginPermissionSet, PluginPermissionStatus,
};

impl PluginManager {
    pub async fn permission_status(&self, plugin_id: &str) -> Result<PluginPermissionStatus> {
        let lock = load_lockfile(self).await?;
        let entry = lock
            .plugins
            .iter()
            .find(|entry| entry.id == plugin_id)
            .with_context(|| format!("plugin '{}' is not installed", plugin_id))?;
        let installed = read_installed_from_lock_entry(self, entry).await?;
        let permissions = load_permissions(self).await?;
        let grant = permissions.plugins.get(plugin_id);
        let grant_is_current = grant.and_then(|grant| grant.plugin_version.as_deref())
            == Some(installed.version.as_str())
            && grant.and_then(|grant| grant.requested.as_ref())
                == Some(&installed.requested_permissions)
            && grant.and_then(|grant| grant.publisher.as_deref())
                == Some(installed.publisher.as_str())
            && grant.map(|grant| &grant.source) == Some(&installed.source);
        let granted = if grant_is_current {
            grant.map(|grant| grant.granted.clone()).unwrap_or_default()
        } else {
            PluginPermissionSet::default()
        };

        Ok(PluginPermissionStatus {
            plugin_id: installed.id,
            plugin_version: installed.version,
            publisher: installed.publisher,
            source: installed.source,
            requested: installed.requested_permissions,
            granted,
            grant_is_current,
        })
    }

    pub async fn grant_plugin_permissions(
        &self,
        plugin_id: &str,
        granted: PluginPermissionSet,
    ) -> Result<PluginPermissionStatus> {
        let _guard = self.acquire_mutation().await?;
        let current = self.permission_status(plugin_id).await?;
        if !granted.is_subset_of(&current.requested) {
            bail!(
                "cannot grant permissions that plugin '{}' did not request",
                plugin_id
            );
        }

        let mut permissions = load_permissions(self).await?;
        permissions.plugins.insert(
            plugin_id.to_string(),
            PluginPermissionGrant {
                plugin_version: Some(current.plugin_version.clone()),
                publisher: Some(current.publisher.clone()),
                source: current.source.clone(),
                requested: Some(current.requested.clone()),
                granted: granted.clone(),
            },
        );
        save_permissions(self, &permissions).await?;

        Ok(PluginPermissionStatus {
            granted,
            grant_is_current: true,
            ..current
        })
    }

    pub async fn grant_all_plugin_permissions(
        &self,
        plugin_id: &str,
    ) -> Result<PluginPermissionStatus> {
        let status = self.permission_status(plugin_id).await?;
        self.grant_plugin_permissions(plugin_id, status.requested)
            .await
    }

    pub async fn revoke_plugin_permissions(&self, plugin_id: &str) -> Result<()> {
        let _guard = self.acquire_mutation().await?;
        let lock = load_lockfile(self).await?;
        if lock.plugins.iter().all(|entry| entry.id != plugin_id) {
            bail!("plugin '{}' is not installed", plugin_id);
        }

        let mut permissions = load_permissions(self).await?;
        permissions.plugins.remove(plugin_id);
        save_permissions(self, &permissions).await
    }

    /// Runtime enforcement hook. Call this immediately before any privileged
    /// plugin operation; absent, stale, or partial grants are denied.
    pub async fn ensure_plugin_permission(
        &self,
        plugin_id: &str,
        permission: PluginPermission,
    ) -> Result<()> {
        let status = self.permission_status(plugin_id).await?;
        if !status.requested.allows(permission) {
            bail!(
                "plugin '{}' attempted {:?} without declaring it in requested_permissions",
                plugin_id,
                permission
            );
        }
        if !status.allows(permission) {
            bail!(
                "permission {:?} is not granted for plugin '{}'; review and grant it explicitly",
                permission,
                plugin_id
            );
        }
        Ok(())
    }
}
