use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

fn default_manifest_version() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginRenderCapability {
    Text,
    Frame,
}

fn default_plugin_runtime() -> PluginRuntime {
    PluginRuntime::Wasm
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginRuntime {
    /// Native dynamic library plugin using Krusty's C ABI.
    Native,
    /// Installable TUI WebAssembly descriptor. This remains the manifest
    /// default, but package entry components are not executed by this runtime
    /// today. Executable Wasmtime isolation belongs to Krusty's separate
    /// Zed-compatible editor/language ABI.
    Wasm,
    /// JavaScript/TypeScript TUI package runtime through the libnode/edon bridge.
    Js,
}

impl PluginRuntime {
    /// Native libraries and JS/TS entry components execute with the current
    /// user's OS authority. Installable WASM TUI entries are descriptor-only
    /// today, so accepting one does not require process permission. Krusty's
    /// separate executable Wasmtime editor/language ABI is not a drop-in TUI
    /// or agent runtime.
    pub fn requires_process_permission(self) -> bool {
        matches!(self, Self::Native | Self::Js)
    }
}

/// Auditable permissions for host-mediated plugin capabilities.
///
/// `process` is also the explicit trust decision for native, JS/TS, and shell
/// components. Once granted, that code has the current user's OS authority;
/// the other bits do not form a kernel sandbox around it. Installable WASM TUI
/// entries are currently descriptor-only; executable Wasmtime isolation is a
/// separate Zed-compatible editor/language ABI, not a drop-in package runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginPermissionSet {
    #[serde(default)]
    pub fs_read: bool,
    #[serde(default)]
    pub fs_write: bool,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub process: bool,
}

impl PluginPermissionSet {
    pub fn allows(&self, permission: PluginPermission) -> bool {
        match permission {
            PluginPermission::FsRead => self.fs_read,
            PluginPermission::FsWrite => self.fs_write,
            PluginPermission::Network => self.network,
            PluginPermission::Process => self.process,
        }
    }

    pub fn is_subset_of(&self, requested: &Self) -> bool {
        (!self.fs_read || requested.fs_read)
            && (!self.fs_write || requested.fs_write)
            && (!self.network || requested.network)
            && (!self.process || requested.process)
    }

    pub fn is_empty(&self) -> bool {
        !self.fs_read && !self.fs_write && !self.network && !self.process
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPermission {
    FsRead,
    FsWrite,
    Network,
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginCompat {
    #[serde(default)]
    pub krusty_min: Option<String>,
    #[serde(default)]
    pub krusty_max: Option<String>,
}

fn default_release_artifact_kind() -> PluginReleaseArtifactKind {
    PluginReleaseArtifactKind::SingleComponent
}

/// On-wire shape of a signed release artifact.
///
/// The single-component default preserves the original signed-manifest format.
/// A zip bundle is an authenticated package snapshot whose paths are extracted
/// beneath the manager-owned transaction root before component validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PluginReleaseArtifactKind {
    #[default]
    SingleComponent,
    ZipBundle,
}

impl PluginReleaseArtifactKind {
    fn is_single_component(&self) -> bool {
        matches!(self, Self::SingleComponent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRelease {
    /// Artifact location. This and every other immutable manifest field are
    /// authenticated by the release-envelope signature.
    pub url: String,
    pub sha256: String,
    pub signature: String,
    pub signing_key_id: String,
    /// Artifact container. Omitted values retain the legacy single-component
    /// signing representation, so existing release envelopes remain valid.
    #[serde(
        default = "default_release_artifact_kind",
        skip_serializing_if = "PluginReleaseArtifactKind::is_single_component"
    )]
    pub artifact_kind: PluginReleaseArtifactKind,
    /// Explicit signature protocol. Missing values identify legacy manifests
    /// whose artifact-only signatures cannot be safely interpreted as release
    /// envelopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_scheme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestV1 {
    #[serde(default = "default_manifest_version")]
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_plugin_runtime")]
    pub runtime: PluginRuntime,
    /// Optional TUI/runtime entry point. Bundle-only packages can omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_component: Option<String>,
    /// Skill files or directories contributed by this bundle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Executable agent-extension entry points contributed by this bundle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_extensions: Vec<String>,
    /// MCP server configuration file contributed by this bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<String>,
    /// Declarative JSON/TOML command-hook configuration files. Executable
    /// JavaScript/TypeScript belongs in `agent_extensions`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<String>,
    /// Static asset root contributed by this bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    #[serde(default)]
    pub render_capabilities: Vec<PluginRenderCapability>,
    #[serde(default)]
    pub requested_permissions: PluginPermissionSet,
    #[serde(default)]
    pub release: Option<PluginRelease>,
    #[serde(default)]
    pub compat: PluginCompat,
}

impl PluginManifestV1 {
    pub fn normalized_render_capabilities(&self) -> Vec<PluginRenderCapability> {
        if self.render_capabilities.is_empty() {
            vec![PluginRenderCapability::Text]
        } else {
            self.render_capabilities.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_plugin_runtime")]
    pub runtime: PluginRuntime,
    pub install_path: PathBuf,
    pub manifest_path: PathBuf,
    pub entry_component_path: Option<PathBuf>,
    #[serde(default)]
    pub skill_paths: Vec<PathBuf>,
    #[serde(default)]
    pub agent_extension_paths: Vec<PathBuf>,
    #[serde(default)]
    pub mcp_servers_path: Option<PathBuf>,
    #[serde(default)]
    pub hook_paths: Vec<PathBuf>,
    #[serde(default)]
    pub assets_path: Option<PathBuf>,
    pub enabled: bool,
    pub pinned: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_trust: PluginSourceTrust,
    #[serde(default)]
    pub package_scripts_allowed: bool,
    #[serde(default)]
    pub requested_permissions: PluginPermissionSet,
    #[serde(default)]
    pub render_capabilities: Vec<PluginRenderCapability>,
}

impl InstalledPlugin {
    pub fn has_agent_components(&self) -> bool {
        !self.skill_paths.is_empty()
            || !self.agent_extension_paths.is_empty()
            || self.mcp_servers_path.is_some()
            || !self.hook_paths.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSourceTrust {
    /// Legacy lock entries did not persist their trust boundary. Runtime callers
    /// should treat this as unsigned until the plugin is reinstalled.
    #[default]
    LegacyUnknown,
    /// Artifact digest and Ed25519 release-envelope signature were verified
    /// against an allowlisted publisher during installation. This does not
    /// assert that the current bytes were revalidated during activation.
    SignedPublisher,
    /// Installed from npm. Registry transport is trusted, package code is not signed by Krusty.
    NpmUnsigned,
    /// Installed from a local directory explicitly selected by the user.
    LocalUnsigned,
}

impl PluginSourceTrust {
    /// Whether the currently stored bytes have been cryptographically
    /// revalidated for this activation. The persisted source classification is
    /// installation-time provenance only, so it cannot establish this alone.
    pub fn is_cryptographically_verified(self) -> bool {
        let _ = self;
        false
    }

    pub fn was_verified_at_install(self) -> bool {
        matches!(self, Self::SignedPublisher)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PluginInstallOptions {
    /// Allows npm lifecycle scripts and a declared npm `build` script. This is
    /// intentionally false by default because both execute arbitrary code.
    pub allow_package_scripts: bool,
    /// Overrides source-derived pinning. `None` pins signed/local sources and
    /// pins npm sources only when their package spec contains an explicit version.
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSource {
    pub name: String,
    /// URL or local path to a Krusty plugin catalog file.
    ///
    /// Kept as `manifest_url` for compatibility with the original signed-manifest
    /// source list; new code treats it as a catalog URL/path.
    pub manifest_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub package: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_plugin_runtime")]
    pub runtime: PluginRuntime,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub official: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalogFile {
    #[serde(default = "default_manifest_version")]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<PluginCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSourcesFile {
    #[serde(default = "default_manifest_version")]
    pub version: u32,
    #[serde(default)]
    pub sources: Vec<PluginSource>,
}

impl Default for PluginSourcesFile {
    fn default() -> Self {
        Self {
            version: default_manifest_version(),
            sources: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLockEntry {
    pub id: String,
    pub version: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub pinned: bool,
    /// Optional package/source root for npm or local package installs.
    #[serde(default)]
    pub package_path: Option<PathBuf>,
    /// Manifest path relative to package_path for package installs.
    #[serde(default)]
    pub manifest_path: Option<PathBuf>,
    /// Original source spec such as `npm:@scope/pkg` or a local path.
    #[serde(default)]
    pub source: Option<String>,
    /// Manager-owned transaction root. Only this path may be recursively removed.
    #[serde(default)]
    pub managed_root: Option<PathBuf>,
    #[serde(default)]
    pub source_trust: PluginSourceTrust,
    #[serde(default)]
    pub package_scripts_allowed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLockfile {
    #[serde(default = "default_manifest_version")]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
}

impl Default for PluginLockfile {
    fn default() -> Self {
        Self {
            version: default_manifest_version(),
            plugins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginTrustPolicy {
    #[serde(default)]
    pub allowed_publishers: Vec<String>,
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
    /// Publisher-to-key bindings prevent an allowlisted publisher from naming
    /// some other trusted publisher's signing key.
    #[serde(default)]
    pub publisher_keys: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginPermissionGrant {
    /// Version at the time of the decision, retained for audit display.
    #[serde(default)]
    pub plugin_version: Option<String>,
    /// Publisher and source are part of the reviewed identity. Replacing an ID
    /// from a different source cannot inherit the previous plugin's grants.
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// Exact requested set the user reviewed. A changed request invalidates the grant.
    #[serde(default)]
    pub requested: Option<PluginPermissionSet>,
    #[serde(default)]
    pub granted: PluginPermissionSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPermissionsFile {
    #[serde(default = "default_manifest_version")]
    pub version: u32,
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginPermissionGrant>,
}

impl Default for PluginPermissionsFile {
    fn default() -> Self {
        Self {
            version: default_manifest_version(),
            plugins: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPermissionStatus {
    pub plugin_id: String,
    pub plugin_version: String,
    pub publisher: String,
    pub source: Option<String>,
    pub requested: PluginPermissionSet,
    pub granted: PluginPermissionSet,
    pub grant_is_current: bool,
}

impl PluginPermissionStatus {
    pub fn allows(&self, permission: PluginPermission) -> bool {
        self.grant_is_current
            && self.requested.allows(permission)
            && self.granted.allows(permission)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginUpdateRecord {
    pub id: String,
    pub previous_version: String,
    pub current_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginUpdateReport {
    pub updated: Vec<PluginUpdateRecord>,
    pub unchanged: Vec<String>,
    pub removed: Vec<String>,
    pub skipped_pinned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginReconcileReport {
    pub valid_plugins: Vec<String>,
    pub invalid_plugins: Vec<(String, String)>,
    pub removed_orphan_roots: Vec<PathBuf>,
    pub updates: PluginUpdateReport,
}

#[cfg(test)]
mod tests {
    use super::PluginSourceTrust;

    #[test]
    fn install_time_provenance_does_not_claim_activation_time_verification() {
        assert!(PluginSourceTrust::SignedPublisher.was_verified_at_install());
        assert!(!PluginSourceTrust::SignedPublisher.is_cryptographically_verified());
        assert!(!PluginSourceTrust::NpmUnsigned.is_cryptographically_verified());
        assert!(!PluginSourceTrust::LocalUnsigned.is_cryptographically_verified());
        assert!(!PluginSourceTrust::LegacyUnknown.is_cryptographically_verified());
    }
}
