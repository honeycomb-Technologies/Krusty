use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_ID_BYTES: usize = 64;
const MAX_NAME_BYTES: usize = 256;
const MAX_VERSION_BYTES: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 16 * 1024;
const MAX_ENTRY_BYTES: usize = 4 * 1024;
const MAX_ENV_NAMES: usize = 128;
const MAX_ENV_NAME_BYTES: usize = 128;

fn default_manifest_version() -> u32 {
    1
}

fn default_version() -> String {
    "0.0.0".to_string()
}

fn default_true() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    30_000
}

/// Capabilities requested by executable extension code.
///
/// Agent extensions are trusted local code, just like shell hooks in the other
/// coding agents. These declarations are still useful for package review,
/// environment filtering, and install-time grants; they are not represented as
/// a JavaScript sandbox boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentExtensionPermissions {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub network: bool,
    pub process: bool,
    /// Environment variables copied into the otherwise-cleared worker process.
    pub env: Vec<String>,
}

/// Manifest for a JavaScript/TypeScript agent extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentExtensionManifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub entry: PathBuf,
    pub enabled: bool,
    pub timeout_ms: u64,
    pub permissions: AgentExtensionPermissions,
}

impl Default for AgentExtensionManifest {
    fn default() -> Self {
        Self {
            manifest_version: default_manifest_version(),
            id: String::new(),
            name: String::new(),
            version: default_version(),
            description: None,
            entry: PathBuf::from("index.ts"),
            enabled: default_true(),
            timeout_ms: default_timeout_ms(),
            permissions: AgentExtensionPermissions::default(),
        }
    }
}

impl AgentExtensionManifest {
    pub(crate) async fn from_json_file(path: &Path) -> Result<Self> {
        let metadata = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if metadata.len() > MAX_MANIFEST_BYTES as u64 {
            bail!(
                "agent extension manifest '{}' exceeds {} bytes",
                path.display(),
                MAX_MANIFEST_BYTES
            );
        }

        let file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("failed to open {}", path.display()))?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take((MAX_MANIFEST_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            bail!(
                "agent extension manifest '{}' exceeds {} bytes",
                path.display(),
                MAX_MANIFEST_BYTES
            );
        }

        serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn from_entry(entry: &Path) -> Result<Self> {
        let stem = entry
            .file_stem()
            .and_then(|value| value.to_str())
            .context("extension entry must have a UTF-8 file name")?;
        let id = normalize_id(stem);
        if id.is_empty() {
            bail!("extension entry '{}' has no usable id", entry.display());
        }

        let manifest = Self {
            id,
            name: stem.to_string(),
            entry: entry
                .file_name()
                .map(PathBuf::from)
                .context("extension entry is missing a file name")?,
            ..Self::default()
        };
        manifest.validate_declared_fields()?;
        Ok(manifest)
    }

    pub fn validate_and_resolve(&self, extension_dir: &Path) -> Result<PathBuf> {
        self.validate_declared_fields()?;

        let root = extension_dir.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize extension directory {}",
                extension_dir.display()
            )
        })?;
        let entry = extension_dir
            .join(&self.entry)
            .canonicalize()
            .with_context(|| {
                format!(
                    "agent extension '{}' entry does not exist: {}",
                    self.id,
                    extension_dir.join(&self.entry).display()
                )
            })?;
        if !entry.starts_with(&root) {
            bail!(
                "agent extension '{}' entry escapes its extension directory",
                self.id
            );
        }
        if !entry.is_file() {
            bail!("agent extension '{}' entry is not a file", self.id);
        }
        match entry.extension().and_then(|value| value.to_str()) {
            Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts") => {}
            _ => bail!(
                "agent extension '{}' entry must be JavaScript or TypeScript",
                self.id
            ),
        }

        Ok(entry)
    }

    pub(crate) fn validate_id(&self) -> Result<()> {
        validate_extension_id(&self.id)
    }

    fn validate_declared_fields(&self) -> Result<()> {
        if self.manifest_version != 1 {
            bail!(
                "unsupported agent extension manifest_version {} for '{}'",
                self.manifest_version,
                self.id
            );
        }
        self.validate_id()?;
        if self.name.trim().is_empty() {
            bail!("agent extension '{}' has an empty name", self.id);
        }
        if self.name.len() > MAX_NAME_BYTES {
            bail!(
                "agent extension '{}' name exceeds {} bytes",
                self.id,
                MAX_NAME_BYTES
            );
        }
        if self.name.chars().any(char::is_control) {
            bail!(
                "agent extension '{}' name contains control characters",
                self.id
            );
        }
        if self.version.trim().is_empty() || self.version.len() > MAX_VERSION_BYTES {
            bail!(
                "agent extension '{}' version must contain 1 to {} bytes",
                self.id,
                MAX_VERSION_BYTES
            );
        }
        if self
            .description
            .as_ref()
            .is_some_and(|description| description.len() > MAX_DESCRIPTION_BYTES)
        {
            bail!(
                "agent extension '{}' description exceeds {} bytes",
                self.id,
                MAX_DESCRIPTION_BYTES
            );
        }
        if self.entry.as_os_str().is_empty() || self.entry.is_absolute() {
            bail!(
                "agent extension '{}' entry must be a relative path",
                self.id
            );
        }
        let entry_text = self
            .entry
            .to_str()
            .with_context(|| format!("agent extension '{}' entry must be valid UTF-8", self.id))?;
        if entry_text.len() > MAX_ENTRY_BYTES {
            bail!(
                "agent extension '{}' entry exceeds {} bytes",
                self.id,
                MAX_ENTRY_BYTES
            );
        }
        if self.timeout_ms == 0 || self.timeout_ms > 600_000 {
            bail!(
                "agent extension '{}' timeout_ms must be between 1 and 600000",
                self.id
            );
        }
        if self.permissions.env.len() > MAX_ENV_NAMES {
            bail!(
                "agent extension '{}' requests more than {} environment variables",
                self.id,
                MAX_ENV_NAMES
            );
        }
        let mut env_names = BTreeSet::new();
        for name in &self.permissions.env {
            if !is_safe_env_name(name) {
                bail!(
                    "agent extension '{}' environment variable names must use 1 to {} uppercase ASCII letters, digits, or '_'",
                    self.id,
                    MAX_ENV_NAME_BYTES
                );
            }
            if !env_names.insert(name) {
                bail!(
                    "agent extension '{}' requests environment variable '{}' more than once",
                    self.id,
                    name
                );
            }
        }

        Ok(())
    }
}

fn validate_extension_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("agent extension id cannot be empty");
    }
    if id.len() > MAX_ID_BYTES {
        bail!("agent extension id exceeds {} bytes", MAX_ID_BYTES);
    }
    if id == "." || id == ".." {
        bail!("agent extension id '{id}' is not a safe path component");
    }
    if normalize_id(id) != id || id.ends_with('.') {
        bail!(
            "agent extension id '{id}' must be one lowercase portable path component using only letters, numbers, '-', '_', or '.'"
        );
    }

    let mut components = Path::new(id).components();
    if !matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(id)
    ) || components.next().is_some()
    {
        bail!("agent extension id '{id}' is not a safe path component");
    }

    // Windows reserves these names even when an extension is present. Reject
    // them everywhere so a package has the same identity on every platform.
    let base_name = id.split('.').next().unwrap_or(id);
    let numbered_device = base_name
        .strip_prefix("com")
        .or_else(|| base_name.strip_prefix("lpt"))
        .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'));
    if matches!(base_name, "con" | "prn" | "aux" | "nul") || numbered_device {
        bail!("agent extension id '{id}' is reserved as a platform path component");
    }

    Ok(())
}

pub(super) fn is_safe_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ENV_NAME_BYTES
        && name.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

pub(crate) fn normalize_id(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                Some(character)
            } else if character.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn manifest_entry_must_stay_inside_extension_directory() {
        let temp = TempDir::new().expect("temp dir");
        let extension = temp.path().join("extension");
        fs::create_dir_all(&extension).expect("extension directory");
        fs::write(temp.path().join("outside.ts"), "export default () => {}")
            .expect("outside entry");

        let manifest = AgentExtensionManifest {
            id: "escape".to_string(),
            name: "Escape".to_string(),
            entry: PathBuf::from("../outside.ts"),
            ..AgentExtensionManifest::default()
        };

        assert!(manifest.validate_and_resolve(&extension).is_err());
    }

    #[test]
    fn standalone_entry_gets_a_stable_manifest() {
        let manifest = AgentExtensionManifest::from_entry(Path::new("release-notes.ts"))
            .expect("standalone manifest");
        assert_eq!(manifest.id, "release-notes");
        assert_eq!(manifest.entry, PathBuf::from("release-notes.ts"));
    }

    #[test]
    fn manifest_id_is_one_portable_path_component() {
        for id in [
            ".",
            "..",
            "../escape",
            "escape/path",
            r"escape\path",
            "Uppercase",
            "trailing.",
            "con",
            "lpt1.logs",
        ] {
            let manifest = AgentExtensionManifest {
                id: id.to_string(),
                name: "Unsafe".to_string(),
                ..AgentExtensionManifest::default()
            };
            assert!(
                manifest.validate_id().is_err(),
                "unsafe extension id was accepted: {id:?}"
            );
        }

        let manifest = AgentExtensionManifest {
            id: "acme.tools-v2_1".to_string(),
            name: "Portable".to_string(),
            ..AgentExtensionManifest::default()
        };
        manifest.validate_id().expect("portable extension id");
    }

    #[test]
    fn manifest_fields_are_bounded_and_environment_names_are_validated() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(temp.path().join("index.ts"), "export default () => {}").expect("manifest entry");

        let mut manifest = AgentExtensionManifest {
            id: "bounded".to_string(),
            name: "Bounded".to_string(),
            ..AgentExtensionManifest::default()
        };
        manifest.description = Some("x".repeat(MAX_DESCRIPTION_BYTES + 1));
        assert!(manifest.validate_and_resolve(temp.path()).is_err());

        manifest.description = None;
        manifest.permissions.env = vec!["lowercase".to_string()];
        assert!(manifest.validate_and_resolve(temp.path()).is_err());

        manifest.permissions.env = vec!["GITHUB_TOKEN".to_string(); 2];
        assert!(manifest.validate_and_resolve(temp.path()).is_err());
    }

    #[tokio::test]
    async fn manifest_file_read_is_bounded() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("krusty-extension.json");
        fs::write(&path, vec![b' '; MAX_MANIFEST_BYTES + 1]).expect("oversized manifest");

        let error = AgentExtensionManifest::from_json_file(&path)
            .await
            .expect_err("oversized manifest must be rejected");
        assert!(error.to_string().contains("exceeds"));
    }
}
