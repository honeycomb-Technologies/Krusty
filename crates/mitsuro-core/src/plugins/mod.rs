//! Installable TUI plugin management.
//!
//! This module manages trusted plugin distribution metadata, installation,
//! lockfile pinning, and filesystem layout under `~/.mitsuro/plugins`.

mod manager;
mod signing;
mod types;

pub use manager::PluginManager;
pub use signing::plugin_release_signing_payload;
pub use types::{
    InstalledPlugin, PluginCatalogEntry, PluginCatalogFile, PluginCompat, PluginInstallOptions,
    PluginLockEntry, PluginLockfile, PluginManifestV1, PluginPermission, PluginPermissionGrant,
    PluginPermissionSet, PluginPermissionStatus, PluginPermissionsFile, PluginReconcileReport,
    PluginRelease, PluginReleaseArtifactKind, PluginRenderCapability, PluginRuntime, PluginSource,
    PluginSourceTrust, PluginSourcesFile, PluginTrustPolicy, PluginUpdateRecord,
    PluginUpdateReport,
};
