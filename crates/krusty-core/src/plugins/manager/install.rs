use std::{
    collections::BTreeSet,
    fs::OpenOptions as StdOpenOptions,
    io::{Cursor, Read as _, Write as _},
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use sha2::{Digest as _, Sha256};
use tokio::{fs, io::AsyncWriteExt as _, task};
use zip::ZipArchive;

use super::super::signing::{
    plugin_release_signing_payload, validate_release_signature_scheme, verify_artifact_signature,
};
use super::super::{
    InstalledPlugin, PluginInstallOptions, PluginLockEntry, PluginManifestV1,
    PluginReleaseArtifactKind, PluginSourceTrust,
};
use super::io::{read_local_bytes_with_limit, read_remote_bytes_with_limit};
use super::package::{
    MAX_PACKAGE_SNAPSHOT_BYTES, MAX_PACKAGE_SNAPSHOT_ENTRIES, MAX_PACKAGE_SNAPSHOT_FILE_BYTES,
};
use super::storage::{load_lockfile, load_trust_policy, write_toml};
use super::transaction::{commit_staged_install, create_staging_root};
use super::validation::{
    self, resolve_artifact_location, validate_compatibility, validate_plugin_id,
    validate_plugin_version, validate_relative_path_for, ArtifactLocation, ManifestLocation,
    MAX_ARTIFACT_BYTES, MAX_MANIFEST_BYTES,
};
use super::PluginManager;

impl PluginManager {
    pub async fn install_from_manifest_ref(&self, manifest_ref: &str) -> Result<InstalledPlugin> {
        self.install_from_manifest_ref_with_options(manifest_ref, PluginInstallOptions::default())
            .await
    }

    pub async fn install_from_manifest_ref_with_options(
        &self,
        manifest_ref: &str,
        options: PluginInstallOptions,
    ) -> Result<InstalledPlugin> {
        let _guard = self.acquire_mutation().await?;
        self.install_from_manifest_ref_with_options_unlocked(manifest_ref, options)
            .await
    }

    pub(super) async fn install_from_manifest_ref_with_options_unlocked(
        &self,
        manifest_ref: &str,
        options: PluginInstallOptions,
    ) -> Result<InstalledPlugin> {
        let (manifest, manifest_location) = self.read_manifest_from_ref(manifest_ref).await?;
        let manifest_source = normalized_manifest_source(&manifest_location)?;

        self.validate_manifest(&manifest, true)?;
        self.verify_publisher_allowed(&manifest.publisher).await?;

        let release = manifest
            .release
            .as_ref()
            .context("manifest release metadata is required for manifest installs")?;
        self.verify_publisher_key_binding(&manifest.publisher, &release.signing_key_id)
            .await?;
        let artifact_location = resolve_artifact_location(&release.url, &manifest_location)?;
        self.verify_release_envelope(&manifest).await?;
        let artifact_bytes = self.read_artifact(&artifact_location).await?;
        self.verify_artifact_digest(&manifest, &artifact_bytes)?;

        let staging_root = create_staging_root(self).await?;
        let payload_root = staging_root.join("payload");
        let staged_result = async {
            fs::create_dir_all(&payload_root).await?;
            write_toml(&payload_root.join("plugin.toml"), &manifest).await?;
            stage_signed_release_artifact(&manifest, artifact_bytes, &payload_root).await?;

            let previous_lock = load_lockfile(self).await?;
            let existing = previous_lock
                .plugins
                .iter()
                .find(|entry| entry.id == manifest.id);
            let entry = PluginLockEntry {
                id: manifest.id.clone(),
                version: manifest.version.clone(),
                enabled: existing.map(|entry| entry.enabled).unwrap_or(true),
                pinned: options
                    .pinned
                    .or_else(|| existing.map(|entry| entry.pinned))
                    .unwrap_or(true),
                package_path: Some(payload_root),
                manifest_path: Some(PathBuf::from("plugin.toml")),
                source: Some(manifest_source),
                managed_root: Some(staging_root.clone()),
                source_trust: PluginSourceTrust::SignedPublisher,
                package_scripts_allowed: false,
            };
            let mut installed = commit_staged_install(self, &staging_root, vec![entry]).await?;
            installed
                .pop()
                .context("signed plugin transaction returned no installed plugin")
        }
        .await;

        if staged_result.is_err() && staging_root.exists() {
            let _ = fs::remove_dir_all(&staging_root).await;
        }
        staged_result
    }

    fn verify_artifact_digest(
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

        Ok(())
    }

    async fn verify_release_envelope(&self, manifest: &PluginManifestV1) -> Result<()> {
        let release = manifest
            .release
            .as_ref()
            .context("manifest release metadata is required for signature verification")?;
        let trust = load_trust_policy(self).await?;
        let public_key = trust.keys.get(&release.signing_key_id).with_context(|| {
            format!(
                "trusted key '{}' not found in trust policy",
                release.signing_key_id
            )
        })?;

        let signed_payload = plugin_release_signing_payload(manifest)?;
        verify_artifact_signature(&signed_payload, &release.signature, public_key)?;
        Ok(())
    }

    async fn read_manifest_from_ref(
        &self,
        manifest_ref: &str,
    ) -> Result<(PluginManifestV1, ManifestLocation)> {
        if let Ok(url) = url::Url::parse(manifest_ref) {
            match url.scheme() {
                "https" | "http" => {
                    let bytes =
                        read_remote_bytes_with_limit(self, &url, MAX_MANIFEST_BYTES, "manifest")
                            .await?;
                    let manifest = validation::parse_toml_or_json::<PluginManifestV1>(&bytes)?;
                    return Ok((manifest, ManifestLocation::Remote(url)));
                }
                "file" => {
                    let path = url
                        .to_file_path()
                        .map_err(|_| anyhow::anyhow!("invalid file URL: {}", url))?;
                    let path = fs::canonicalize(&path).await.with_context(|| {
                        format!("failed to canonicalize manifest {}", path.display())
                    })?;
                    let bytes =
                        read_local_bytes_with_limit(&path, MAX_MANIFEST_BYTES, "manifest").await?;
                    let manifest = validation::parse_toml_or_json::<PluginManifestV1>(&bytes)?;
                    return Ok((manifest, ManifestLocation::Local(path)));
                }
                scheme if !scheme.is_empty() => {
                    bail!("unsupported manifest URL scheme '{}': {}", scheme, url)
                }
                _ => {}
            }
        }

        let path = PathBuf::from(manifest_ref);
        let path = fs::canonicalize(&path)
            .await
            .with_context(|| format!("failed to canonicalize manifest {}", path.display()))?;
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
        if require_release
            && manifest.entry_component.is_none()
            && manifest.release.as_ref().is_some_and(|release| {
                release.artifact_kind == PluginReleaseArtifactKind::SingleComponent
            })
        {
            bail!("signed single-component manifests require entry_component");
        }
        if manifest.entry_component.is_none()
            && manifest.skills.is_empty()
            && manifest.agent_extensions.is_empty()
            && manifest.mcp_servers.is_none()
            && manifest.hooks.is_empty()
            && manifest.assets.is_none()
        {
            bail!("plugin manifest must declare at least one bundle component");
        }
        if manifest.entry_component.is_some()
            && manifest.runtime.requires_process_permission()
            && !manifest.requested_permissions.process
        {
            let runtime = match manifest.runtime {
                crate::plugins::PluginRuntime::Native => "native",
                crate::plugins::PluginRuntime::Js => "js",
                crate::plugins::PluginRuntime::Wasm => {
                    unreachable!("WASM does not require process permission for an entry component")
                }
            };
            bail!("{runtime} entry_component requires requested_permissions.process = true");
        }
        if (!manifest.agent_extensions.is_empty() || !manifest.hooks.is_empty())
            && !manifest.requested_permissions.process
        {
            bail!("agent_extensions and hooks require requested_permissions.process = true");
        }
        if manifest.mcp_servers.is_some()
            && !manifest.requested_permissions.process
            && !manifest.requested_permissions.network
        {
            bail!(
                "mcp_servers requires requested_permissions.process or requested_permissions.network"
            );
        }
        if let Some(release) = &manifest.release {
            if release.url.trim().is_empty() {
                bail!("manifest release.url cannot be empty");
            }
            if release.sha256.len() != 64
                || !release.sha256.chars().all(|ch| ch.is_ascii_hexdigit())
            {
                bail!("manifest release.sha256 must be a 64-character hexadecimal digest");
            }
            if release.signature.trim().is_empty() {
                bail!("manifest release.signature cannot be empty");
            }
            if release.signing_key_id.trim().is_empty() {
                bail!("manifest release.signing_key_id cannot be empty");
            }
            if require_release || release.signature_scheme.is_some() {
                validate_release_signature_scheme(release)?;
            }
        }

        validate_plugin_id(&manifest.id)?;
        validate_plugin_version(&manifest.version)?;
        validate_compatibility(&manifest.compat)?;
        validate_manifest_component_paths(manifest)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct SignedBundleLimits {
    max_entries: usize,
    max_total_bytes: u64,
    max_file_bytes: u64,
}

const SIGNED_BUNDLE_LIMITS: SignedBundleLimits = SignedBundleLimits {
    max_entries: MAX_PACKAGE_SNAPSHOT_ENTRIES,
    max_total_bytes: MAX_PACKAGE_SNAPSHOT_BYTES,
    max_file_bytes: MAX_PACKAGE_SNAPSHOT_FILE_BYTES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignedBundleEntryKind {
    Directory,
    File,
}

#[derive(Debug)]
struct SignedBundleEntry {
    index: usize,
    path: PathBuf,
    kind: SignedBundleEntryKind,
    declared_size: u64,
}

async fn stage_signed_release_artifact(
    manifest: &PluginManifestV1,
    artifact_bytes: Vec<u8>,
    payload_root: &Path,
) -> Result<()> {
    let artifact_kind = manifest
        .release
        .as_ref()
        .context("manifest release metadata is required for artifact staging")?
        .artifact_kind;

    match artifact_kind {
        PluginReleaseArtifactKind::SingleComponent => {
            let entry_component = manifest
                .entry_component
                .as_deref()
                .context("signed single-component manifests require entry_component")?;
            let entry_rel = validate_relative_path_for(entry_component, "entry_component")?;
            let entry_path = payload_root.join(entry_rel);
            if let Some(parent) = entry_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&entry_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to create signed artifact without overwriting {}",
                        entry_path.display()
                    )
                })?;
            output
                .write_all(&artifact_bytes)
                .await
                .with_context(|| format!("failed to stage artifact {}", entry_path.display()))?;
            output
                .sync_all()
                .await
                .with_context(|| format!("failed to sync artifact {}", entry_path.display()))?;
            Ok(())
        }
        PluginReleaseArtifactKind::ZipBundle => {
            let payload_root = payload_root.to_path_buf();
            task::spawn_blocking(move || {
                extract_signed_zip_bundle(&artifact_bytes, &payload_root, SIGNED_BUNDLE_LIMITS)
            })
            .await
            .context("signed zip-bundle extraction task failed")?
        }
    }
}

/// Reject ZIP64 before `ZipArchive` allocates central-directory metadata. A
/// standard ZIP cannot advertise more than 65,535 entries, which is already
/// below Krusty's snapshot limit. ZIP64 is unnecessary here because per-file,
/// aggregate, and compressed-artifact limits are all far below its thresholds.
fn preflight_standard_zip(artifact_bytes: &[u8], max_entries: usize) -> Result<usize> {
    const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const EOCD_LEN: usize = 22;
    const MAX_COMMENT_LEN: usize = u16::MAX as usize;

    if artifact_bytes.len() < EOCD_LEN {
        bail!("release artifact is too short to be a standard zip");
    }
    let search_start = artifact_bytes
        .len()
        .saturating_sub(EOCD_LEN + MAX_COMMENT_LEN);
    let eocd = (search_start..=artifact_bytes.len() - EOCD_LEN)
        .rev()
        .find(|offset| {
            artifact_bytes[*offset..].starts_with(EOCD_SIGNATURE)
                && read_zip_u16(artifact_bytes, *offset + 20).is_some_and(|comment_len| {
                    *offset + EOCD_LEN + comment_len as usize == artifact_bytes.len()
                })
        })
        .context("release artifact has no bounded standard ZIP end record")?;
    if eocd >= 20 && &artifact_bytes[eocd - 20..eocd - 16] == b"PK\x06\x07" {
        bail!("ZIP64 bundles are not supported by the bounded signed-bundle format");
    }

    let disk = read_zip_u16(artifact_bytes, eocd + 4).context("truncated ZIP end record")?;
    let central_disk =
        read_zip_u16(artifact_bytes, eocd + 6).context("truncated ZIP end record")?;
    let disk_entries =
        read_zip_u16(artifact_bytes, eocd + 8).context("truncated ZIP end record")?;
    let total_entries =
        read_zip_u16(artifact_bytes, eocd + 10).context("truncated ZIP end record")?;
    let central_size =
        read_zip_u32(artifact_bytes, eocd + 12).context("truncated ZIP end record")?;
    let central_offset =
        read_zip_u32(artifact_bytes, eocd + 16).context("truncated ZIP end record")?;

    if disk != 0 || central_disk != 0 || disk_entries != total_entries {
        bail!("multi-disk ZIP bundles are not supported");
    }
    if total_entries == u16::MAX || central_size == u32::MAX || central_offset == u32::MAX {
        bail!("ZIP64 bundles are not supported by the bounded signed-bundle format");
    }
    let total_entries = total_entries as usize;
    if total_entries > max_entries {
        bail!(
            "signed zip bundle contains {} entries; limit is {}",
            total_entries,
            max_entries
        );
    }
    let central_end = (central_offset as usize)
        .checked_add(central_size as usize)
        .context("ZIP central-directory bounds overflow")?;
    if central_end > eocd {
        bail!("ZIP central directory escapes its bounded end record");
    }
    Ok(total_entries)
}

fn read_zip_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(raw))
}

fn read_zip_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

fn extract_signed_zip_bundle(
    artifact_bytes: &[u8],
    payload_root: &Path,
    limits: SignedBundleLimits,
) -> Result<()> {
    if artifact_bytes.len() > MAX_ARTIFACT_BYTES {
        bail!(
            "signed zip bundle exceeds compressed artifact limit of {} bytes",
            MAX_ARTIFACT_BYTES
        );
    }

    let expected_archive_entries = preflight_standard_zip(artifact_bytes, limits.max_entries)?;

    let root_metadata = std::fs::symlink_metadata(payload_root).with_context(|| {
        format!(
            "failed to inspect signed bundle destination {}",
            payload_root.display()
        )
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!(
            "signed bundle destination must be a real directory: {}",
            payload_root.display()
        );
    }
    let canonical_root = std::fs::canonicalize(payload_root).with_context(|| {
        format!(
            "failed to canonicalize signed bundle destination {}",
            payload_root.display()
        )
    })?;

    let authenticated_manifest = payload_root.join("plugin.toml");
    let manifest_metadata =
        std::fs::symlink_metadata(&authenticated_manifest).with_context(|| {
            format!(
                "signed bundle destination is missing authenticated manifest {}",
                authenticated_manifest.display()
            )
        })?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        bail!(
            "authenticated plugin manifest must be a real regular file: {}",
            authenticated_manifest.display()
        );
    }
    if manifest_metadata.len() > limits.max_file_bytes {
        bail!(
            "authenticated plugin manifest is {} bytes; per-file limit is {} bytes",
            manifest_metadata.len(),
            limits.max_file_bytes
        );
    }
    if manifest_metadata.len() > limits.max_total_bytes {
        bail!(
            "signed bundle snapshot exceeds aggregate limit of {} bytes before extraction",
            limits.max_total_bytes
        );
    }

    let cursor = Cursor::new(artifact_bytes);
    let mut archive = ZipArchive::new(cursor).context("release artifact is not a valid zip")?;
    if archive.len() != expected_archive_entries {
        bail!(
            "zip central-directory entry count changed during parsing: expected {}, read {}",
            expected_archive_entries,
            archive.len()
        );
    }
    if archive.len() > limits.max_entries {
        bail!(
            "signed zip bundle contains {} entries; limit is {}",
            archive.len(),
            limits.max_entries
        );
    }

    let mut plans = Vec::with_capacity(archive.len());
    let mut seen_paths = BTreeSet::new();
    let mut materialized_paths = BTreeSet::from([PathBuf::from("plugin.toml")]);
    if materialized_paths.len().saturating_add(1) > limits.max_entries {
        bail!(
            "signed zip bundle materializes more than {} filesystem entries including the transaction root and authenticated manifest",
            limits.max_entries
        );
    }
    let mut declared_total = manifest_metadata.len();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .with_context(|| format!("failed to inspect zip entry {}", index))?;
        if entry.name().chars().any(|ch| matches!(ch, '\\' | ':')) {
            bail!(
                "signed zip bundle entry {} contains a cross-platform path separator, drive prefix, or alternate data stream",
                index
            );
        }
        let raw_path = entry.enclosed_name().with_context(|| {
            format!(
                "signed zip bundle entry {} has an absolute, escaping, or invalid path",
                index
            )
        })?;
        let path = normalize_signed_bundle_path(&raw_path, index)?;
        if path == Path::new("plugin.toml") {
            bail!(
                "signed zip bundle entry {} cannot overwrite the authenticated plugin.toml",
                index
            );
        }
        if !seen_paths.insert(path.clone()) {
            bail!(
                "signed zip bundle contains duplicate path '{}'",
                path.display()
            );
        }
        for materialized in path.ancestors() {
            if materialized.as_os_str().is_empty() {
                break;
            }
            materialized_paths.insert(materialized.to_path_buf());
        }
        let materialized_entry_count = materialized_paths
            .len()
            .checked_add(1)
            .context("signed zip bundle materialized entry count overflow")?;
        if materialized_entry_count > limits.max_entries {
            bail!(
                "signed zip bundle materializes more than {} filesystem entries including implicit directories and the authenticated manifest",
                limits.max_entries
            );
        }

        const MODE_TYPE_MASK: u32 = 0o170000;
        const MODE_DIRECTORY: u32 = 0o040000;
        const MODE_REGULAR: u32 = 0o100000;
        let kind = match entry.unix_mode() {
            Some(mode) => match mode & MODE_TYPE_MASK {
                MODE_DIRECTORY if entry.is_dir() => SignedBundleEntryKind::Directory,
                MODE_REGULAR if !entry.is_dir() => SignedBundleEntryKind::File,
                _ => {
                    bail!(
                        "signed zip bundle entry '{}' is a symlink, special file, or has inconsistent type metadata",
                        path.display()
                    )
                }
            },
            // ZIPs produced on Windows commonly omit Unix mode metadata. In
            // that representation there is no special-file type to honor: a
            // trailing slash denotes a directory and every other entry is
            // materialized by this extractor as a new regular file.
            None if entry.is_dir() => SignedBundleEntryKind::Directory,
            None => SignedBundleEntryKind::File,
        };

        let declared_size = entry.size();
        match kind {
            SignedBundleEntryKind::Directory if declared_size != 0 => {
                bail!(
                    "signed zip bundle directory '{}' unexpectedly contains data",
                    path.display()
                )
            }
            SignedBundleEntryKind::File => {
                if declared_size > limits.max_file_bytes {
                    bail!(
                        "signed zip bundle file '{}' is {} bytes; per-file limit is {} bytes",
                        path.display(),
                        declared_size,
                        limits.max_file_bytes
                    );
                }
                declared_total = declared_total.checked_add(declared_size).with_context(|| {
                    format!(
                        "signed zip bundle size overflow while inspecting '{}'",
                        path.display()
                    )
                })?;
                if declared_total > limits.max_total_bytes {
                    bail!(
                        "signed zip bundle expands to more than {} bytes",
                        limits.max_total_bytes
                    );
                }
            }
            SignedBundleEntryKind::Directory => {}
        }

        plans.push(SignedBundleEntry {
            index,
            path,
            kind,
            declared_size,
        });
    }

    let mut regular_paths: BTreeSet<&Path> = plans
        .iter()
        .filter(|entry| entry.kind == SignedBundleEntryKind::File)
        .map(|entry| entry.path.as_path())
        .collect();
    regular_paths.insert(Path::new("plugin.toml"));
    for plan in &plans {
        for ancestor in plan.path.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            if regular_paths.contains(ancestor) {
                bail!(
                    "signed zip bundle path '{}' is nested beneath file '{}'",
                    plan.path.display(),
                    ancestor.display()
                );
            }
        }
    }
    drop(regular_paths);

    let mut extracted_total = manifest_metadata.len();
    for plan in plans {
        let destination = canonical_root.join(&plan.path);
        match plan.kind {
            SignedBundleEntryKind::Directory => {
                ensure_signed_bundle_directory(&canonical_root, &destination)?;
            }
            SignedBundleEntryKind::File => {
                let parent = destination
                    .parent()
                    .context("signed bundle file has no parent directory")?;
                ensure_signed_bundle_directory(&canonical_root, parent)?;

                let mut output = StdOpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .with_context(|| {
                        format!(
                            "failed to create signed bundle file without overwriting {}",
                            destination.display()
                        )
                    })?;
                let entry = archive.by_index(plan.index).with_context(|| {
                    format!("failed to open signed zip entry '{}'", plan.path.display())
                })?;
                let aggregate_remaining = limits
                    .max_total_bytes
                    .checked_sub(extracted_total)
                    .context("signed zip bundle exceeded aggregate extraction limit")?;
                let read_limit = limits
                    .max_file_bytes
                    .min(aggregate_remaining)
                    .saturating_add(1);
                let copied =
                    std::io::copy(&mut entry.take(read_limit), &mut output).with_context(|| {
                        format!(
                            "failed to extract signed bundle file {}",
                            plan.path.display()
                        )
                    })?;
                if copied > limits.max_file_bytes {
                    bail!(
                        "signed zip bundle file '{}' exceeds per-file limit of {} bytes",
                        plan.path.display(),
                        limits.max_file_bytes
                    );
                }
                extracted_total = extracted_total.checked_add(copied).with_context(|| {
                    format!(
                        "signed zip bundle size overflow while extracting '{}'",
                        plan.path.display()
                    )
                })?;
                if extracted_total > limits.max_total_bytes {
                    bail!(
                        "signed zip bundle expands to more than {} bytes",
                        limits.max_total_bytes
                    );
                }
                if copied != plan.declared_size {
                    bail!(
                        "signed zip bundle file '{}' size changed during extraction: declared {}, read {}",
                        plan.path.display(),
                        plan.declared_size,
                        copied
                    );
                }
                output.flush().with_context(|| {
                    format!(
                        "failed to flush signed bundle file {}",
                        destination.display()
                    )
                })?;
                output.sync_all().with_context(|| {
                    format!(
                        "failed to sync signed bundle file {}",
                        destination.display()
                    )
                })?;
            }
        }
    }

    Ok(())
}

fn normalize_signed_bundle_path(path: &Path, index: usize) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("signed zip bundle entry {} contains path traversal", index)
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        bail!("signed zip bundle entry {} has an empty path", index);
    }
    Ok(normalized)
}

fn ensure_signed_bundle_directory(canonical_root: &Path, directory: &Path) -> Result<()> {
    let relative = directory.strip_prefix(canonical_root).with_context(|| {
        format!(
            "signed bundle directory is outside the transaction root: {}",
            directory.display()
        )
    })?;
    let mut current = canonical_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            bail!(
                "signed bundle directory has an invalid component: {}",
                directory.display()
            );
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    bail!(
                        "signed bundle path is not a real directory: {}",
                        current.display()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(create_error) = std::fs::create_dir(&current) {
                    if create_error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(create_error).with_context(|| {
                            format!(
                                "failed to create signed bundle directory {}",
                                current.display()
                            )
                        });
                    }
                }
                let metadata = std::fs::symlink_metadata(&current).with_context(|| {
                    format!(
                        "failed to inspect newly created signed bundle directory {}",
                        current.display()
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    bail!(
                        "signed bundle path became a symlink or non-directory: {}",
                        current.display()
                    );
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect signed bundle directory {}",
                        current.display()
                    )
                })
            }
        }
        let canonical = std::fs::canonicalize(&current).with_context(|| {
            format!(
                "failed to canonicalize signed bundle directory {}",
                current.display()
            )
        })?;
        if !canonical.starts_with(canonical_root) {
            bail!(
                "signed bundle directory escapes the transaction root: {}",
                current.display()
            );
        }
    }
    Ok(())
}

fn normalized_manifest_source(location: &ManifestLocation) -> Result<String> {
    match location {
        ManifestLocation::Remote(url) => Ok(url.to_string()),
        ManifestLocation::Local(path) => path.to_str().map(str::to_owned).with_context(|| {
            format!(
                "canonical manifest path cannot be recorded because it is not valid UTF-8: {}",
                path.display()
            )
        }),
    }
}

fn validate_manifest_component_paths(manifest: &PluginManifestV1) -> Result<()> {
    if let Some(path) = manifest.entry_component.as_deref() {
        validate_relative_path_for(path, "entry_component")?;
    }
    for (index, path) in manifest.skills.iter().enumerate() {
        validate_relative_path_for(path, &format!("skills[{}]", index))?;
    }
    for (index, path) in manifest.agent_extensions.iter().enumerate() {
        validate_relative_path_for(path, &format!("agent_extensions[{}]", index))?;
    }
    if let Some(path) = manifest.mcp_servers.as_deref() {
        validate_relative_path_for(path, "mcp_servers")?;
    }
    for (index, path) in manifest.hooks.iter().enumerate() {
        validate_relative_path_for(path, &format!("hooks[{}]", index))?;
        let extension = PathBuf::from(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("json" | "toml")) {
            bail!(
                "hooks[{}] must be a declarative .json or .toml config; executable hooks belong in agent_extensions",
                index
            );
        }
    }
    if let Some(path) = manifest.assets.as_deref() {
        validate_relative_path_for(path, "assets")?;
    }
    Ok(())
}

#[cfg(test)]
mod signed_bundle_tests {
    use std::io::{Cursor, Write as _};

    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use ed25519_dalek::{Signer as _, SigningKey};
    use tempfile::{tempdir, TempDir};
    use zip::{write::SimpleFileOptions, ZipWriter};

    use super::*;

    enum TestZipEntry<'a> {
        File(&'a str, &'a [u8]),
        Directory(&'a str),
        Symlink(&'a str, &'a str),
    }

    fn test_zip(entries: &[TestZipEntry<'_>]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for entry in entries {
            match entry {
                TestZipEntry::File(path, contents) => {
                    writer.start_file(*path, options).expect("start zip file");
                    writer.write_all(contents).expect("write zip file");
                }
                TestZipEntry::Directory(path) => writer
                    .add_directory(*path, options)
                    .expect("add zip directory"),
                TestZipEntry::Symlink(path, target) => writer
                    .add_symlink(*path, *target, options)
                    .expect("add zip symlink"),
            }
        }
        writer.finish().expect("finish zip").into_inner()
    }

    fn test_payload(temp: &TempDir) -> PathBuf {
        let payload = temp.path().join("payload");
        std::fs::create_dir(&payload).expect("create payload");
        std::fs::write(payload.join("plugin.toml"), b"").expect("write manifest sentinel");
        payload
    }

    fn strip_zip_type_metadata(mut artifact: Vec<u8>) -> Vec<u8> {
        let mut cursor = 0;
        while let Some(relative) = artifact[cursor..]
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
        {
            let central = cursor + relative;
            artifact[central + 5] = 99;
            artifact[central + 38..central + 42].fill(0);
            cursor = central + 4;
        }
        artifact
    }

    async fn signed_bundle_fixture(
        artifact: &[u8],
        component_declarations: &str,
    ) -> (TempDir, PluginManager, PathBuf) {
        let temp = tempdir().expect("tempdir");
        let manifest_dir = temp.path().join("manifest");
        fs::create_dir_all(&manifest_dir)
            .await
            .expect("create manifest directory");
        fs::write(manifest_dir.join("bundle.zip"), artifact)
            .await
            .expect("write bundle artifact");

        let signing_key = SigningKey::from_bytes(&[71_u8; 32]);
        let digest = format!("{:x}", Sha256::digest(artifact));
        let unsigned = format!(
            r#"
manifest_version = 1
id = "signed.bundle"
name = "Signed Bundle"
version = "1.0.0"
publisher = "bundle.publisher"
{component_declarations}

[release]
url = "bundle.zip"
sha256 = "{digest}"
signature = "SIGNATURE_PLACEHOLDER"
signing_key_id = "bundle-key"
signature_scheme = "manifest-envelope-v1"
artifact_kind = "zip-bundle"
"#
        );
        let manifest: PluginManifestV1 = toml::from_str(&unsigned).expect("parse manifest");
        let signature = signing_key.sign(
            &plugin_release_signing_payload(&manifest).expect("create release signing payload"),
        );
        let manifest_path = manifest_dir.join("plugin.toml");
        fs::write(
            &manifest_path,
            unsigned.replace(
                "SIGNATURE_PLACEHOLDER",
                &BASE64.encode(signature.to_bytes()),
            ),
        )
        .await
        .expect("write signed manifest");

        let manager = PluginManager::new(reqwest::Client::new(), temp.path().join("plugins"));
        manager.ensure_layout().await.expect("ensure plugin layout");
        manager
            .add_allowed_publisher("bundle.publisher")
            .await
            .expect("allow publisher");
        manager
            .add_trusted_key_for_publisher(
                "bundle.publisher",
                "bundle-key",
                &BASE64.encode(signing_key.verifying_key().to_bytes()),
            )
            .await
            .expect("bind publisher key");

        (temp, manager, manifest_path)
    }

    #[test]
    fn zip_bundle_rejects_path_traversal() {
        let temp = tempdir().expect("tempdir");
        let payload = test_payload(&temp);
        let artifact = test_zip(&[TestZipEntry::File("../escape.txt", b"escape")]);

        let error = extract_signed_zip_bundle(&artifact, &payload, SIGNED_BUNDLE_LIMITS)
            .expect_err("traversal must fail");

        assert!(error.to_string().contains("absolute, escaping, or invalid"));
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn zip_bundle_rejects_symlinks_and_special_entries() {
        let temp = tempdir().expect("tempdir");
        let payload = test_payload(&temp);
        let artifact = test_zip(&[TestZipEntry::Symlink("link", "outside")]);

        let error = extract_signed_zip_bundle(&artifact, &payload, SIGNED_BUNDLE_LIMITS)
            .expect_err("symlink must fail");

        assert!(error.to_string().contains("symlink, special file"));
        assert!(!payload.join("link").exists());
    }

    #[test]
    fn zip_bundle_rejects_cross_platform_path_syntax_and_manifest_overwrite() {
        for unsafe_path in ["nested\\escape.txt", "drive:C.txt", "plugin.toml"] {
            let temp = tempdir().expect("tempdir");
            let payload = test_payload(&temp);
            std::fs::write(payload.join("plugin.toml"), b"authenticated")
                .expect("write manifest sentinel");
            let artifact = test_zip(&[TestZipEntry::File(unsafe_path, b"replacement")]);

            extract_signed_zip_bundle(&artifact, &payload, SIGNED_BUNDLE_LIMITS)
                .expect_err("unsafe path must fail");
            assert_eq!(
                std::fs::read(payload.join("plugin.toml")).expect("read manifest sentinel"),
                b"authenticated"
            );
        }
    }

    #[test]
    fn zip_bundle_enforces_file_and_aggregate_quotas_on_decompressed_bytes() {
        let limits = SignedBundleLimits {
            max_entries: 4,
            max_total_bytes: 6,
            max_file_bytes: 4,
        };
        let temp = tempdir().expect("tempdir");
        let payload = test_payload(&temp);
        let oversized_file = test_zip(&[TestZipEntry::File("large.txt", b"12345")]);
        let error = extract_signed_zip_bundle(&oversized_file, &payload, limits)
            .expect_err("per-file quota must fail");
        assert!(error.to_string().contains("per-file limit"));

        let aggregate = test_zip(&[
            TestZipEntry::File("one.txt", b"1234"),
            TestZipEntry::File("two.txt", b"5678"),
        ]);
        let error = extract_signed_zip_bundle(&aggregate, &payload, limits)
            .expect_err("aggregate quota must fail");
        assert!(error.to_string().contains("expands to more than"));
    }

    #[test]
    fn zip_bundle_counts_implicit_directories_and_rejects_duplicate_paths() {
        let temp = tempdir().expect("tempdir");
        let payload = test_payload(&temp);
        let limits = SignedBundleLimits {
            max_entries: 3,
            max_total_bytes: 64,
            max_file_bytes: 64,
        };
        let implicit_directories = test_zip(&[TestZipEntry::File("one/two/file.txt", b"x")]);
        let error = extract_signed_zip_bundle(&implicit_directories, &payload, limits)
            .expect_err("implicit directories must count toward the entry quota");
        assert!(error.to_string().contains("materializes more than"));

        let mut duplicate = test_zip(&[
            TestZipEntry::File("duplicate.txt", b"one"),
            TestZipEntry::File("duplicatf.txt", b"two"),
        ]);
        // Recent `zip` releases reject duplicate names while constructing an
        // archive. Rewrite the equal-length second name in both its local and
        // central-directory records so the extractor still gets a realistic
        // hostile archive to validate.
        let mut replacements = 0;
        for offset in 0..=duplicate.len() - b"duplicatf.txt".len() {
            if &duplicate[offset..offset + b"duplicatf.txt".len()] == b"duplicatf.txt" {
                duplicate[offset..offset + b"duplicate.txt".len()]
                    .copy_from_slice(b"duplicate.txt");
                replacements += 1;
            }
        }
        assert_eq!(
            replacements, 2,
            "local and central names should be rewritten"
        );
        let error = extract_signed_zip_bundle(&duplicate, &payload, SIGNED_BUNDLE_LIMITS)
            .expect_err("duplicate archive paths must fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("duplicate path")
                || message.contains("Duplicate filename")
                || message.contains("entry count changed during parsing"),
            "{message}"
        );
    }

    #[test]
    fn zip_bundle_accepts_windows_archives_without_unix_type_metadata() {
        let temp = tempdir().expect("tempdir");
        let payload = test_payload(&temp);
        let artifact = strip_zip_type_metadata(test_zip(&[
            TestZipEntry::Directory("assets/"),
            TestZipEntry::File("assets/readme.txt", b"portable"),
        ]));

        extract_signed_zip_bundle(&artifact, &payload, SIGNED_BUNDLE_LIMITS)
            .expect("Windows-style archive should extract safely");

        assert_eq!(
            std::fs::read(payload.join("assets/readme.txt")).expect("read portable file"),
            b"portable"
        );
    }

    #[test]
    fn zip_bundle_rejects_zip64_before_archive_metadata_allocation() {
        let mut artifact = test_zip(&[TestZipEntry::File("file.txt", b"content")]);
        let eocd = artifact
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .expect("EOCD");
        artifact[eocd + 8..eocd + 12].fill(0xff);
        let temp = tempdir().expect("tempdir");
        let payload = test_payload(&temp);

        let error = extract_signed_zip_bundle(&artifact, &payload, SIGNED_BUNDLE_LIMITS)
            .expect_err("ZIP64 sentinel must fail before parser allocation");

        assert!(error
            .to_string()
            .contains("ZIP64 bundles are not supported"));
    }

    #[tokio::test]
    async fn signed_multi_resource_zip_bundle_installs_transactionally_without_entry_component() {
        let artifact = test_zip(&[
            TestZipEntry::Directory("assets/"),
            TestZipEntry::File("assets/readme.txt", b"authenticated asset"),
            TestZipEntry::File(
                "skills/demo/SKILL.md",
                b"---\nname: demo\ndescription: demo\n---\n",
            ),
        ]);
        let (_temp, manager, manifest_path) = signed_bundle_fixture(
            &artifact,
            r#"skills = ["skills/demo/SKILL.md"]
assets = "assets""#,
        )
        .await;

        let installed = manager
            .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
            .await
            .expect("install signed bundle");

        assert!(installed.entry_component_path.is_none());
        assert_eq!(installed.skill_paths.len(), 1);
        assert_eq!(
            fs::read(&installed.skill_paths[0])
                .await
                .expect("read installed skill"),
            b"---\nname: demo\ndescription: demo\n---\n"
        );
        assert!(installed
            .assets_path
            .as_ref()
            .expect("installed assets")
            .join("readme.txt")
            .exists());
        assert_eq!(
            manager
                .list_installed_plugins()
                .await
                .expect("list installed plugins")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn signed_zip_bundle_rejects_missing_declared_component_before_publication() {
        let artifact = test_zip(&[TestZipEntry::File("other.txt", b"present")]);
        let (_temp, manager, manifest_path) =
            signed_bundle_fixture(&artifact, r#"skills = ["skills/missing/SKILL.md"]"#).await;

        let error = manager
            .install_from_manifest_ref(manifest_path.to_str().expect("manifest path utf8"))
            .await
            .expect_err("missing signed component must fail");

        assert!(format!("{error:#}").contains("skills[0] does not exist"));
        assert!(manager
            .list_installed_plugins()
            .await
            .expect("list installed plugins")
            .is_empty());
    }
}
