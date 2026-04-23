use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest as _, Sha256};
use tokio::fs;
use tracing::warn;

use self::io::{read_local_bytes_with_limit, read_remote_bytes_with_limit};
use self::storage::{
    load_lockfile, load_sources, load_trust_policy, read_installed_from_lock_entry, save_lockfile,
    save_sources, save_trust_policy, upsert_lock_entry, write_toml,
};
use self::validation::{
    infer_source_name, resolve_artifact_location, validate_compatibility, validate_plugin_id,
    validate_plugin_version, validate_relative_path, ArtifactLocation, ManifestLocation,
    MAX_ARTIFACT_BYTES, MAX_MANIFEST_BYTES,
};
use super::{
    signing::{validate_public_key_base64, verify_artifact_signature},
    InstalledPlugin, PluginLockEntry, PluginManifestV1, PluginSource,
};

mod io;
mod storage;
#[cfg(test)]
mod tests;
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

    /// Ensure required plugin directories and config files exist.
    pub async fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.installed_root()).await?;
        fs::create_dir_all(self.active_root()).await?;
        fs::create_dir_all(self.state_root()).await?;
        fs::create_dir_all(self.index_root()).await?;
        fs::create_dir_all(self.trust_root()).await?;

        load_lockfile(self).await?;
        load_trust_policy(self).await?;
        load_sources(self).await?;

        Ok(())
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

    pub async fn add_allowed_publisher(&self, publisher: &str) -> Result<()> {
        let mut trust = load_trust_policy(self).await?;
        if !trust
            .allowed_publishers
            .iter()
            .any(|existing| existing == publisher)
        {
            trust.allowed_publishers.push(publisher.to_string());
            trust.allowed_publishers.sort();
            save_trust_policy(self, &trust).await?;
        }
        Ok(())
    }

    pub async fn add_trusted_key(&self, key_id: &str, public_key_b64: &str) -> Result<()> {
        validate_public_key_base64(public_key_b64)?;

        let mut trust = load_trust_policy(self).await?;
        trust
            .keys
            .insert(key_id.to_string(), public_key_b64.to_string());
        save_trust_policy(self, &trust).await
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

    /// Explicit reload request. v1 behavior is descriptor refresh only.
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

        installed.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(installed)
    }

    pub async fn install_from_manifest_ref(&self, manifest_ref: &str) -> Result<InstalledPlugin> {
        let (manifest, manifest_location) = self.read_manifest_from_ref(manifest_ref).await?;

        self.validate_manifest(&manifest)?;
        self.verify_publisher_allowed(&manifest.publisher).await?;

        let artifact_location =
            resolve_artifact_location(&manifest.release.url, &manifest_location)?;
        let artifact_bytes = self.read_artifact(&artifact_location).await?;

        self.verify_artifact_integrity(&manifest, &artifact_bytes)
            .await?;

        let install_dir = self
            .installed_root()
            .join(&manifest.id)
            .join(&manifest.version);
        fs::create_dir_all(&install_dir).await?;

        let manifest_path = install_dir.join("plugin.toml");
        write_toml(&manifest_path, &manifest).await?;

        let entry_component_rel = validate_relative_path(&manifest.entry_component)?;
        let entry_component_path = install_dir.join(&entry_component_rel);
        if let Some(parent) = entry_component_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&entry_component_path, artifact_bytes).await?;

        upsert_lock_entry(self, &manifest.id, &manifest.version, true, true).await?;

        read_installed_from_lock_entry(
            self,
            &PluginLockEntry {
                id: manifest.id,
                version: manifest.version,
                enabled: true,
                pinned: true,
            },
        )
        .await
    }

    async fn verify_publisher_allowed(&self, publisher: &str) -> Result<()> {
        let trust = load_trust_policy(self).await?;
        if trust
            .allowed_publishers
            .iter()
            .any(|allowed| allowed == publisher)
        {
            return Ok(());
        }

        bail!(
            "publisher '{}' is not allowlisted. Add it via plugin trust configuration first",
            publisher
        )
    }

    async fn verify_artifact_integrity(
        &self,
        manifest: &PluginManifestV1,
        artifact_bytes: &[u8],
    ) -> Result<()> {
        let digest = Sha256::digest(artifact_bytes);
        let digest_hex = format!("{:x}", digest);
        if digest_hex != manifest.release.sha256.to_lowercase() {
            bail!(
                "sha256 mismatch for '{}': expected {}, got {}",
                manifest.id,
                manifest.release.sha256,
                digest_hex
            );
        }

        let trust = load_trust_policy(self).await?;
        let public_key = trust
            .keys
            .get(&manifest.release.signing_key_id)
            .with_context(|| {
                format!(
                    "trusted key '{}' not found in trust policy",
                    manifest.release.signing_key_id
                )
            })?;

        verify_artifact_signature(artifact_bytes, &manifest.release.signature, public_key)?;
        Ok(())
    }

    async fn read_manifest_from_ref(
        &self,
        manifest_ref: &str,
    ) -> Result<(PluginManifestV1, ManifestLocation)> {
        if let Ok(url) = url::Url::parse(manifest_ref) {
            if matches!(url.scheme(), "http" | "https") {
                let bytes =
                    read_remote_bytes_with_limit(self, &url, MAX_MANIFEST_BYTES, "manifest")
                        .await?;

                let manifest = validation::parse_toml_or_json::<PluginManifestV1>(&bytes)?;
                return Ok((manifest, ManifestLocation::Remote(url)));
            }

            if url.scheme() == "file" {
                let path = url
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("invalid file URL: {}", url))?;
                let bytes =
                    read_local_bytes_with_limit(&path, MAX_MANIFEST_BYTES, "manifest").await?;
                let manifest = validation::parse_toml_or_json::<PluginManifestV1>(&bytes)?;
                return Ok((manifest, ManifestLocation::Local(path)));
            }
        }

        let path = PathBuf::from(manifest_ref);
        let bytes = read_local_bytes_with_limit(&path, MAX_MANIFEST_BYTES, "manifest").await?;
        let manifest = validation::parse_toml_or_json::<PluginManifestV1>(&bytes)?;
        Ok((manifest, ManifestLocation::Local(path)))
    }

    async fn read_artifact(&self, location: &ArtifactLocation) -> Result<Vec<u8>> {
        match location {
            ArtifactLocation::Remote(url) => {
                read_remote_bytes_with_limit(self, url, MAX_ARTIFACT_BYTES, "artifact").await
            }
            ArtifactLocation::Local(path) => {
                read_local_bytes_with_limit(path, MAX_ARTIFACT_BYTES, "artifact").await
            }
        }
    }

    fn validate_manifest(&self, manifest: &PluginManifestV1) -> Result<()> {
        if manifest.manifest_version != 1 {
            bail!(
                "unsupported manifest version '{}'; expected version 1",
                manifest.manifest_version
            );
        }

        if manifest.id.trim().is_empty() {
            bail!("manifest id cannot be empty");
        }
        if manifest.name.trim().is_empty() {
            bail!("manifest name cannot be empty");
        }
        if manifest.version.trim().is_empty() {
            bail!("manifest version cannot be empty");
        }
        if manifest.publisher.trim().is_empty() {
            bail!("manifest publisher cannot be empty");
        }
        if manifest.release.url.trim().is_empty() {
            bail!("manifest release.url cannot be empty");
        }
        if manifest.release.sha256.trim().is_empty() {
            bail!("manifest release.sha256 cannot be empty");
        }
        if manifest.release.signature.trim().is_empty() {
            bail!("manifest release.signature cannot be empty");
        }
        if manifest.release.signing_key_id.trim().is_empty() {
            bail!("manifest release.signing_key_id cannot be empty");
        }

        validate_plugin_id(&manifest.id)?;
        validate_plugin_version(&manifest.version)?;
        validate_compatibility(&manifest.compat)?;
        validate_relative_path(&manifest.entry_component)?;
        Ok(())
    }
}
