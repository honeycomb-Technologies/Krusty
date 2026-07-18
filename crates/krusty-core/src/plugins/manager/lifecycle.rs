use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use tokio::fs;
use tracing::warn;

use super::layout::ensure_real_directory;
use super::storage::{
    load_lockfile, load_permissions, read_installed_from_lock_entry, save_lockfile,
    save_permissions,
};
use super::transaction::remove_manager_owned_root;
use super::PluginManager;
use crate::plugins::{
    InstalledPlugin, PluginInstallOptions, PluginPermission, PluginPermissionSet,
    PluginPermissionStatus, PluginReconcileReport, PluginUpdateRecord, PluginUpdateReport,
};

impl PluginManager {
    /// Resolve a permission decision against the exact installed descriptor a
    /// host is about to activate. Unlike the ID-only management API, this does
    /// not replace the caller's descriptor with whichever version currently
    /// occupies the same ID.
    pub async fn permission_status_for_installed(
        &self,
        plugin: &InstalledPlugin,
    ) -> Result<PluginPermissionStatus> {
        let permissions = load_permissions(self).await?;
        let grant = permissions.plugins.get(&plugin.id);
        let grant_is_current = grant.and_then(|grant| grant.plugin_version.as_deref())
            == Some(plugin.version.as_str())
            && grant.and_then(|grant| grant.requested.as_ref())
                == Some(&plugin.requested_permissions)
            && grant.and_then(|grant| grant.publisher.as_deref())
                == Some(plugin.publisher.as_str())
            && grant.map(|grant| &grant.source) == Some(&plugin.source);
        let granted = if grant_is_current {
            grant.map(|grant| grant.granted.clone()).unwrap_or_default()
        } else {
            PluginPermissionSet::default()
        };

        Ok(PluginPermissionStatus {
            plugin_id: plugin.id.clone(),
            plugin_version: plugin.version.clone(),
            publisher: plugin.publisher.clone(),
            source: plugin.source.clone(),
            requested: plugin.requested_permissions.clone(),
            granted,
            grant_is_current,
        })
    }

    /// Runtime activation check bound to an exact installed descriptor.
    pub async fn ensure_installed_plugin_permission(
        &self,
        plugin: &InstalledPlugin,
        permission: PluginPermission,
    ) -> Result<()> {
        let status = self.permission_status_for_installed(plugin).await?;
        if !status.requested.allows(permission) {
            bail!(
                "plugin '{}@{}' attempted {:?} without declaring it in requested_permissions",
                plugin.id,
                plugin.version,
                permission
            );
        }
        if !status.allows(permission) {
            bail!(
                "permission {:?} is not granted for exact plugin descriptor '{}@{}'; review and grant it explicitly",
                permission,
                plugin.id,
                plugin.version
            );
        }
        Ok(())
    }

    pub async fn set_plugin_pinned(&self, plugin_id: &str, pinned: bool) -> Result<()> {
        let _guard = self.acquire_mutation().await?;
        let mut lock = load_lockfile(self).await?;
        let entry = lock
            .plugins
            .iter_mut()
            .find(|entry| entry.id == plugin_id)
            .with_context(|| format!("plugin '{}' is not installed", plugin_id))?;
        entry.pinned = pinned;
        save_lockfile(self, &lock).await
    }

    /// Removes plugin state first, then reclaims its immutable snapshot only if
    /// no other plugin references it. Manager-owned containment is revalidated
    /// immediately before recursive deletion.
    pub async fn uninstall_plugin(&self, plugin_id: &str) -> Result<()> {
        let _guard = self.acquire_mutation().await?;
        let mut lock = load_lockfile(self).await?;
        let removed = lock
            .plugins
            .iter()
            .find(|entry| entry.id == plugin_id)
            .cloned()
            .with_context(|| format!("plugin '{}' is not installed", plugin_id))?;

        // Revoke first: if a later write fails, the safe failure mode is an
        // installed plugin with no privileged grants.
        let mut permissions = load_permissions(self).await?;
        permissions.plugins.remove(plugin_id);
        save_permissions(self, &permissions).await?;

        lock.plugins.retain(|entry| entry.id != plugin_id);
        save_lockfile(self, &lock).await?;

        if let Err(error) = remove_active_entry(self, plugin_id).await {
            warn!(
                "plugin '{}' was uninstalled but its active entry could not be safely removed: {}",
                plugin_id, error
            );
        }
        let removal_root = removed.managed_root.clone().or_else(|| {
            removed.package_path.is_none().then(|| {
                self.installed_root()
                    .join(&removed.id)
                    .join(&removed.version)
            })
        });
        if let Some(root) = removal_root {
            let still_referenced = lock
                .plugins
                .iter()
                .any(|entry| entry.managed_root.as_ref() == Some(&root));
            if !still_referenced {
                if let Err(error) = remove_manager_owned_root(self, &root).await {
                    warn!(
                        "plugin '{}' was uninstalled but snapshot {} needs reconciliation: {}",
                        plugin_id,
                        root.display(),
                        error
                    );
                }
            }
        }
        Ok(())
    }

    pub async fn update_plugin(
        &self,
        plugin_id: &str,
        include_pinned: bool,
    ) -> Result<PluginUpdateReport> {
        let _guard = self.acquire_mutation().await?;
        let lock = load_lockfile(self).await?;
        let target = lock
            .plugins
            .iter()
            .find(|entry| entry.id == plugin_id)
            .cloned()
            .with_context(|| format!("plugin '{}' is not installed", plugin_id))?;
        let source = target.source.clone().with_context(|| {
            format!(
                "plugin '{}' has no recorded source; reinstall it before updating",
                plugin_id
            )
        })?;
        let source_entries: Vec<_> = lock
            .plugins
            .iter()
            .filter(|entry| entry.source.as_deref() == Some(source.as_str()))
            .cloned()
            .collect();

        if !include_pinned && source_entries.iter().any(|entry| entry.pinned) {
            return Ok(PluginUpdateReport {
                skipped_pinned: source_entries
                    .into_iter()
                    .filter(|entry| entry.pinned)
                    .map(|entry| entry.id)
                    .collect(),
                ..PluginUpdateReport::default()
            });
        }

        self.update_source_unlocked(&source, &source_entries).await
    }

    pub async fn update_all_plugins(&self, include_pinned: bool) -> Result<PluginUpdateReport> {
        // Selection, pin evaluation, and every resulting commit share one
        // cross-process mutation lease. Otherwise an uninstall/reinstall can
        // change a source between the snapshot below and its update commit.
        let _guard = self.acquire_mutation().await?;
        let lock = load_lockfile(self).await?;
        let mut by_source: BTreeMap<String, Vec<_>> = BTreeMap::new();
        let mut report = PluginUpdateReport::default();
        for entry in lock.plugins {
            if let Some(source) = entry.source.clone() {
                by_source.entry(source).or_default().push(entry);
            } else {
                report.unchanged.push(entry.id);
            }
        }

        for (source, entries) in by_source {
            if !include_pinned && entries.iter().any(|entry| entry.pinned) {
                report.skipped_pinned.extend(
                    entries
                        .iter()
                        .filter(|entry| entry.pinned)
                        .map(|entry| entry.id.clone()),
                );
                continue;
            }
            let source_report = self.update_source_unlocked(&source, &entries).await?;
            report.updated.extend(source_report.updated);
            report.unchanged.extend(source_report.unchanged);
            report.removed.extend(source_report.removed);
            report.skipped_pinned.extend(source_report.skipped_pinned);
        }

        report.updated.sort_by(|a, b| a.id.cmp(&b.id));
        report.unchanged.sort();
        report.unchanged.dedup();
        report.removed.sort();
        report.removed.dedup();
        report.skipped_pinned.sort();
        report.skipped_pinned.dedup();
        Ok(report)
    }

    async fn update_source_unlocked(
        &self,
        source: &str,
        previous_entries: &[crate::plugins::PluginLockEntry],
    ) -> Result<PluginUpdateReport> {
        let allow_package_scripts = previous_entries
            .iter()
            .any(|entry| entry.package_scripts_allowed);
        let installed = self
            .install_from_ref_with_options_unlocked(
                source,
                PluginInstallOptions {
                    allow_package_scripts,
                    // Existing IDs preserve their own pin state in the installer.
                    pinned: None,
                },
            )
            .await
            .with_context(|| format!("failed to update plugin source '{}'", source))?;

        let previous: BTreeMap<_, _> = previous_entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.version.as_str()))
            .collect();
        let mut report = PluginUpdateReport::default();
        let mut current_ids = BTreeSet::new();
        for plugin in installed {
            current_ids.insert(plugin.id.clone());
            match previous.get(plugin.id.as_str()) {
                Some(previous_version) if *previous_version == plugin.version.as_str() => {
                    report.unchanged.push(plugin.id)
                }
                Some(previous_version) => report.updated.push(PluginUpdateRecord {
                    id: plugin.id,
                    previous_version: (*previous_version).to_string(),
                    current_version: plugin.version,
                }),
                None => report.updated.push(PluginUpdateRecord {
                    id: plugin.id,
                    previous_version: "not-installed".to_string(),
                    current_version: plugin.version,
                }),
            }
        }
        report.removed.extend(
            previous_entries
                .iter()
                .filter(|entry| !current_ids.contains(&entry.id))
                .map(|entry| entry.id.clone()),
        );
        Ok(report)
    }

    /// Validates every lock entry and removes transaction/orphan snapshots only.
    /// Invalid installed entries remain recorded for diagnosis and explicit repair.
    pub async fn reconcile_plugins(&self, update_unpinned: bool) -> Result<PluginReconcileReport> {
        let guard = self.acquire_mutation().await?;
        let lock = load_lockfile(self).await?;
        let mut report = PluginReconcileReport::default();
        for entry in &lock.plugins {
            match read_installed_from_lock_entry(self, entry).await {
                Ok(_) => report.valid_plugins.push(entry.id.clone()),
                Err(error) => report
                    .invalid_plugins
                    .push((entry.id.clone(), error.to_string())),
            }
        }

        let installed_ids: BTreeSet<_> =
            lock.plugins.iter().map(|entry| entry.id.as_str()).collect();
        let mut permissions = load_permissions(self).await?;
        let original_grants = permissions.plugins.len();
        permissions
            .plugins
            .retain(|plugin_id, _| installed_ids.contains(plugin_id.as_str()));
        if permissions.plugins.len() != original_grants {
            save_permissions(self, &permissions).await?;
        }

        let referenced_roots: BTreeSet<PathBuf> = lock
            .plugins
            .iter()
            .filter_map(|entry| entry.managed_root.clone())
            .collect();
        remove_orphan_children(
            self,
            &self.staging_root(),
            &BTreeSet::new(),
            &mut report.removed_orphan_roots,
        )
        .await?;
        remove_orphan_children(
            self,
            &self.managed_root(),
            &referenced_roots,
            &mut report.removed_orphan_roots,
        )
        .await?;

        report.valid_plugins.sort();
        report.invalid_plugins.sort_by(|a, b| a.0.cmp(&b.0));
        drop(guard);
        if update_unpinned {
            report.updates = self.update_all_plugins(false).await?;
        }
        Ok(report)
    }
}

pub(super) async fn remove_active_entry(manager: &PluginManager, plugin_id: &str) -> Result<()> {
    super::validation::validate_plugin_id(plugin_id)?;
    let active_root = manager.active_root();
    ensure_real_directory(&active_root, "active plugin root").await?;
    let path = active_root.join(plugin_id);
    let metadata = match fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect active plugin entry {}", path.display())
            })
        }
    };

    // Revalidate the parent immediately before removal. Active entries are
    // metadata/cache state, so directories are removed only when empty. Never
    // recurse through this path: a swapped active root must not turn cleanup
    // into remove_dir_all against an external victim tree.
    ensure_real_directory(&active_root, "active plugin root").await?;
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir(&path).await
    } else {
        fs::remove_file(&path).await
    };
    result.with_context(|| format!("failed to remove active plugin entry {}", path.display()))
}

async fn remove_orphan_children(
    manager: &PluginManager,
    directory: &std::path::Path,
    referenced: &BTreeSet<PathBuf>,
    removed: &mut Vec<PathBuf>,
) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if referenced.contains(&path) {
            continue;
        }
        remove_manager_owned_root(manager, &path).await?;
        removed.push(path);
    }
    Ok(())
}
