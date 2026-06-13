use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio::{fs, process::Command};

use super::storage::{read_installed_from_lock_entry, upsert_lock_entry_record};
use super::validation::{parse_toml_or_json, validate_relative_path, MAX_MANIFEST_BYTES};
use super::PluginManager;
use crate::plugins::{InstalledPlugin, PluginLockEntry, PluginManifestV1};

#[derive(Debug, Deserialize)]
struct PackageJson {
    #[allow(dead_code)]
    name: Option<String>,
    #[allow(dead_code)]
    version: Option<String>,
    #[serde(default)]
    krusty: Option<KrustyPackageManifest>,
    #[serde(default)]
    scripts: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct KrustyPackageManifest {
    #[serde(default)]
    plugins: Vec<String>,
}

impl PluginManager {
    /// Install either a signed standalone plugin manifest, an npm package (`npm:<spec>`),
    /// or a local package directory containing `package.json` with `krusty.plugins`.
    pub async fn install_from_ref(&self, plugin_ref: &str) -> Result<Vec<InstalledPlugin>> {
        if plugin_ref.trim().starts_with("npm:") {
            return self.install_from_package_ref(plugin_ref).await;
        }

        let path = PathBuf::from(plugin_ref);
        if path.is_dir() {
            return self.install_from_package_ref(plugin_ref).await;
        }

        Ok(vec![self.install_from_manifest_ref(plugin_ref).await?])
    }

    /// Install plugins declared by an npm or local package.
    pub async fn install_from_package_ref(
        &self,
        package_ref: &str,
    ) -> Result<Vec<InstalledPlugin>> {
        let (package_root, source) = if let Some(npm_spec) = package_ref.trim().strip_prefix("npm:")
        {
            let spec = npm_spec.trim();
            if spec.is_empty() {
                bail!("npm package spec cannot be empty");
            }
            let package_root = self.install_npm_package(spec).await?;
            (package_root, format!("npm:{spec}"))
        } else {
            let package_root = PathBuf::from(package_ref);
            if !package_root.is_dir() {
                bail!(
                    "plugin package path is not a directory: {}",
                    package_root.display()
                );
            }
            let package_root = fs::canonicalize(&package_root)
                .await
                .with_context(|| format!("failed to canonicalize {}", package_root.display()))?;
            (package_root, package_ref.to_string())
        };

        let manifest_paths = self
            .discover_package_plugin_manifests(&package_root)
            .await?;
        if manifest_paths.is_empty() {
            bail!(
                "package {} does not declare any Krusty plugins (expected package.json krusty.plugins or plugin.toml)",
                package_root.display()
            );
        }

        let mut installed = Vec::new();
        let mut build_attempted = false;
        for manifest_rel in manifest_paths {
            let manifest_path = package_root.join(&manifest_rel);
            let bytes = fs::read(&manifest_path)
                .await
                .with_context(|| format!("failed to read {}", manifest_path.display()))?;
            if bytes.len() > MAX_MANIFEST_BYTES {
                bail!(
                    "plugin manifest exceeds size limit: {}",
                    manifest_path.display()
                );
            }
            let manifest: PluginManifestV1 = parse_toml_or_json(&bytes)
                .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

            self.validate_manifest(&manifest, false)?;
            let entry_rel = validate_relative_path(&manifest.entry_component)?;
            let entry_path = package_root.join(&entry_rel);
            if !entry_path.exists() && !build_attempted {
                build_attempted = self.try_build_package(&package_root).await?;
            }
            if !entry_path.exists() {
                let build_hint = if build_attempted {
                    "build script ran but did not produce the entry_component"
                } else {
                    "run the package build first (for the example: cd examples/plugins/native-rust && npm run build)"
                };
                bail!(
                    "plugin '{}' entry_component does not exist: {} ({})",
                    manifest.id,
                    entry_path.display(),
                    build_hint
                );
            }

            let entry = PluginLockEntry {
                id: manifest.id.clone(),
                version: manifest.version.clone(),
                enabled: true,
                pinned: !source.starts_with("npm:")
                    || npm_spec_is_pinned(source.trim_start_matches("npm:")),
                package_path: Some(package_root.clone()),
                manifest_path: Some(manifest_rel.clone()),
                source: Some(source.clone()),
            };
            upsert_lock_entry_record(self, entry.clone()).await?;
            installed.push(read_installed_from_lock_entry(self, &entry).await?);
        }

        Ok(installed)
    }

    async fn try_build_package(&self, package_root: &Path) -> Result<bool> {
        let package_json = match self.read_package_json(package_root).await? {
            Some(package_json) => package_json,
            None => return Ok(false),
        };

        if !package_json.scripts.contains_key("build") {
            return Ok(false);
        }

        let output = Command::new("npm")
            .args(["run", "build"])
            .current_dir(package_root)
            .output()
            .await
            .with_context(|| {
                format!(
                    "failed to execute npm build script in {}; install npm or build the plugin manually",
                    package_root.display()
                )
            })?;

        if !output.status.success() {
            bail!(
                "npm run build failed for plugin package {}: {}",
                package_root.display(),
                format_command_output(&output)
            );
        }

        Ok(true)
    }

    async fn install_npm_package(&self, spec: &str) -> Result<PathBuf> {
        let install_root = self.npm_install_root();
        self.ensure_npm_project(&install_root).await?;

        let output = Command::new("npm")
            .args(["install", spec, "--prefix"])
            .arg(&install_root)
            .output()
            .await
            .with_context(|| "failed to execute npm; install npm or use a local plugin package")?;

        if !output.status.success() {
            bail!(
                "npm install failed for package spec '{}': {}",
                spec,
                format_command_output(&output)
            );
        }

        let package_name = npm_package_name(spec);
        let package_path = install_root.join("node_modules").join(package_name);
        if !package_path.exists() {
            bail!(
                "npm install completed but package directory was not found: {}",
                package_path.display()
            );
        }
        Ok(package_path)
    }

    async fn ensure_npm_project(&self, install_root: &Path) -> Result<()> {
        fs::create_dir_all(install_root)
            .await
            .with_context(|| format!("failed to create {}", install_root.display()))?;
        let package_json = install_root.join("package.json");
        if !package_json.exists() {
            let content = serde_json::json!({
                "name": "krusty-plugins",
                "private": true
            });
            fs::write(&package_json, serde_json::to_vec_pretty(&content)?)
                .await
                .with_context(|| format!("failed to write {}", package_json.display()))?;
        }
        let gitignore = install_root.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "*\n!.gitignore\n")
                .await
                .with_context(|| format!("failed to write {}", gitignore.display()))?;
        }
        Ok(())
    }

    fn npm_install_root(&self) -> PathBuf {
        self.root().join("npm")
    }

    async fn discover_package_plugin_manifests(&self, package_root: &Path) -> Result<Vec<PathBuf>> {
        if let Some(package_json) = self.read_package_json(package_root).await? {
            if let Some(krusty) = package_json.krusty {
                let mut manifests = Vec::new();
                for plugin_path in krusty.plugins {
                    let rel = validate_relative_path(&plugin_path).with_context(|| {
                        format!("invalid krusty.plugins entry '{}'", plugin_path)
                    })?;
                    if package_root.join(&rel).exists() {
                        manifests.push(rel);
                    }
                }
                if !manifests.is_empty() {
                    return Ok(manifests);
                }
            }
        }

        let default_manifest = PathBuf::from("plugin.toml");
        if package_root.join(&default_manifest).exists() {
            return Ok(vec![default_manifest]);
        }

        Ok(Vec::new())
    }

    async fn read_package_json(&self, package_root: &Path) -> Result<Option<PackageJson>> {
        let package_json_path = package_root.join("package.json");
        if !package_json_path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&package_json_path)
            .await
            .with_context(|| format!("failed to read {}", package_json_path.display()))?;
        let package_json: PackageJson = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", package_json_path.display()))?;
        Ok(Some(package_json))
    }
}

fn format_command_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut message = format!("status {}", output.status);
    if !stdout.trim().is_empty() {
        message.push_str("; stdout: ");
        message.push_str(truncated_command_stream(stdout.trim()));
    }
    if !stderr.trim().is_empty() {
        message.push_str("; stderr: ");
        message.push_str(truncated_command_stream(stderr.trim()));
    }
    message
}

fn truncated_command_stream(stream: &str) -> &str {
    const MAX_CHARS: usize = 12_000;
    if stream.chars().count() <= MAX_CHARS {
        return stream;
    }

    let start = stream
        .char_indices()
        .nth(stream.chars().count().saturating_sub(MAX_CHARS))
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    &stream[start..]
}

fn npm_package_name(spec: &str) -> &str {
    if let Some(scoped) = spec.strip_prefix('@') {
        let mut parts = scoped.splitn(3, '@');
        let scope_and_name = parts.next().unwrap_or(scoped);
        return &spec[..scope_and_name.len() + 1];
    }

    spec.split('@').next().unwrap_or(spec)
}

fn npm_spec_is_pinned(spec: &str) -> bool {
    if let Some(scoped) = spec.strip_prefix('@') {
        scoped.split('@').nth(1).is_some()
    } else {
        spec.split_once('@').map(|x| x.1).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_npm_package_names() {
        assert_eq!(npm_package_name("left-pad"), "left-pad");
        assert_eq!(npm_package_name("left-pad@1.0.0"), "left-pad");
        assert_eq!(npm_package_name("@scope/plugin"), "@scope/plugin");
        assert_eq!(npm_package_name("@scope/plugin@1.0.0"), "@scope/plugin");
    }

    #[test]
    fn detects_pinned_npm_specs() {
        assert!(!npm_spec_is_pinned("left-pad"));
        assert!(npm_spec_is_pinned("left-pad@1.0.0"));
        assert!(!npm_spec_is_pinned("@scope/plugin"));
        assert!(npm_spec_is_pinned("@scope/plugin@1.0.0"));
    }
}
