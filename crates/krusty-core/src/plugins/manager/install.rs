use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use sha2::{Digest as _, Sha256};
use tokio::fs;

use super::super::signing::verify_artifact_signature;
use super::super::{InstalledPlugin, PluginLockEntry, PluginManifestV1};
use super::io::{read_local_bytes_with_limit, read_remote_bytes_with_limit};
use super::storage::{
    load_trust_policy, read_installed_from_lock_entry, upsert_lock_entry, write_toml,
};
use super::validation::{
    self, resolve_artifact_location, validate_compatibility, validate_plugin_id,
    validate_plugin_version, validate_relative_path, ArtifactLocation, ManifestLocation,
    MAX_ARTIFACT_BYTES, MAX_MANIFEST_BYTES,
};
use super::PluginManager;

impl PluginManager {
    pub async fn install_from_manifest_ref(&self, manifest_ref: &str) -> Result<InstalledPlugin> {
        let (manifest, manifest_location) = self.read_manifest_from_ref(manifest_ref).await?;

        self.validate_manifest(&manifest, true)?;
        self.verify_publisher_allowed(&manifest.publisher).await?;

        let release = manifest
            .release
            .as_ref()
            .context("manifest release metadata is required for manifest installs")?;
        let artifact_location = resolve_artifact_location(&release.url, &manifest_location)?;
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
                package_path: None,
                manifest_path: None,
                source: None,
            },
        )
        .await
    }

    async fn verify_artifact_integrity(
        &self,
        manifest: &PluginManifestV1,
        artifact_bytes: &[u8],
    ) -> Result<()> {
        let release = manifest
            .release
            .as_ref()
            .context("manifest release metadata is required for artifact verification")?;

        let digest = Sha256::digest(artifact_bytes);
        let digest_hex = format!("{:x}", digest);
        if digest_hex != release.sha256.to_lowercase() {
            bail!(
                "sha256 mismatch for '{}': expected {}, got {}",
                manifest.id,
                release.sha256,
                digest_hex
            );
        }

        let trust = load_trust_policy(self).await?;
        let public_key = trust.keys.get(&release.signing_key_id).with_context(|| {
            format!(
                "trusted key '{}' not found in trust policy",
                release.signing_key_id
            )
        })?;

        verify_artifact_signature(artifact_bytes, &release.signature, public_key)?;
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

    pub(super) fn validate_manifest(
        &self,
        manifest: &PluginManifestV1,
        require_release: bool,
    ) -> Result<()> {
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
        if require_release && manifest.release.is_none() {
            bail!("manifest release metadata is required for manifest installs");
        }
        if let Some(release) = &manifest.release {
            if release.url.trim().is_empty() {
                bail!("manifest release.url cannot be empty");
            }
            if release.sha256.trim().is_empty() {
                bail!("manifest release.sha256 cannot be empty");
            }
            if release.signature.trim().is_empty() {
                bail!("manifest release.signature cannot be empty");
            }
            if release.signing_key_id.trim().is_empty() {
                bail!("manifest release.signing_key_id cannot be empty");
            }
        }

        validate_plugin_id(&manifest.id)?;
        validate_plugin_version(&manifest.version)?;
        validate_compatibility(&manifest.compat)?;
        validate_relative_path(&manifest.entry_component)?;
        Ok(())
    }
}
