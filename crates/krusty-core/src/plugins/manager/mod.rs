use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tracing::warn;

use self::storage::{
    load_lockfile, load_sources, read_installed_from_lock_entry, save_lockfile, save_sources,
};
use self::validation::infer_source_name;
use super::{InstalledPlugin, PluginSource};

mod catalog;
mod install;
mod io;
mod layout;
mod package;
mod storage;
#[cfg(test)]
mod tests;
mod trust;
mod validation;

/// Central manager for installable TUI plugins.
#[derive(Clone)]
pub struct PluginManager {
    root: PathBuf,
    http_client: reqwest::Client,
}

impl PluginManager {
    pub fn new(http_client: reqwest::Client, root: PathBuf) -> Self {
        Self { root, http_client }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn installed_root(&self) -> PathBuf {
        self.root.join("installed")
    }

    pub fn active_root(&self) -> PathBuf {
        self.root.join("active")
    }

    pub fn state_root(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn index_root(&self) -> PathBuf {
        self.root.join("index")
    }

    pub fn trust_root(&self) -> PathBuf {
        self.root.join("trust")
    }

    pub fn lockfile_path(&self) -> PathBuf {
        self.root.join("plugins.lock")
    }

    pub(super) fn trust_file_path(&self) -> PathBuf {
        self.trust_root().join("allowlist.toml")
    }

    pub(super) fn sources_file_path(&self) -> PathBuf {
        self.index_root().join("sources.toml")
    }

    pub async fn list_sources(&self) -> Result<Vec<PluginSource>> {
        Ok(load_sources(self).await?.sources)
    }

    pub async fn add_source(&self, name: Option<&str>, manifest_url: &str) -> Result<PluginSource> {
        let mut sources = load_sources(self).await?;
        let resolved_name = match name {
            Some(explicit) if !explicit.trim().is_empty() => explicit.trim().to_string(),
            _ => infer_source_name(manifest_url),
        };

        let source = PluginSource {
            name: resolved_name,
            manifest_url: manifest_url.to_string(),
        };

        sources
            .sources
            .retain(|entry| entry.name != source.name && entry.manifest_url != source.manifest_url);
        sources.sources.push(source.clone());
        sources.sources.sort_by(|a, b| a.name.cmp(&b.name));

        save_sources(self, &sources).await?;
        Ok(source)
    }

    pub async fn set_plugin_enabled(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        let mut lock = load_lockfile(self).await?;
        let entry = lock
            .plugins
            .iter_mut()
            .find(|entry| entry.id == plugin_id)
            .with_context(|| format!("plugin '{}' is not installed", plugin_id))?;

        entry.enabled = enabled;
        save_lockfile(self, &lock).await
    }

    /// Explicit reload request. Runtime hosts perform the actual reload when the
    /// active plugin instance is recreated by the caller.
    pub async fn reload_plugin(&self, plugin_id: &str) -> Result<()> {
        let installed = self.list_installed_plugins().await?;
        if installed.iter().all(|plugin| plugin.id != plugin_id) {
            bail!("plugin '{}' is not installed", plugin_id);
        }
        Ok(())
    }

    pub async fn list_installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        let lock = load_lockfile(self).await?;
        let mut installed = Vec::new();

        for entry in lock.plugins {
            match read_installed_from_lock_entry(self, &entry).await {
                Ok(plugin) => installed.push(plugin),
                Err(err) => {
                    warn!(
                        "Skipping installed plugin {}@{} due to invalid metadata: {}",
                        entry.id, entry.version, err
                    );
                }
            }
        }

        installed.sort_by_key(|plugin| plugin.name.to_lowercase());
        Ok(installed)
    }
}
