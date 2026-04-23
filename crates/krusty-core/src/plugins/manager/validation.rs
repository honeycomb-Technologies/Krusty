use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use semver::Version;
use serde::de::DeserializeOwned;
use url::Url;

use crate::plugins::PluginCompat;

pub(super) const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub(super) const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) enum ManifestLocation {
    Remote(Url),
    Local(PathBuf),
}

#[derive(Debug, Clone)]
pub(super) enum ArtifactLocation {
    Remote(Url),
    Local(PathBuf),
}

pub(super) fn parse_toml_or_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if let Ok(as_toml) = toml::from_str::<T>(&String::from_utf8_lossy(bytes)) {
        return Ok(as_toml);
    }

    serde_json::from_slice::<T>(bytes).context("content is neither valid TOML nor JSON")
}

pub(super) fn validate_relative_path(path: &str) -> Result<PathBuf> {
    let candidate = PathBuf::from(path);

    if candidate.as_os_str().is_empty() {
        bail!("entry_component cannot be empty");
    }

    if candidate.is_absolute() {
        bail!("entry_component must be a relative path");
    }

    for component in candidate.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("entry_component cannot contain path traversal")
            }
            _ => {}
        }
    }

    Ok(candidate)
}

pub(super) fn validate_plugin_id(id: &str) -> Result<()> {
    validate_path_segment("manifest id", id, |ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')
    })
}

pub(super) fn validate_plugin_version(version: &str) -> Result<()> {
    validate_path_segment("manifest version", version, |ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+')
    })
}

fn validate_path_segment<F>(label: &str, value: &str, is_allowed_char: F) -> Result<()>
where
    F: Fn(char) -> bool,
{
    if value.is_empty() {
        bail!("{} cannot be empty", label);
    }
    if matches!(value, "." | "..") {
        bail!("{} cannot be '.' or '..'", label);
    }
    if value.contains('/') || value.contains('\\') {
        bail!("{} cannot contain path separators", label);
    }
    if !value.chars().all(is_allowed_char) {
        bail!("{} contains unsupported characters", label);
    }
    Ok(())
}

pub(super) fn validate_compatibility(compat: &PluginCompat) -> Result<()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).with_context(|| {
        format!(
            "failed to parse current krusty version '{}'",
            env!("CARGO_PKG_VERSION")
        )
    })?;

    let min = compat
        .krusty_min
        .as_deref()
        .map(|value| {
            Version::parse(value)
                .with_context(|| format!("invalid compat.krusty_min version '{}'", value))
        })
        .transpose()?;
    let max = compat
        .krusty_max
        .as_deref()
        .map(|value| {
            Version::parse(value)
                .with_context(|| format!("invalid compat.krusty_max version '{}'", value))
        })
        .transpose()?;

    if let (Some(min), Some(max)) = (&min, &max) {
        if min > max {
            bail!(
                "invalid compat range: compat.krusty_min ({}) is greater than compat.krusty_max ({})",
                min,
                max
            );
        }
    }

    if let Some(min) = min {
        if current < min {
            bail!(
                "plugin requires krusty >= {}, current version is {}",
                min,
                current
            );
        }
    }
    if let Some(max) = max {
        if current > max {
            bail!(
                "plugin requires krusty <= {}, current version is {}",
                max,
                current
            );
        }
    }

    Ok(())
}

pub(super) fn infer_source_name(manifest_url: &str) -> String {
    if let Ok(url) = Url::parse(manifest_url) {
        if let Some(host) = url.host_str() {
            return host.to_string();
        }
    }

    Path::new(manifest_url)
        .file_name()
        .and_then(OsStr::to_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "plugin-source".to_string())
}

pub(super) fn resolve_artifact_location(
    release_ref: &str,
    manifest_location: &ManifestLocation,
) -> Result<ArtifactLocation> {
    if let Ok(url) = Url::parse(release_ref) {
        return match url.scheme() {
            "http" | "https" => Ok(ArtifactLocation::Remote(url)),
            "file" => Ok(ArtifactLocation::Local(
                url.to_file_path()
                    .map_err(|_| anyhow!("invalid file URL: {}", url))?,
            )),
            other => bail!("unsupported release URL scheme: {}", other),
        };
    }

    match manifest_location {
        ManifestLocation::Local(manifest_path) => {
            let parent = manifest_path
                .parent()
                .ok_or_else(|| anyhow!("manifest path has no parent directory"))?;
            let release_path = validate_relative_path(release_ref).with_context(|| {
                format!(
                    "invalid local release path '{}' (must be relative and traversal-safe)",
                    release_ref
                )
            })?;
            Ok(ArtifactLocation::Local(parent.join(release_path)))
        }
        ManifestLocation::Remote(manifest_url) => bail!(
            "relative release path '{}' is not allowed for remote manifest {}",
            release_ref,
            manifest_url
        ),
    }
}
