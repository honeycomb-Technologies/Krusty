use std::ffi::OsStr;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use super::validation::validate_relative_path;
use super::PluginManager;
use crate::plugins::{
    InstalledPlugin, PluginLockEntry, PluginLockfile, PluginManifestV1, PluginSourcesFile,
    PluginTrustPolicy,
};

pub(super) async fn read_installed_from_lock_entry(
    manager: &PluginManager,
    entry: &PluginLockEntry,
) -> Result<InstalledPlugin> {
    let install_path = manager
        .installed_root()
        .join(&entry.id)
        .join(&entry.version);
    let manifest_path = install_path.join("plugin.toml");

    let manifest: PluginManifestV1 = read_toml_or_json(&manifest_path)
        .await
        .with_context(|| format!("failed to load manifest for {}@{}", entry.id, entry.version))?;

    let entry_component_rel = validate_relative_path(&manifest.entry_component)?;
    let entry_component_path = install_path.join(&entry_component_rel);
    let render_capabilities = manifest.normalized_render_capabilities();

    Ok(InstalledPlugin {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        publisher: manifest.publisher,
        description: manifest.description,
        install_path,
        manifest_path,
        entry_component_path,
        enabled: entry.enabled,
        pinned: entry.pinned,
        render_capabilities,
    })
}

pub(super) async fn upsert_lock_entry(
    manager: &PluginManager,
    plugin_id: &str,
    version: &str,
    enabled: bool,
    pinned: bool,
) -> Result<()> {
    let mut lock = load_lockfile(manager).await?;
    lock.plugins.retain(|entry| entry.id != plugin_id);
    lock.plugins.push(PluginLockEntry {
        id: plugin_id.to_string(),
        version: version.to_string(),
        enabled,
        pinned,
    });
    lock.plugins.sort_by(|a, b| a.id.cmp(&b.id));
    save_lockfile(manager, &lock).await
}

pub(super) async fn load_lockfile(manager: &PluginManager) -> Result<PluginLockfile> {
    let path = manager.lockfile_path();
    if !path.exists() {
        let lock = PluginLockfile::default();
        write_toml(&path, &lock).await?;
        return Ok(lock);
    }

    read_toml_or_json(&path).await
}

pub(super) async fn save_lockfile(manager: &PluginManager, lock: &PluginLockfile) -> Result<()> {
    write_toml(&manager.lockfile_path(), lock).await
}

pub(super) async fn load_trust_policy(manager: &PluginManager) -> Result<PluginTrustPolicy> {
    let path = manager.trust_file_path();
    if !path.exists() {
        let trust = PluginTrustPolicy::default();
        write_toml(&path, &trust).await?;
        return Ok(trust);
    }

    read_toml_or_json(&path).await
}

pub(super) async fn save_trust_policy(
    manager: &PluginManager,
    trust: &PluginTrustPolicy,
) -> Result<()> {
    write_toml(&manager.trust_file_path(), trust).await
}

pub(super) async fn load_sources(manager: &PluginManager) -> Result<PluginSourcesFile> {
    let path = manager.sources_file_path();
    if !path.exists() {
        let sources = PluginSourcesFile::default();
        write_toml(&path, &sources).await?;
        return Ok(sources);
    }

    read_toml_or_json(&path).await
}

pub(super) async fn save_sources(
    manager: &PluginManager,
    sources: &PluginSourcesFile,
) -> Result<()> {
    write_toml(&manager.sources_file_path(), sources).await
}

async fn read_toml_or_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;

    super::validation::parse_toml_or_json(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))
}

pub(super) async fn write_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = toml::to_string_pretty(value).context("failed to serialize toml")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("invalid target path for write: {}", path.display()))?;
    let temp_path = path.with_file_name(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4()));

    let mut file = fs::File::create(&temp_path)
        .await
        .with_context(|| format!("failed to create temp file {}", temp_path.display()))?;
    file.write_all(content.as_bytes())
        .await
        .with_context(|| format!("failed to write temp file {}", temp_path.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("failed to sync temp file {}", temp_path.display()))?;
    drop(file);

    if let Err(err) = fs::rename(&temp_path, path).await {
        let _ = fs::remove_file(&temp_path).await;
        return Err(err).with_context(|| {
            format!(
                "failed to atomically replace {} with {}",
                path.display(),
                temp_path.display()
            )
        });
    }

    Ok(())
}
