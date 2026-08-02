//! Extension manifest types
//!
//! Ported from Zed's crates/extension/src/extension_manifest.rs

use anyhow::{bail, Context as _, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The schema version of the [`ExtensionManifest`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct SchemaVersion(pub i32);

impl Default for SchemaVersion {
    fn default() -> Self {
        Self::ZERO
    }
}

impl SchemaVersion {
    pub const ZERO: Self = Self(0);
}

/// Extension manifest (extension.toml)
/// Compatible with Zed's extension manifest format
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub schema_version: SchemaVersion,

    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub lib: LibManifestEntry,

    // Asset paths
    #[serde(default)]
    pub themes: Vec<PathBuf>,
    #[serde(default)]
    pub icon_themes: Vec<PathBuf>,
    #[serde(default)]
    pub languages: Vec<PathBuf>,
    #[serde(default)]
    pub snippets: Option<PathBuf>,

    // Capabilities (e.g., "lsp", "completions")
    #[serde(default)]
    pub capabilities: Vec<String>,

    // Component registrations
    #[serde(default)]
    pub grammars: BTreeMap<String, GrammarManifestEntry>,
    #[serde(default)]
    pub language_servers: BTreeMap<String, LanguageServerManifestEntry>,
    #[serde(default)]
    pub context_servers: BTreeMap<String, ContextServerManifestEntry>,
    #[serde(default)]
    pub slash_commands: BTreeMap<String, SlashCommandManifestEntry>,
    #[serde(default)]
    pub indexed_docs_providers: BTreeMap<String, IndexedDocsProviderManifestEntry>,
    #[serde(default)]
    pub debug_adapters: BTreeMap<String, DebugAdapterManifestEntry>,
    #[serde(default)]
    pub debug_locators: BTreeMap<String, DebugLocatorManifestEntry>,
    #[serde(default)]
    pub agent_servers: BTreeMap<String, AgentServerManifestEntry>,
}

impl Default for ExtensionManifest {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: "0.0.0".to_string(),
            schema_version: SchemaVersion::ZERO,
            description: None,
            repository: None,
            authors: Vec::new(),
            lib: LibManifestEntry::default(),
            themes: Vec::new(),
            icon_themes: Vec::new(),
            languages: Vec::new(),
            snippets: None,
            capabilities: Vec::new(),
            grammars: BTreeMap::new(),
            language_servers: BTreeMap::new(),
            context_servers: BTreeMap::new(),
            slash_commands: BTreeMap::new(),
            indexed_docs_providers: BTreeMap::new(),
            debug_adapters: BTreeMap::new(),
            debug_locators: BTreeMap::new(),
            agent_servers: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LibManifestEntry {
    pub kind: Option<ExtensionLibraryKind>,
    pub version: Option<Version>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub enum ExtensionLibraryKind {
    /// Rust WASM extension (standard Zed format uses "Rust")
    Rust,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GrammarManifestEntry {
    pub repository: String,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LanguageServerManifestEntry {
    /// Single language (Zed uses this in newer extensions)
    #[serde(default)]
    pub language: Option<String>,
    /// Multiple languages (older format, still supported)
    #[serde(default)]
    pub languages: Vec<String>,
    /// Language ID mappings (e.g., {"Zig" = "zig"})
    #[serde(default)]
    pub language_ids: BTreeMap<String, String>,
    /// Supported code action kinds
    #[serde(default)]
    pub code_action_kinds: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ContextServerManifestEntry {
    // Context server configuration
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct SlashCommandManifestEntry {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub requires_argument: bool,
}

/// Indexed documentation provider entry
#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct IndexedDocsProviderManifestEntry {
    // Provider configuration - currently empty in most extensions
}

/// Debug adapter entry (DAP support)
#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct DebugAdapterManifestEntry {
    #[serde(default)]
    pub languages: Vec<String>,
}

/// Debug locator entry (finds debug configurations)
#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct DebugLocatorManifestEntry {
    #[serde(default)]
    pub languages: Vec<String>,
}

/// Agent server entry (AI agent integration)
#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct AgentServerManifestEntry {
    // Agent server configuration - currently empty in most extensions
}

/// Parse the zed:api-version from WASM bytes
pub fn parse_wasm_extension_version(extension_id: &str, wasm_bytes: &[u8]) -> Result<Version> {
    let mut version = None;

    for part in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        if let wasmparser::Payload::CustomSection(s) =
            part.context("error parsing wasm extension")?
        {
            if s.name() == "zed:api-version" {
                version = parse_wasm_extension_version_custom_section(s.data());
                if version.is_none() {
                    bail!(
                        "extension {} has invalid zed:api-version section: {:?}",
                        extension_id,
                        s.data()
                    );
                }
            }
        }
    }

    version.with_context(|| format!("extension {extension_id} has no zed:api-version section"))
}

fn parse_wasm_extension_version_custom_section(data: &[u8]) -> Option<Version> {
    if data.len() == 6 {
        Some(Version::new(
            u16::from_be_bytes([data[0], data[1]]) as _,
            u16::from_be_bytes([data[2], data[3]]) as _,
            u16::from_be_bytes([data[4], data[5]]) as _,
        ))
    } else {
        None
    }
}
