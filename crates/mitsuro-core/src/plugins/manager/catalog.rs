use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::io::{read_local_bytes_with_limit, read_remote_bytes_with_limit};
use super::storage::load_sources;
use super::validation::{parse_toml_or_json, MAX_MANIFEST_BYTES};
use super::PluginManager;
use crate::plugins::{PluginCatalogEntry, PluginCatalogFile};

const BUILTIN_CATALOG: &[u8] = include_bytes!("../../../../../docs/extensions/catalog.json");

impl PluginManager {
    pub(super) fn validate_catalog_source_ref(&self, source_ref: &str) -> Result<()> {
        if let Ok(url) = url::Url::parse(source_ref) {
            return match url.scheme() {
                "https" | "file" => Ok(()),
                "http" => bail!(
                    "remote plugin catalogs must use HTTPS; use a local file path for development"
                ),
                scheme => bail!("unsupported catalog source scheme '{}'", scheme),
            };
        }

        let path = PathBuf::from(source_ref);
        if path.as_os_str().is_empty() {
            bail!("plugin catalog source cannot be empty");
        }
        Ok(())
    }

    /// Load plugin catalog entries from configured sources.
    ///
    /// Catalog files are static TOML/JSON documents with shape:
    /// `version = 1`, `plugins = [{ id, name, version, package, ... }]`.
    pub async fn list_catalog_plugins(&self) -> Result<Vec<PluginCatalogEntry>> {
        let sources = load_sources(self).await?;
        let mut entries = parse_builtin_catalog()?;

        for source in sources.sources {
            let bytes = if let Ok(url) = url::Url::parse(&source.manifest_url) {
                if matches!(url.scheme(), "http" | "https") {
                    read_remote_bytes_with_limit(self, &url, MAX_MANIFEST_BYTES, "plugin catalog")
                        .await
                        .with_context(|| {
                            format!("failed to fetch catalog source '{}'", source.name)
                        })?
                } else if url.scheme() == "file" {
                    let path = url
                        .to_file_path()
                        .map_err(|_| anyhow::anyhow!("invalid file URL: {}", url))?;
                    read_local_bytes_with_limit(&path, MAX_MANIFEST_BYTES, "plugin catalog")
                        .await
                        .with_context(|| {
                            format!("failed to read catalog source '{}'", source.name)
                        })?
                } else {
                    bail!(
                        "unsupported catalog source scheme '{}' for '{}'",
                        url.scheme(),
                        source.name
                    );
                }
            } else {
                let path = PathBuf::from(&source.manifest_url);
                read_local_bytes_with_limit(&path, MAX_MANIFEST_BYTES, "plugin catalog")
                    .await
                    .with_context(|| format!("failed to read catalog source '{}'", source.name))?
            };

            let catalog: PluginCatalogFile = parse_toml_or_json(&bytes)
                .with_context(|| format!("failed to parse catalog source '{}'", source.name))?;
            if catalog.version != 1 {
                bail!(
                    "unsupported catalog version '{}' from source '{}'",
                    catalog.version,
                    source.name
                );
            }
            // `official` is a property of the catalog trust root, not a claim
            // third-party catalog authors can make about themselves.
            entries.extend(catalog.plugins.into_iter().map(|mut entry| {
                entry.official = false;
                entry
            }));
        }

        entries.sort_by_key(|entry| entry.id.clone());
        entries.dedup_by(|a, b| a.id == b.id);
        entries.sort_by_key(|entry| (!entry.official, entry.name.to_lowercase(), entry.id.clone()));
        Ok(entries)
    }
}

fn parse_builtin_catalog() -> Result<Vec<PluginCatalogEntry>> {
    let catalog: PluginCatalogFile =
        parse_toml_or_json(BUILTIN_CATALOG).context("failed to parse built-in plugin catalog")?;
    if catalog.version != 1 {
        bail!("unsupported built-in catalog version '{}'", catalog.version);
    }
    Ok(catalog.plugins)
}
