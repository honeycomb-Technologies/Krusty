use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use super::validation::validate_relative_path_for;
use super::PluginManager;
use crate::plugins::{
    InstalledPlugin, PluginLockEntry, PluginLockfile, PluginManifestV1, PluginPermissionsFile,
    PluginSourcesFile, PluginTrustPolicy,
};

const SUPPORTED_PLUGIN_STATE_VERSION: u32 = 1;

#[derive(Deserialize)]
struct PluginStateSchemaVersion {
    #[serde(default = "supported_plugin_state_version")]
    version: u32,
}

#[derive(Serialize)]
struct VersionedTrustPolicy<'a> {
    version: u32,
    #[serde(flatten)]
    policy: &'a PluginTrustPolicy,
}

const fn supported_plugin_state_version() -> u32 {
    SUPPORTED_PLUGIN_STATE_VERSION
}

pub(super) async fn read_installed_from_lock_entry(
    manager: &PluginManager,
    entry: &PluginLockEntry,
) -> Result<InstalledPlugin> {
    super::validation::validate_plugin_id(&entry.id)?;
    super::validation::validate_plugin_version(&entry.version)?;

    let configured_install_path = entry.package_path.clone().unwrap_or_else(|| {
        manager
            .installed_root()
            .join(&entry.id)
            .join(&entry.version)
    });
    let install_path =
        canonicalize_manager_owned_path(manager, &configured_install_path, "plugin install path")
            .await?;

    let manifest_rel = entry
        .manifest_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("plugin.toml"));
    validate_relative_path_buf(&manifest_rel, "manifest_path")?;
    let manifest_path = canonicalize_descendant(
        &install_path,
        &install_path.join(&manifest_rel),
        "plugin manifest",
    )
    .await?;

    let manifest: PluginManifestV1 = read_toml_or_json(&manifest_path)
        .await
        .with_context(|| format!("failed to load manifest for {}@{}", entry.id, entry.version))?;
    manager.validate_manifest(&manifest, false)?;

    if manifest.id != entry.id {
        bail!(
            "lockfile identity mismatch: entry '{}' points to manifest '{}'",
            entry.id,
            manifest.id
        );
    }
    if manifest.version != entry.version {
        bail!(
            "lockfile version mismatch for '{}': lock has {}, manifest has {}",
            entry.id,
            entry.version,
            manifest.version
        );
    }

    let entry_component_path = match manifest.entry_component.as_deref() {
        Some(path) => Some(resolve_component_path(&install_path, path, "entry_component").await?),
        None => None,
    };
    let skill_paths = resolve_component_paths(&install_path, &manifest.skills, "skills").await?;
    let agent_extension_paths = resolve_component_paths(
        &install_path,
        &manifest.agent_extensions,
        "agent_extensions",
    )
    .await?;
    let mcp_servers_path = match manifest.mcp_servers.as_deref() {
        Some(path) => Some(resolve_component_path(&install_path, path, "mcp_servers").await?),
        None => None,
    };
    let hook_paths = resolve_component_paths(&install_path, &manifest.hooks, "hooks").await?;
    let assets_path = match manifest.assets.as_deref() {
        Some(path) => Some(resolve_component_path(&install_path, path, "assets").await?),
        None => None,
    };
    let render_capabilities = if entry_component_path.is_some() {
        manifest.normalized_render_capabilities()
    } else {
        Vec::new()
    };

    Ok(InstalledPlugin {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        publisher: manifest.publisher,
        description: manifest.description,
        runtime: manifest.runtime,
        install_path,
        manifest_path,
        entry_component_path,
        skill_paths,
        agent_extension_paths,
        mcp_servers_path,
        hook_paths,
        assets_path,
        enabled: entry.enabled,
        pinned: entry.pinned,
        source: entry.source.clone(),
        source_trust: entry.source_trust,
        package_scripts_allowed: entry.package_scripts_allowed,
        requested_permissions: manifest.requested_permissions,
        render_capabilities,
    })
}

async fn resolve_component_paths(
    install_path: &Path,
    paths: &[String],
    label: &str,
) -> Result<Vec<PathBuf>> {
    let mut resolved = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        resolved.push(
            resolve_component_path(install_path, path, &format!("{}[{}]", label, index)).await?,
        );
    }
    Ok(resolved)
}

pub(super) async fn resolve_component_path(
    install_path: &Path,
    path: &str,
    label: &str,
) -> Result<PathBuf> {
    let relative = validate_relative_path_for(path, label)?;
    canonicalize_descendant(install_path, &install_path.join(relative), label).await
}

pub(super) async fn canonicalize_manager_owned_path(
    manager: &PluginManager,
    path: &Path,
    label: &str,
) -> Result<PathBuf> {
    let installed_root = fs::canonicalize(manager.installed_root())
        .await
        .context("failed to canonicalize plugin installed root")?;
    canonicalize_descendant(&installed_root, path, label).await
}

pub(super) async fn canonicalize_descendant(
    root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf> {
    let canonical_root = fs::canonicalize(root)
        .await
        .with_context(|| format!("failed to canonicalize {} root {}", label, root.display()))?;
    let canonical_path = fs::canonicalize(path)
        .await
        .with_context(|| format!("{} does not exist: {}", label, path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("{} escapes plugin install root: {}", label, path.display());
    }
    Ok(canonical_path)
}

pub(super) fn validate_relative_path_buf(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("{} cannot be empty", label);
    }
    if path.is_absolute() {
        bail!("{} must be a relative path", label);
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("{} cannot contain path traversal", label);
    }
    Ok(())
}

pub(super) async fn load_lockfile(manager: &PluginManager) -> Result<PluginLockfile> {
    load_versioned_or_default(&manager.lockfile_path(), "plugin lockfile").await
}

pub(super) async fn save_lockfile(manager: &PluginManager, lock: &PluginLockfile) -> Result<()> {
    ensure_supported_plugin_state_version(lock.version, "plugin lockfile")?;
    write_toml(&manager.lockfile_path(), lock).await
}

pub(super) async fn load_trust_policy(manager: &PluginManager) -> Result<PluginTrustPolicy> {
    load_versioned_or_default(&manager.trust_file_path(), "plugin trust policy").await
}

pub(super) async fn save_trust_policy(
    manager: &PluginManager,
    trust: &PluginTrustPolicy,
) -> Result<()> {
    write_toml(
        &manager.trust_file_path(),
        &VersionedTrustPolicy {
            version: SUPPORTED_PLUGIN_STATE_VERSION,
            policy: trust,
        },
    )
    .await
}

pub(super) async fn load_permissions(manager: &PluginManager) -> Result<PluginPermissionsFile> {
    load_versioned_or_default(&manager.permissions_file_path(), "plugin permissions").await
}

pub(super) async fn save_permissions(
    manager: &PluginManager,
    permissions: &PluginPermissionsFile,
) -> Result<()> {
    ensure_supported_plugin_state_version(permissions.version, "plugin permissions")?;
    write_toml(&manager.permissions_file_path(), permissions).await
}

pub(super) async fn load_sources(manager: &PluginManager) -> Result<PluginSourcesFile> {
    load_versioned_or_default(&manager.sources_file_path(), "plugin sources").await
}

pub(super) async fn save_sources(
    manager: &PluginManager,
    sources: &PluginSourcesFile,
) -> Result<()> {
    ensure_supported_plugin_state_version(sources.version, "plugin sources")?;
    write_toml(&manager.sources_file_path(), sources).await
}

async fn load_versioned_or_default<T>(path: &Path, label: &str) -> Result<T>
where
    T: Default + DeserializeOwned,
{
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };

    let schema: PluginStateSchemaVersion = super::validation::parse_toml_or_json(&bytes)
        .with_context(|| format!("failed to parse {} schema in {}", label, path.display()))?;
    ensure_supported_plugin_state_version(schema.version, label)?;

    super::validation::parse_toml_or_json(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn ensure_supported_plugin_state_version(version: u32, label: &str) -> Result<()> {
    if version != SUPPORTED_PLUGIN_STATE_VERSION {
        bail!(
            "unsupported {} schema version {}; this Mitsuro build supports version {}",
            label,
            version,
            SUPPORTED_PLUGIN_STATE_VERSION
        );
    }
    Ok(())
}

pub(super) async fn read_toml_or_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
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

    if let Some(parent) = path.parent() {
        // The rename above is the commit point. A directory-fsync failure
        // cannot be reported as a normal pre-commit error: callers might then
        // delete a published snapshot even though the new lockfile names it.
        // Preserve the committed state and make the durability degradation
        // operator-visible instead.
        if let Err(error) = sync_directory(parent) {
            tracing::warn!(
                path = %path.display(),
                %error,
                "State file was replaced but its directory could not be synced"
            );
        }
    }

    Ok(())
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> Result<()> {
    std::fs::File::open(path)
        .with_context(|| format!("failed to open directory for sync: {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync directory: {}", path.display()))
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}
