//! Canonical Mitsuro identity and the bounded legacy compatibility bridge.
//!
//! New writes and public surfaces use the canonical constants in this module.
//! Old spellings are deliberately confined to [`legacy`] and are accepted
//! only for reads, verification, and an explicit offline migration.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

pub const CONFIG_DIR_NAME: &str = ".mitsuro";
pub const DATABASE_FILE_NAME: &str = "mitsuro.db";
pub const HIVE_DIR_NAME: &str = "hive";
pub const HIVE_SOCKET_FILE_NAME: &str = "hive.sock";
pub const HIVE_KEY_FILE_NAME: &str = "hive-ipc.key";
pub const AGENT_EXTENSION_MANIFEST_FILE_NAME: &str = "mitsuro-extension.json";
pub const ENV_PREFIX: &str = "MITSURO_";
pub const HIVE_ENV_PREFIX: &str = "MITSURO_HIVE_";
pub const EXTENSION_ID_ENV: &str = "MITSURO_EXTENSION_ID";
pub const EXTENSION_DIR_ENV: &str = "MITSURO_EXTENSION_DIR";
pub const EXTENSION_STATE_DIR_ENV: &str = "MITSURO_EXTENSION_STATE_DIR";
pub const WORKING_DIR_ENV: &str = "MITSURO_WORKING_DIR";
pub const MIGRATION_RECEIPT_FILE_NAME: &str = ".identity-migration-v2";
pub const MIGRATION_RECEIPT_MAX_BYTES: u64 = 16 * 1024;

const LEGACY_ONLY_MIGRATION_GUIDANCE: &str = "legacy Mitsuro state requires an offline migration: run `mitsuro migrate-identity --confirm-offline`; compatibility updater payloads without the canonical executable may run `krusty migrate-identity --confirm-offline`, then install the canonical `mitsuro` command";

/// Old identifiers accepted only at compatibility boundaries.
pub mod legacy {
    pub const CONFIG_DIR_NAME: &str = ".krusty";
    pub const DATABASE_FILE_NAME: &str = "krusty.db";
    pub const HIVE_DIR_NAME: &str = "mako";
    pub const HIVE_SOCKET_FILE_NAME: &str = "mako.sock";
    pub const HIVE_KEY_FILE_NAME: &str = "mako-ipc.key";
    pub const HIVE_SOUL_FILE_NAME: &str = "MAKO_SOUL.md";
    pub const HIVE_IDENTITY_FILE_NAME: &str = "MAKO_IDENTITY.md";
    pub const HIVE_HEARTBEAT_FILE_NAME: &str = "MAKO_HEARTBEAT.md";
    pub const HIVE_MEMORY_FILE_NAME: &str = "MAKO_MEMORY.md";
    pub const HIVE_CHANNELS_FILE_NAME: &str = "MAKO_CHANNELS.md";
    pub const HIVE_PROJECT_OVERLAY_FILE_NAME: &str = "HIVE.md";
    pub const HIVE_PROJECT_OVERLAY_FILE_NAME_LOWERCASE: &str = "hive.md";
    pub const AGENT_EXTENSION_MANIFEST_FILE_NAME: &str = "krusty-extension.json";
    pub const ENV_PREFIX: &str = "KRUSTY_";
    pub const HIVE_ENV_PREFIX: &str = "KRUSTY_MAKO_";
    pub const EXTENSION_ID_ENV: &str = "KRUSTY_EXTENSION_ID";
    pub const EXTENSION_DIR_ENV: &str = "KRUSTY_EXTENSION_DIR";
    pub const EXTENSION_STATE_DIR_ENV: &str = "KRUSTY_EXTENSION_STATE_DIR";
    pub const WORKING_DIR_ENV: &str = "KRUSTY_WORKING_DIR";
    pub const PACKAGE_PLUGIN_ID_ENV: &str = "KRUSTY_PLUGIN_ID";
    pub const PACKAGE_HOOK_CONFIG_ENV: &str = "KRUSTY_HOOK_CONFIG";
    pub const PACKAGE_PLUGIN_ROOT_ENV: &str = "KRUSTY_PLUGIN_ROOT";
    pub const SESSION_TYPE: &str = "mako";
    pub const MEMORY_NAMESPACE: &str = "mako";
    pub const PROJECT_SETTINGS_FIELD: &str = "mako";
    pub const API_PREFIX: &str = "/mako";
    pub const CLI_NAME: &str = "krusty";
    pub const HIVE_CLI_NAME: &str = "mako";
    pub const ACP_MODEL_KEY_PREFIX: &str = "krusty:model-key:";
    pub const COMPACTION_SEGMENT_SCHEMA: &str = "krusty.compaction_segment.v1";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDiscovery {
    Empty,
    CanonicalOnly,
    LegacyOnly,
    /// Both roots exist and the canonical root contains the offline migration
    /// receipt. The legacy root is retained as the rollback authority.
    MigratedWithRollback,
    /// Both roots exist without a receipt, so choosing one would be unsafe.
    UnreconciledCoexistence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigMigrationReceipt {
    pub canonical_root: PathBuf,
    pub rollback_root: PathBuf,
    pub receipt_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedMigrationReceipt {
    source: PathBuf,
    created_unix: u64,
    source_authority: SourceAuthorityFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceSqliteSnapshot {
    Absent,
    Sha256 {
        digest: String,
        source_stat: SqliteSourceStat,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SqliteSourceStat {
    main: SqliteFileStat,
    wal: Option<SqliteFileStat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SqliteFileStat {
    len: u64,
    modified_ns: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceAuthorityFingerprint {
    sqlite: SourceSqliteSnapshot,
    durable_tree: DurableTreeFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableTreeFingerprint {
    content_sha256: String,
    stat_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessQuiescencePolicy {
    OfflineCutover,
    PreservedLegacyAuthority,
}

/// Resolve a canonical environment variable with a legacy read fallback.
/// The canonical key always wins and old values are never written back.
pub fn env_var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => legacy_env_name(name)
            .map_or(Err(std::env::VarError::NotPresent), |legacy_name| {
                std::env::var(legacy_name)
            }),
        Err(error) => Err(error),
    }
}

pub fn env_var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).or_else(|| legacy_env_name(name).and_then(std::env::var_os))
}

/// Mirror inherited old variables into canonical process-local names for
/// child-process compatibility. Existing canonical values are never replaced.
pub fn import_legacy_environment() {
    let inherited: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (name, value) in inherited {
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(canonical) = canonical_env_name(name) else {
            continue;
        };
        if std::env::var_os(&canonical).is_none() {
            std::env::set_var(canonical, value);
        }
    }
}

fn state_root_exists(path: &Path, label: &str) -> io::Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "refusing symlinked {label} state root at {}",
                path.display()
            ),
        ));
    }
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} state root is not a directory at {}",
                path.display()
            ),
        ));
    }
    Ok(true)
}

fn path_exists_unfollowed(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn receipt_source_path(path: &Path) -> io::Result<String> {
    let source = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "legacy configuration path is not valid UTF-8 and cannot be represented losslessly in the v2 migration receipt",
        )
    })?;
    if source.contains(['\r', '\n']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "legacy configuration path cannot contain CR or LF",
        ));
    }
    Ok(source.to_string())
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(path)?;
    // A hostile or unusually restrictive ambient umask must not decide the
    // final authority's mode. Set it explicitly while the directory is empty.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(unix)]
fn secure_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn secure_private_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn discover_config_for_home(home: &Path) -> io::Result<ConfigDiscovery> {
    let canonical = home.join(CONFIG_DIR_NAME);
    let old = home.join(legacy::CONFIG_DIR_NAME);
    let canonical_exists = state_root_exists(&canonical, "canonical Mitsuro")?;
    let old_exists = state_root_exists(&old, "legacy Mitsuro")?;
    let receipt_present =
        canonical_exists && path_exists_unfollowed(&canonical.join(MIGRATION_RECEIPT_FILE_NAME))?;
    if receipt_present && !old_exists {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "v2 identity receipt requires the preserved rollback root at {}",
                old.display()
            ),
        ));
    }
    Ok(match (canonical_exists, old_exists) {
        (false, false) => ConfigDiscovery::Empty,
        (true, false) => ConfigDiscovery::CanonicalOnly,
        (false, true) => ConfigDiscovery::LegacyOnly,
        (true, true) if validated_migration_receipt(&canonical, &old).is_some() => {
            ConfigDiscovery::MigratedWithRollback
        }
        (true, true) => ConfigDiscovery::UnreconciledCoexistence,
    })
}

pub fn discover_config() -> io::Result<ConfigDiscovery> {
    match dirs::home_dir() {
        Some(home) => discover_config_for_home(&home),
        None => Ok(ConfigDiscovery::Empty),
    }
}

/// Read-only startup gate. It never moves, copies, deletes, or rewrites state.
pub fn require_startup_identity() -> io::Result<ConfigDiscovery> {
    let Some(home) = dirs::home_dir() else {
        return Ok(ConfigDiscovery::Empty);
    };
    require_startup_identity_for_home(&home)
}

pub fn require_startup_identity_for_home(home: &Path) -> io::Result<ConfigDiscovery> {
    require_startup_identity_for_home_with_process_probe(home, true)
}

fn require_startup_identity_for_home_with_process_probe(
    home: &Path,
    probe_processes: bool,
) -> io::Result<ConfigDiscovery> {
    let discovery = discover_config_for_home(home)?;
    match discovery {
        ConfigDiscovery::LegacyOnly => Err(io::Error::new(
            io::ErrorKind::NotFound,
            LEGACY_ONLY_MIGRATION_GUIDANCE,
        )),
        ConfigDiscovery::UnreconciledCoexistence => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "canonical and legacy Mitsuro roots coexist without a migration receipt; refusing to choose an authority",
        )),
        ConfigDiscovery::MigratedWithRollback
            if probe_processes && legacy_generation_is_running(home) =>
        {
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the legacy Mitsuro generation is still running; refusing concurrent database authority",
            ))
        }
        ConfigDiscovery::MigratedWithRollback => {
            verify_preserved_source_authority(home, probe_processes)?;
            Ok(discovery)
        }
        _ => Ok(discovery),
    }
}

/// Copy legacy state into a new canonical root while retaining the complete
/// old root for rollback. Callers must explicitly confirm that both server
/// generations are quiesced before invoking this function.
pub fn migrate_config_root_offline_for_home(home: &Path) -> io::Result<ConfigMigrationReceipt> {
    migrate_config_root_offline_for_home_with_runtime_probe(home, true)
}

fn migrate_config_root_offline_for_home_with_runtime_probe(
    home: &Path,
    probe_ambient_runtime_locations: bool,
) -> io::Result<ConfigMigrationReceipt> {
    let canonical = home.join(CONFIG_DIR_NAME);
    let old = home.join(legacy::CONFIG_DIR_NAME);
    let receipt_source = receipt_source_path(&old)?;
    let existing_receipt = canonical.join(MIGRATION_RECEIPT_FILE_NAME);
    let canonical_exists = state_root_exists(&canonical, "canonical Mitsuro")?;
    let old_exists = state_root_exists(&old, "legacy Mitsuro")?;
    if canonical_exists && old_exists && validated_migration_receipt(&canonical, &old).is_some() {
        verify_preserved_source_authority(home, probe_ambient_runtime_locations)?;
        return Ok(ConfigMigrationReceipt {
            canonical_root: canonical,
            rollback_root: old,
            receipt_path: existing_receipt,
        });
    }
    if !old_exists {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("legacy configuration root not found at {}", old.display()),
        ));
    }
    if canonical_exists {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "canonical configuration root already exists at {}",
                canonical.display()
            ),
        ));
    }
    let config_root_generation_is_running =
        generation_is_running(
            &home.join(legacy::CONFIG_DIR_NAME),
            legacy::HIVE_SOCKET_FILE_NAME,
            true,
        ) || generation_is_running(&home.join(CONFIG_DIR_NAME), HIVE_SOCKET_FILE_NAME, false);
    let ambient_generation_is_running = probe_ambient_runtime_locations
        && (runtime_socket_is_live(true) || runtime_socket_is_live(false));
    if config_root_generation_is_running || ambient_generation_is_running {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "a Mitsuro process is still using legacy or canonical state; stop every CLI, TUI, desktop, server, and Hive process before migration",
        ));
    }
    if probe_ambient_runtime_locations {
        ensure_processes_quiescent(&old, ProcessQuiescencePolicy::OfflineCutover)?;
    }

    let staging = home.join(format!(".mitsuro.migrating.{}", std::process::id()));
    if path_exists_unfollowed(&staging)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "migration staging directory already exists at {}",
                staging.display()
            ),
        ));
    }
    create_private_directory(&staging)?;

    let cutover = match LegacySqliteCutover::acquire(&old.join(legacy::DATABASE_FILE_NAME)) {
        Ok(cutover) => cutover,
        Err(error) => {
            let _ = std::fs::remove_dir(&staging);
            return Err(error);
        }
    };
    let mut published = false;
    let result = (|| -> io::Result<()> {
        if probe_ambient_runtime_locations {
            ensure_processes_quiescent(&old, ProcessQuiescencePolicy::OfflineCutover)?;
        }
        let durable_tree = durable_tree_fingerprint(&old)?;
        copy_tree_without_sqlite(&old, &staging)?;
        let source_sqlite_snapshot =
            cutover.backup_snapshot_to(&staging.join(DATABASE_FILE_NAME))?;
        rename_directory_if_present(
            &staging.join(legacy::HIVE_DIR_NAME),
            &staging.join(HIVE_DIR_NAME),
        )?;
        rename_legacy_hive_profile_files(&staging.join(HIVE_DIR_NAME))?;
        rename_regular_file_if_present(
            &staging.join("run").join(legacy::HIVE_KEY_FILE_NAME),
            &staging.join("run").join(HIVE_KEY_FILE_NAME),
        )?;

        let database_path = staging.join(DATABASE_FILE_NAME);
        if database_path.is_file() {
            let database = crate::Database::new(&database_path).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("staged canonical database migration failed: {error:#}"),
                )
            })?;
            drop(database);
        }
        let final_durable_tree = durable_tree_fingerprint(&old)?;
        if final_durable_tree != durable_tree {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "durable legacy state changed during identity cutover",
            ));
        }
        let source_authority = SourceAuthorityFingerprint {
            sqlite: source_sqlite_snapshot,
            durable_tree,
        };

        let receipt = staging.join(MIGRATION_RECEIPT_FILE_NAME);
        let migrated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut receipt_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&receipt)?;
        receipt_file.write_all(
            format!(
                "version=2\nsource={}\ncreated_unix={}\nrollback_preserved=true\nsource_authority_fingerprint={}\n",
                receipt_source,
                migrated_at,
                source_authority.receipt_value()
            )
            .as_bytes(),
        )?;
        receipt_file.sync_all()?;
        drop(receipt_file);

        sync_tree(&staging)?;
        secure_private_directory(&staging)?;
        sync_directory(home)?;
        std::fs::rename(&staging, &canonical)?;
        published = true;
        sync_directory(home)
    })();
    let release_result = cutover.release();
    let result = combine_cutover_result(result, release_result);
    if let Err(error) = result {
        if published {
            return Err(quarantine_failed_published_cutover(home, error));
        }
        if path_exists_unfollowed(&staging).unwrap_or(false) {
            return match std::fs::remove_dir_all(&staging) {
                Ok(()) => Err(error),
                Err(cleanup_error) if cleanup_error.kind() == io::ErrorKind::NotFound => Err(error),
                Err(cleanup_error) => Err(io::Error::new(
                    error.kind(),
                    format!(
                        "identity migration failed: {error}; staging cleanup at {} also failed: {cleanup_error}",
                        staging.display()
                    ),
                )),
            };
        }
        return Err(error);
    }

    Ok(ConfigMigrationReceipt {
        receipt_path: canonical.join(MIGRATION_RECEIPT_FILE_NAME),
        canonical_root: canonical,
        rollback_root: old,
    })
}

fn quarantine_failed_published_cutover(home: &Path, error: io::Error) -> io::Error {
    match quarantine_failed_config_root_for_home(home) {
        Ok(quarantined) => io::Error::new(
            error.kind(),
            format!(
                "identity cutover failed after publication: {error}; the canonical root was quarantined at {} and the legacy root remains authoritative",
                quarantined.display()
            ),
        ),
        Err(quarantine_error) => io::Error::new(
            error.kind(),
            format!(
                "identity cutover failed after publication: {error}; quarantining the canonical root also failed: {quarantine_error}"
            ),
        ),
    }
}

fn combine_cutover_result(migration: io::Result<()>, release: io::Result<()>) -> io::Result<()> {
    match (migration, release) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(release_error)) => Err(io::Error::new(
            release_error.kind(),
            format!("failed to release legacy SQLite cutover fence: {release_error}"),
        )),
        (Err(error), Err(release_error)) => Err(io::Error::new(
            error.kind(),
            format!(
                "identity migration failed: {error}; releasing the SQLite cutover fence also failed: {release_error}"
            ),
        )),
    }
}

pub fn migrate_config_root_offline() -> io::Result<ConfigMigrationReceipt> {
    let home = dirs::home_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot determine the user home directory",
        )
    })?;
    migrate_config_root_offline_for_home(&home)
}

/// Move a failed canonical migration aside without deleting either authority.
pub fn quarantine_failed_config_root() -> io::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "cannot determine the user home directory",
        )
    })?;
    quarantine_failed_config_root_for_home(&home)
}

pub fn quarantine_failed_config_root_for_home(home: &Path) -> io::Result<PathBuf> {
    let canonical = home.join(CONFIG_DIR_NAME);
    if !state_root_exists(&canonical, "canonical Mitsuro")? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "canonical migration root does not exist",
        ));
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let failed = home.join(format!(
        ".mitsuro.failed.{timestamp}.{}",
        std::process::id()
    ));
    if path_exists_unfollowed(&failed)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "failed-migration recovery path already exists at {}",
                failed.display()
            ),
        ));
    }
    std::fs::rename(&canonical, &failed)?;
    sync_directory(home)?;
    Ok(failed)
}

pub fn legacy_config_dir_for_home(home: &Path) -> PathBuf {
    home.join(legacy::CONFIG_DIR_NAME)
}

pub fn legacy_project_state_dir(project_root: &Path) -> PathBuf {
    project_root.join(legacy::CONFIG_DIR_NAME)
}

/// Project state roots in read precedence order. New writes always target the
/// canonical root; the deprecated root remains a read-only fallback.
pub fn project_state_read_dirs(project_root: &Path) -> [PathBuf; 2] {
    [
        project_root.join(CONFIG_DIR_NAME),
        legacy_project_state_dir(project_root),
    ]
}

fn copy_tree_without_sqlite(source: &Path, destination: &Path) -> io::Result<()> {
    copy_tree_without_root_sqlite(source, destination, source, true)
}

fn copy_tree_without_root_sqlite(
    source: &Path,
    destination: &Path,
    source_root: &Path,
    is_config_root: bool,
) -> io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let source_path = entry.path();
        let target_path = destination.join(&name);
        let file_type = entry.file_type()?;
        let relative_path = source_path.strip_prefix(source_root).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "legacy configuration entry {} escaped source root {}: {error}",
                    source_path.display(),
                    source_root.display()
                ),
            )
        })?;
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to follow configuration symlink {}",
                    source_path.display()
                ),
            ));
        }
        if should_skip_legacy_entry(relative_path, &name, &file_type, is_config_root)? {
            continue;
        }
        if file_type.is_dir() {
            std::fs::create_dir(&target_path)?;
            copy_tree_without_root_sqlite(&source_path, &target_path, source_root, false)?;
            std::fs::set_permissions(&target_path, entry.metadata()?.permissions())?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to copy special configuration entry {}",
                    source_path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn should_skip_legacy_entry(
    relative_path: &Path,
    name: &std::ffi::OsStr,
    file_type: &std::fs::FileType,
    is_config_root: bool,
) -> io::Result<bool> {
    if is_config_root && is_legacy_sqlite_root_entry(name) {
        return Ok(true);
    }
    if relative_path == Path::new("logs") || relative_path == Path::new("backups") {
        if file_type.is_dir() {
            return Ok(true);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy excluded artifact is not a directory at {}",
                relative_path.display()
            ),
        ));
    }
    if is_known_ephemeral_legacy_file(relative_path) {
        if file_type.is_file() {
            return Ok(true);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy runtime artifact is not a regular file at {}",
                relative_path.display()
            ),
        ));
    }
    if relative_path == Path::new("run").join(legacy::HIVE_SOCKET_FILE_NAME) {
        if is_legacy_runtime_socket(file_type) {
            return Ok(true);
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy runtime socket has an unexpected file type at {}",
                relative_path.display()
            ),
        ));
    }
    Ok(false)
}

fn is_legacy_sqlite_root_entry(name: &std::ffi::OsStr) -> bool {
    [
        legacy::DATABASE_FILE_NAME.to_string(),
        format!("{}-wal", legacy::DATABASE_FILE_NAME),
        format!("{}-shm", legacy::DATABASE_FILE_NAME),
        format!("{}-journal", legacy::DATABASE_FILE_NAME),
    ]
    .iter()
    .any(|candidate| name == std::ffi::OsStr::new(candidate))
}

fn is_known_ephemeral_legacy_file(relative_path: &Path) -> bool {
    if relative_path == Path::new("server.pid")
        || relative_path == Path::new("krusty.log")
        || relative_path == Path::new("krusty-desktop-server.log")
        || relative_path == Path::new("plugins").join(".mutation.lock")
        || relative_path == Path::new("tokens").join("oauth.lock")
        || relative_path == Path::new("tokens").join("vapid_key.pem.lock")
    {
        return true;
    }
    let Some(parent) = relative_path.parent() else {
        return false;
    };
    let Some(name) = relative_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    parent == Path::new("tokens")
        && name
            .strip_prefix("oauth.")
            .and_then(|name| name.strip_suffix(".refresh.lock"))
            .is_some_and(|provider| !provider.is_empty())
}

#[cfg(unix)]
fn is_legacy_runtime_socket(file_type: &std::fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt;

    file_type.is_socket()
}

#[cfg(not(unix))]
fn is_legacy_runtime_socket(_file_type: &std::fs::FileType) -> bool {
    false
}

fn rename_legacy_hive_profile_files(hive_dir: &Path) -> io::Result<()> {
    for (old_name, canonical_name) in [
        (legacy::HIVE_SOUL_FILE_NAME, crate::paths::HIVE_SOUL_FILE),
        (
            legacy::HIVE_IDENTITY_FILE_NAME,
            crate::paths::HIVE_IDENTITY_FILE,
        ),
        (
            legacy::HIVE_HEARTBEAT_FILE_NAME,
            crate::paths::HIVE_HEARTBEAT_FILE,
        ),
        (
            legacy::HIVE_MEMORY_FILE_NAME,
            crate::paths::HIVE_MEMORY_FILE,
        ),
        (
            legacy::HIVE_CHANNELS_FILE_NAME,
            crate::paths::HIVE_CHANNELS_FILE,
        ),
    ] {
        rename_regular_file_if_present(&hive_dir.join(old_name), &hive_dir.join(canonical_name))?;
    }
    Ok(())
}

struct LegacySqliteCutover {
    source_path: PathBuf,
    source_permissions: Option<std::fs::Permissions>,
    writer_fence: Option<rusqlite::Connection>,
    source: Option<rusqlite::Connection>,
}

impl LegacySqliteCutover {
    fn acquire(source_path: &Path) -> io::Result<Self> {
        let source_metadata = validate_legacy_sqlite_authority(source_path)?;
        let Some(source_metadata) = source_metadata else {
            return Ok(Self {
                source_path: source_path.to_path_buf(),
                source_permissions: None,
                writer_fence: None,
                source: None,
            });
        };

        // BEGIN IMMEDIATE occupies SQLite's single-writer slot without
        // changing pages. Keep it open through schema migration, receipt
        // creation, fsync, and the final atomic cutover.
        let writer_fence = rusqlite::Connection::open_with_flags(
            source_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(sqlite_io_error)?;
        writer_fence
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(sqlite_io_error)?;
        writer_fence.execute_batch("BEGIN IMMEDIATE").map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "could not acquire the legacy SQLite cutover fence; stop every process using legacy Mitsuro state before migration: {error}"
                ),
            )
        })?;

        // Re-check sidecar types after taking the fence and before SQLite is
        // allowed to resolve any sidecar path.
        validate_legacy_sqlite_authority(source_path)?;
        let source = rusqlite::Connection::open_with_flags(
            source_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(sqlite_io_error)?;
        source
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(sqlite_io_error)?;
        validate_sqlite_connection(&source, "legacy")?;

        Ok(Self {
            source_path: source_path.to_path_buf(),
            source_permissions: Some(source_metadata.permissions()),
            writer_fence: Some(writer_fence),
            source: Some(source),
        })
    }

    fn backup_snapshot_to(&self, destination_path: &Path) -> io::Result<SourceSqliteSnapshot> {
        let Some(source) = self.source.as_ref() else {
            validate_legacy_sqlite_authority(&self.source_path)?;
            return Ok(SourceSqliteSnapshot::Absent);
        };
        if path_exists_unfollowed(destination_path)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "SQLite snapshot target already exists at {}",
                    destination_path.display()
                ),
            ));
        }
        source
            .backup(rusqlite::DatabaseName::Main, destination_path, None)
            .map_err(sqlite_io_error)?;
        let result = (|| {
            validate_sqlite_copy(destination_path)?;
            if let Some(permissions) = &self.source_permissions {
                std::fs::set_permissions(destination_path, permissions.clone())?;
            }
            std::fs::File::open(destination_path)?.sync_all()?;
            Ok(SourceSqliteSnapshot::Sha256 {
                digest: hash_file_sha256(destination_path)?,
                source_stat: sqlite_source_stat(&self.source_path)?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "legacy SQLite authority disappeared during cutover",
                    )
                })?,
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(destination_path);
        }
        result
    }

    fn fingerprint_snapshot(&self) -> io::Result<SourceSqliteSnapshot> {
        if self.source.is_none() {
            validate_legacy_sqlite_authority(&self.source_path)?;
            return Ok(SourceSqliteSnapshot::Absent);
        }
        let scratch = IdentityScratchDirectory::new()?;
        self.backup_snapshot_to(&scratch.path.join("source-snapshot.db"))
    }

    fn release(mut self) -> io::Result<()> {
        self.source.take();
        match self.writer_fence.take() {
            Some(writer_fence) => writer_fence
                .execute_batch("ROLLBACK")
                .map_err(sqlite_io_error),
            None => Ok(()),
        }
    }
}

fn validate_legacy_sqlite_authority(source_path: &Path) -> io::Result<Option<std::fs::Metadata>> {
    let main = regular_file_metadata_if_present(source_path, "legacy SQLite authority")?;
    let rollback_journal = sqlite_sidecar_path(source_path, "-journal");
    if regular_file_metadata_if_present(&rollback_journal, "legacy SQLite rollback journal")?
        .is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy SQLite rollback journal must be recovered by the old generation before offline migration: {}",
                rollback_journal.display()
            ),
        ));
    }
    let mut present_sidecars = Vec::new();
    for suffix in ["-wal", "-shm"] {
        let sidecar = sqlite_sidecar_path(source_path, suffix);
        if regular_file_metadata_if_present(&sidecar, "legacy SQLite sidecar")?.is_some() {
            present_sidecars.push(sidecar);
        }
    }
    if main.is_none() && !present_sidecars.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy SQLite sidecar exists without the main database: {}",
                present_sidecars[0].display()
            ),
        ));
    }
    Ok(main)
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = database_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn sqlite_source_stat(source_path: &Path) -> io::Result<Option<SqliteSourceStat>> {
    let Some(main) = validate_legacy_sqlite_authority(source_path)? else {
        return Ok(None);
    };
    let wal_path = sqlite_sidecar_path(source_path, "-wal");
    let wal = regular_file_metadata_if_present(&wal_path, "legacy SQLite WAL")?
        .map(sqlite_file_stat)
        .transpose()?;
    Ok(Some(SqliteSourceStat {
        main: sqlite_file_stat(main)?,
        wal,
    }))
}

fn sqlite_file_stat(metadata: std::fs::Metadata) -> io::Result<SqliteFileStat> {
    Ok(SqliteFileStat {
        len: metadata.len(),
        modified_ns: metadata_modified_ns(&metadata)?,
    })
}

fn regular_file_metadata_if_present(
    path: &Path,
    label: &str,
) -> io::Result<Option<std::fs::Metadata>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} is not a regular file at {}", path.display()),
        ));
    }
    Ok(Some(metadata))
}

fn validate_sqlite_connection(connection: &rusqlite::Connection, label: &str) -> io::Result<()> {
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(sqlite_io_error)?;
    if quick_check != "ok" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} SQLite quick_check failed: {quick_check}"),
        ));
    }
    let foreign_key_violation: Option<String> = connection
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()
        .map_err(sqlite_io_error)?;
    if let Some(table) = foreign_key_violation {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} SQLite foreign_key_check failed in {table}"),
        ));
    }
    Ok(())
}

fn hash_file_sha256(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn durable_tree_fingerprint(root: &Path) -> io::Result<DurableTreeFingerprint> {
    let mut content = Sha256::new();
    content.update(b"mitsuro.identity.durable-tree.content.v1\0");
    let mut stat = Sha256::new();
    stat.update(b"mitsuro.identity.durable-tree.stat.v1\0");
    hash_durable_tree_directory(root, root, Some(&mut content), &mut stat)?;
    Ok(DurableTreeFingerprint {
        content_sha256: format!("{:x}", content.finalize()),
        stat_sha256: format!("{:x}", stat.finalize()),
    })
}

fn durable_tree_stat_sha256(root: &Path) -> io::Result<String> {
    let mut stat = Sha256::new();
    stat.update(b"mitsuro.identity.durable-tree.stat.v1\0");
    hash_durable_tree_directory(root, root, None, &mut stat)?;
    Ok(format!("{:x}", stat.finalize()))
}

fn hash_durable_tree_directory(
    root: &Path,
    directory: &Path,
    mut content: Option<&mut Sha256>,
    stat: &mut Sha256,
) -> io::Result<()> {
    let mut entries = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| os_str_bytes(&entry.file_name()));
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("durable-state entry escaped legacy root: {error}"),
            )
        })?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("refusing durable-state symlink {}", path.display()),
            ));
        }
        if should_skip_legacy_entry(relative, &entry.file_name(), &file_type, directory == root)? {
            continue;
        }
        let metadata = entry.metadata()?;
        let marker = if file_type.is_dir() {
            b'D'
        } else if file_type.is_file() {
            b'F'
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("refusing special durable-state entry {}", path.display()),
            ));
        };
        hash_tree_entry_prefix(stat, marker, relative, &metadata, true)?;
        if let Some(content) = content.as_deref_mut() {
            hash_tree_entry_prefix(content, marker, relative, &metadata, false)?;
        }
        if file_type.is_dir() {
            hash_durable_tree_directory(root, &path, content.as_deref_mut(), stat)?;
        } else if let Some(content) = content.as_deref_mut() {
            let before_len = metadata.len();
            let before_modified = metadata_modified_ns(&metadata)?;
            let mut file = std::fs::File::open(&path)?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                content.update(&buffer[..read]);
            }
            let after = std::fs::symlink_metadata(&path)?;
            if !after.file_type().is_file()
                || after.len() != before_len
                || metadata_modified_ns(&after)? != before_modified
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "durable legacy state changed while fingerprinting {}",
                        path.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn hash_tree_entry_prefix(
    hasher: &mut Sha256,
    marker: u8,
    relative: &Path,
    metadata: &std::fs::Metadata,
    include_stat: bool,
) -> io::Result<()> {
    let path = os_str_bytes(relative.as_os_str());
    hasher.update([marker]);
    hasher.update((path.len() as u64).to_le_bytes());
    hasher.update(path);
    hasher.update(metadata_mode(metadata).to_le_bytes());
    if include_stat {
        hasher.update(metadata.len().to_le_bytes());
        hasher.update(metadata_modified_ns(metadata)?.to_le_bytes());
    } else if metadata.file_type().is_file() {
        hasher.update(metadata.len().to_le_bytes());
    }
    Ok(())
}

#[cfg(unix)]
fn os_str_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    value.as_bytes().to_vec()
}

#[cfg(windows)]
fn os_str_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    value
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>()
}

#[cfg(not(any(unix, windows)))]
fn os_str_bytes(value: &std::ffi::OsStr) -> Vec<u8> {
    value.to_string_lossy().as_bytes().to_vec()
}

#[cfg(unix)]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

fn metadata_modified_ns(metadata: &std::fs::Metadata) -> io::Result<u128> {
    metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

struct IdentityScratchDirectory {
    path: PathBuf,
}

impl IdentityScratchDirectory {
    fn new() -> io::Result<Self> {
        for _ in 0..32 {
            let path = std::env::temp_dir().join(format!(
                ".mitsuro-identity-{}-{:016x}",
                std::process::id(),
                rand::random::<u64>()
            ));
            match create_private_directory(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique SQLite fingerprint scratch directory",
        ))
    }
}

impl Drop for IdentityScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn sync_tree(path: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to sync identity-migration symlink {}",
                    entry_path.display()
                ),
            ));
        }
        if file_type.is_dir() {
            sync_tree(&entry_path)?;
        } else if file_type.is_file() {
            std::fs::File::open(&entry_path)?.sync_all()?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "refusing to sync special identity-migration entry {}",
                    entry_path.display()
                ),
            ));
        }
    }
    sync_directory(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    // std does not expose a portable directory handle suitable for fsync.
    // Every staged file is still flushed before the atomic rename.
    Ok(())
}

fn validate_sqlite_copy(path: &Path) -> io::Result<()> {
    let copy =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sqlite_io_error)?;
    validate_sqlite_connection(&copy, "canonical copy")
}

impl SourceSqliteSnapshot {
    fn receipt_value(&self) -> String {
        match self {
            Self::Absent => "absent".to_string(),
            Self::Sha256 {
                digest,
                source_stat,
            } => {
                let (wal_len, wal_mtime_ns) = source_stat.wal.as_ref().map_or_else(
                    || ("absent".to_string(), "absent".to_string()),
                    |wal| (wal.len.to_string(), wal.modified_ns.to_string()),
                );
                format!(
                    "{digest};main_len={};main_mtime_ns={};wal_len={wal_len};wal_mtime_ns={wal_mtime_ns}",
                    source_stat.main.len, source_stat.main.modified_ns
                )
            }
        }
    }

    fn parse_receipt_value(value: &str) -> Option<Self> {
        if value == "absent" {
            return Some(Self::Absent);
        }
        let mut fields = value.split(';');
        let digest = fields.next()?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        let main_len = fields.next()?.strip_prefix("main_len=")?.parse().ok()?;
        let main_mtime_ns = fields
            .next()?
            .strip_prefix("main_mtime_ns=")?
            .parse()
            .ok()?;
        let wal_len = fields.next()?.strip_prefix("wal_len=")?;
        let wal_mtime_ns = fields.next()?.strip_prefix("wal_mtime_ns=")?;
        if fields.next().is_some() {
            return None;
        }
        let wal = match (wal_len, wal_mtime_ns) {
            ("absent", "absent") => None,
            ("absent", _) | (_, "absent") => return None,
            (len, modified_ns) => Some(SqliteFileStat {
                len: len.parse().ok()?,
                modified_ns: modified_ns.parse().ok()?,
            }),
        };
        Some(Self::Sha256 {
            digest: digest.to_string(),
            source_stat: SqliteSourceStat {
                main: SqliteFileStat {
                    len: main_len,
                    modified_ns: main_mtime_ns,
                },
                wal,
            },
        })
    }

    fn stat_matches(&self, current: Option<&SqliteSourceStat>) -> bool {
        match (self, current) {
            (Self::Absent, None) => true,
            (Self::Sha256 { source_stat, .. }, Some(current)) => source_stat == current,
            _ => false,
        }
    }

    fn same_database_content(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Absent, Self::Absent) => true,
            (Self::Sha256 { digest: left, .. }, Self::Sha256 { digest: right, .. }) => {
                left == right
            }
            _ => false,
        }
    }
}

impl SourceAuthorityFingerprint {
    fn receipt_value(&self) -> String {
        format!(
            "sqlite={}|tree_sha256={}|tree_stat_sha256={}",
            self.sqlite.receipt_value(),
            self.durable_tree.content_sha256,
            self.durable_tree.stat_sha256
        )
    }

    fn parse_receipt_value(value: &str) -> Option<Self> {
        let mut fields = value.split('|');
        let sqlite =
            SourceSqliteSnapshot::parse_receipt_value(fields.next()?.strip_prefix("sqlite=")?)?;
        let tree_sha256 = fields.next()?.strip_prefix("tree_sha256=")?;
        let tree_stat_sha256 = fields.next()?.strip_prefix("tree_stat_sha256=")?;
        if fields.next().is_some()
            || !is_lowercase_sha256(tree_sha256)
            || !is_lowercase_sha256(tree_stat_sha256)
        {
            return None;
        }
        Some(Self {
            sqlite,
            durable_tree: DurableTreeFingerprint {
                content_sha256: tree_sha256.to_string(),
                stat_sha256: tree_stat_sha256.to_string(),
            },
        })
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validated_migration_receipt(
    canonical_root: &Path,
    expected_source: &Path,
) -> Option<ValidatedMigrationReceipt> {
    let receipt_path = canonical_root.join(MIGRATION_RECEIPT_FILE_NAME);
    let metadata = std::fs::symlink_metadata(&receipt_path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MIGRATION_RECEIPT_MAX_BYTES {
        return None;
    }
    let contents = std::fs::read_to_string(receipt_path).ok()?;
    if contents.contains('\r') || !contents.ends_with('\n') {
        return None;
    }
    let lines: Vec<&str> = contents.strip_suffix('\n')?.split('\n').collect();
    let [version, source, created_unix, rollback_preserved, source_authority] = lines.as_slice()
    else {
        return None;
    };
    if *version != "version=2" || *rollback_preserved != "rollback_preserved=true" {
        return None;
    }
    let source = PathBuf::from(source.strip_prefix("source=")?);
    if source != expected_source {
        return None;
    }
    let created_unix = created_unix.strip_prefix("created_unix=")?.parse().ok()?;
    let source_authority = SourceAuthorityFingerprint::parse_receipt_value(
        source_authority.strip_prefix("source_authority_fingerprint=")?,
    )?;
    Some(ValidatedMigrationReceipt {
        source,
        created_unix,
        source_authority,
    })
}

fn verify_preserved_source_authority(home: &Path, probe_processes: bool) -> io::Result<()> {
    let canonical = home.join(CONFIG_DIR_NAME);
    let old = home.join(legacy::CONFIG_DIR_NAME);
    let receipt = validated_migration_receipt(&canonical, &old).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "canonical identity migration receipt is missing or invalid",
        )
    })?;
    if probe_processes {
        ensure_processes_quiescent(&old, ProcessQuiescencePolicy::PreservedLegacyAuthority)?;
    }
    let database_path = old.join(legacy::DATABASE_FILE_NAME);
    let current_sqlite_stat = sqlite_source_stat(&database_path)?;
    let sqlite_stat_matches = receipt
        .source_authority
        .sqlite
        .stat_matches(current_sqlite_stat.as_ref());
    let current_tree_stat = durable_tree_stat_sha256(&old)?;
    let tree_stat_matches = current_tree_stat == receipt.source_authority.durable_tree.stat_sha256;
    if sqlite_stat_matches && tree_stat_matches {
        return Ok(());
    }

    if !sqlite_stat_matches {
        let cutover = LegacySqliteCutover::acquire(&database_path)?;
        let actual = cutover.fingerprint_snapshot();
        let release = cutover.release();
        let actual = match (actual, release) {
            (Ok(snapshot), Ok(())) => snapshot,
            (Err(error), Ok(())) => return Err(error),
            (Ok(_), Err(error)) => return Err(error),
            (Err(error), Err(release_error)) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "legacy source verification failed: {error}; releasing its SQLite fence also failed: {release_error}"
                    ),
                ))
            }
        };
        if !receipt
            .source_authority
            .sqlite
            .same_database_content(&actual)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "preserved legacy SQLite authority diverged after Mitsuro cutover; refusing canonical startup",
            ));
        }
    }
    if !tree_stat_matches {
        let actual = durable_tree_fingerprint(&old)?;
        if actual.content_sha256 != receipt.source_authority.durable_tree.content_sha256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "preserved legacy durable state diverged after Mitsuro cutover; refusing canonical startup",
            ));
        }
    }
    Ok(())
}

fn rename_directory_if_present(old: &Path, canonical: &Path) -> io::Result<()> {
    rename_typed_child_if_present(old, canonical, true)
}

fn rename_regular_file_if_present(old: &Path, canonical: &Path) -> io::Result<()> {
    rename_typed_child_if_present(old, canonical, false)
}

fn rename_typed_child_if_present(
    old: &Path,
    canonical: &Path,
    expect_directory: bool,
) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(old) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let expected_type_matches = if expect_directory {
        metadata.file_type().is_dir()
    } else {
        metadata.file_type().is_file()
    };
    if !expected_type_matches {
        let expected = if expect_directory {
            "directory"
        } else {
            "regular file"
        };
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "identity migration source must be a {expected} at {}",
                old.display()
            ),
        ));
    }
    if path_exists_unfollowed(canonical)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "identity migration target already exists at {}",
                canonical.display()
            ),
        ));
    }
    if let Some(parent) = canonical.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(old, canonical)
}

fn sqlite_io_error(error: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(target_os = "linux")]
fn ensure_processes_quiescent(old_root: &Path, policy: ProcessQuiescencePolicy) -> io::Result<()> {
    inspect_linux_legacy_processes(
        Path::new("/proc"),
        old_root,
        unsafe { libc::geteuid() },
        std::process::id(),
        policy,
    )
}

#[cfg(not(target_os = "linux"))]
fn ensure_processes_quiescent(
    _old_root: &Path,
    _policy: ProcessQuiescencePolicy,
) -> io::Result<()> {
    // Other platforms still require the explicit --confirm-offline contract,
    // runtime socket checks, and the SQLite cutover fence. The installer does
    // not claim automated process proof where the OS has no procfs analogue.
    Ok(())
}

#[cfg(target_os = "linux")]
fn inspect_linux_legacy_processes(
    proc_root: &Path,
    old_root: &Path,
    current_uid: u32,
    current_pid: u32,
    policy: ProcessQuiescencePolicy,
) -> io::Result<()> {
    if !proc_root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cannot prove legacy Mitsuro process quiescence because {} is unavailable",
                proc_root.display()
            ),
        ));
    }
    let old_root = std::fs::canonicalize(old_root)?;
    let legacy_database = old_root.join(legacy::DATABASE_FILE_NAME);
    let protected_fds = [
        legacy_database.clone(),
        sqlite_sidecar_path(&legacy_database, "-wal"),
        sqlite_sidecar_path(&legacy_database, "-shm"),
    ];

    for entry in std::fs::read_dir(proc_root)? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let process_dir = entry.path();
        let status = match std::fs::read_to_string(process_dir.join("status")) {
            Ok(status) => status,
            Err(_error) if !process_dir.exists() => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "cannot inspect process {pid} while proving offline identity migration: {error}"
                    ),
                ))
            }
        };
        let (uid, zombie) = parse_linux_process_status(&status).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cannot parse process status for PID {pid}"),
            )
        })?;
        if uid != current_uid || zombie {
            continue;
        }

        let executable = match std::fs::read_link(process_dir.join("exe")) {
            Ok(path) => strip_deleted_proc_suffix(path),
            Err(_error) if !process_dir.exists() => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("cannot resolve executable for same-user PID {pid}: {error}"),
                ))
            }
        };
        let blocked_executable = match policy {
            ProcessQuiescencePolicy::OfflineCutover => is_any_mitsuro_executable(&executable),
            ProcessQuiescencePolicy::PreservedLegacyAuthority => {
                is_legacy_mitsuro_executable(&executable)
            }
        };
        if blocked_executable {
            let action = match policy {
                ProcessQuiescencePolicy::OfflineCutover => {
                    "stop every Mitsuro CLI, TUI, desktop, server, and Hive process before migration"
                }
                ProcessQuiescencePolicy::PreservedLegacyAuthority => {
                    "stop the previous-generation process before starting canonical Mitsuro"
                }
            };
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "Mitsuro process PID {pid} is still running from {}; {action}",
                    executable.display(),
                ),
            ));
        }

        let fd_dir = process_dir.join("fd");
        let descriptors = match std::fs::read_dir(&fd_dir) {
            Ok(descriptors) => descriptors,
            Err(_error) if !process_dir.exists() => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("cannot inspect file descriptors for same-user PID {pid}: {error}"),
                ))
            }
        };
        for descriptor in descriptors {
            let descriptor = match descriptor {
                Ok(descriptor) => descriptor,
                Err(_error) if !process_dir.exists() => break,
                Err(error) => return Err(error),
            };
            let target = match std::fs::read_link(descriptor.path()) {
                Ok(target) => strip_deleted_proc_suffix(target),
                Err(_) if !descriptor.path().exists() => continue,
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("cannot inspect a file descriptor for PID {pid}: {error}"),
                    ))
                }
            };
            if protected_fds
                .iter()
                .any(|protected| target.as_path() == protected.as_path())
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "same-user process PID {pid} still has previous Mitsuro state open at {}; stop every process using legacy state before migration",
                        target.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn parse_linux_process_status(status: &str) -> Option<(u32, bool)> {
    let uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let state = status
        .lines()
        .find_map(|line| line.strip_prefix("State:"))?
        .split_whitespace()
        .next()?;
    Some((uid, state == "Z"))
}

#[cfg(target_os = "linux")]
fn strip_deleted_proc_suffix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    text.strip_suffix(" (deleted)")
        .map_or(path.clone(), PathBuf::from)
}

fn is_legacy_mitsuro_executable(path: &Path) -> bool {
    if path
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new(".krusty-releases"))
    {
        return true;
    }
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            "krusty"
                | "krusty.exe"
                | "krusty-mako"
                | "krusty-mako.exe"
                | "krusty-desktop"
                | "krusty-desktop.exe"
                | "Krusty"
                | "Krusty.exe"
        )
    )
}

fn is_any_mitsuro_executable(path: &Path) -> bool {
    is_legacy_mitsuro_executable(path) || is_canonical_mitsuro_executable(path)
}

fn is_canonical_mitsuro_executable(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new(".mitsuro-releases"))
        || matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some(
                "mitsuro"
                    | "mitsuro.exe"
                    | "mitsuro-hive"
                    | "mitsuro-hive.exe"
                    | "mitsuro-desktop"
                    | "mitsuro-desktop.exe"
                    | "Mitsuro"
                    | "Mitsuro.exe"
            )
        )
}

fn executable_matches_generation(path: &Path, legacy_generation: bool) -> bool {
    if legacy_generation {
        is_legacy_mitsuro_executable(path)
    } else {
        is_canonical_mitsuro_executable(path)
    }
}

fn legacy_generation_is_running(home: &Path) -> bool {
    generation_is_running(
        &home.join(legacy::CONFIG_DIR_NAME),
        legacy::HIVE_SOCKET_FILE_NAME,
        true,
    ) || runtime_socket_is_live(true)
}

fn runtime_socket_is_live(old: bool) -> bool {
    if configured_runtime_socket(old).is_some_and(|path| socket_is_live(&path)) {
        return true;
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            #[cfg(unix)]
            {
                Some(default_user_runtime_dir())
            }
            #[cfg(not(unix))]
            {
                None
            }
        });
    #[cfg(unix)]
    let fallback_is_live = socket_is_live(&fallback_runtime_socket(old));
    #[cfg(not(unix))]
    let fallback_is_live = false;
    runtime_dir
        .as_deref()
        .is_some_and(|runtime_dir| runtime_socket_is_live_at(runtime_dir, old))
        || cache_runtime_socket(old).is_some_and(|path| socket_is_live(&path))
        || fallback_is_live
}

fn configured_runtime_socket(old: bool) -> Option<PathBuf> {
    let variable = if old {
        format!("{}SOCKET", legacy::HIVE_ENV_PREFIX)
    } else {
        format!("{HIVE_ENV_PREFIX}SOCKET")
    };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn runtime_socket_is_live_at(runtime_dir: &Path, old: bool) -> bool {
    let path = if old {
        runtime_dir
            .join(legacy::CLI_NAME)
            .join(legacy::HIVE_SOCKET_FILE_NAME)
    } else {
        runtime_dir.join("mitsuro").join(HIVE_SOCKET_FILE_NAME)
    };
    socket_is_live(&path)
}

#[cfg(any(target_os = "macos", test))]
fn cache_runtime_socket_at(cache_dir: &Path, old: bool) -> PathBuf {
    if old {
        cache_dir
            .join(legacy::CLI_NAME)
            .join("run")
            .join(legacy::HIVE_SOCKET_FILE_NAME)
    } else {
        cache_dir
            .join("mitsuro")
            .join("run")
            .join(HIVE_SOCKET_FILE_NAME)
    }
}

#[cfg(target_os = "macos")]
fn cache_runtime_socket(old: bool) -> Option<PathBuf> {
    dirs::cache_dir().map(|cache_dir| cache_runtime_socket_at(&cache_dir, old))
}

#[cfg(not(target_os = "macos"))]
fn cache_runtime_socket(_old: bool) -> Option<PathBuf> {
    None
}

#[cfg(unix)]
fn default_user_runtime_dir() -> PathBuf {
    PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }))
}

#[cfg(unix)]
fn fallback_runtime_socket(old: bool) -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    if old {
        std::env::temp_dir()
            .join(format!("{}-{uid}", legacy::CLI_NAME))
            .join(legacy::HIVE_SOCKET_FILE_NAME)
    } else {
        std::env::temp_dir()
            .join(format!("mitsuro-{uid}"))
            .join(HIVE_SOCKET_FILE_NAME)
    }
}

fn generation_is_running(root: &Path, socket_name: &str, legacy_generation: bool) -> bool {
    pid_file_is_live(&root.join("server.pid"), legacy_generation)
        || socket_is_live(&root.join("run").join(socket_name))
}

fn pid_file_is_live(path: &Path, legacy_generation: bool) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(pid) = contents
        .trim()
        .split(':')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return false;
    };
    process_matches_generation(pid, legacy_generation)
}

#[cfg(target_os = "linux")]
fn process_matches_generation(pid: u32, legacy_generation: bool) -> bool {
    let process_dir = PathBuf::from(format!("/proc/{pid}"));
    let Ok(status) = std::fs::read_to_string(process_dir.join("status")) else {
        return false;
    };
    let Some((uid, zombie)) = parse_linux_process_status(&status) else {
        return false;
    };
    if uid != unsafe { libc::geteuid() } || zombie {
        return false;
    }
    std::fs::read_link(process_dir.join("exe"))
        .map(strip_deleted_proc_suffix)
        .is_ok_and(|path| executable_matches_generation(&path, legacy_generation))
}

#[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
fn process_matches_generation(_pid: u32, _legacy_generation: bool) -> bool {
    // A PID alone is not proof of process identity and may have been reused.
    // Socket probes remain authoritative on platforms without a safe process
    // executable lookup in this module.
    false
}

#[cfg(target_os = "windows")]
fn process_matches_generation(pid: u32, legacy_generation: bool) -> bool {
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut buffer = vec![0_u16; 32_768];
    let mut len = buffer.len() as u32;
    let queried = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) };
    unsafe {
        CloseHandle(handle);
    }
    if queried == 0 {
        return false;
    }
    let executable = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len as usize]));
    executable_matches_generation(&executable, legacy_generation)
}

#[cfg(unix)]
fn socket_is_live(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
fn socket_is_live(_path: &Path) -> bool {
    false
}

fn legacy_env_name(canonical: &str) -> Option<String> {
    if let Some(suffix) = canonical.strip_prefix(HIVE_ENV_PREFIX) {
        return Some(format!("{}{suffix}", legacy::HIVE_ENV_PREFIX));
    }
    canonical
        .strip_prefix(ENV_PREFIX)
        .map(|suffix| format!("{}{suffix}", legacy::ENV_PREFIX))
}

fn canonical_env_name(old: &str) -> Option<String> {
    if let Some(suffix) = old.strip_prefix(legacy::HIVE_ENV_PREFIX) {
        return Some(format!("{HIVE_ENV_PREFIX}{suffix}"));
    }
    old.strip_prefix(legacy::ENV_PREFIX)
        .map(|suffix| format!("{ENV_PREFIX}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrate_isolated(home: &Path) -> io::Result<ConfigMigrationReceipt> {
        migrate_config_root_offline_for_home_with_runtime_probe(home, false)
    }

    fn create_legacy_root(home: &Path) -> PathBuf {
        let old = home.join(legacy::CONFIG_DIR_NAME);
        std::fs::create_dir(&old).expect("legacy root");
        old
    }

    fn valid_empty_v2_receipt(old: &Path) -> String {
        let authority = SourceAuthorityFingerprint {
            sqlite: SourceSqliteSnapshot::Absent,
            durable_tree: durable_tree_fingerprint(old).expect("empty authority fingerprint"),
        };
        format!(
            "version=2\nsource={}\ncreated_unix=1\nrollback_preserved=true\nsource_authority_fingerprint={}\n",
            old.to_str().expect("UTF-8 test path"),
            authority.receipt_value()
        )
    }

    #[test]
    fn environment_names_translate_without_changing_unrelated_keys() {
        assert_eq!(
            legacy_env_name("MITSURO_HIVE_SOCKET").as_deref(),
            Some("KRUSTY_MAKO_SOCKET")
        );
        assert_eq!(
            legacy_env_name("MITSURO_PROVIDER").as_deref(),
            Some("KRUSTY_PROVIDER")
        );
        assert_eq!(legacy_env_name("PATH"), None);
    }

    #[test]
    fn offline_migration_copies_state_and_preserves_rollback_root() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = temp.path().join(legacy::CONFIG_DIR_NAME);
        let old_hive = old.join(legacy::HIVE_DIR_NAME);
        std::fs::create_dir_all(&old_hive).expect("legacy Hive dir");
        for (name, contents) in [
            (legacy::HIVE_SOUL_FILE_NAME, "live soul"),
            (legacy::HIVE_IDENTITY_FILE_NAME, "live identity"),
            (legacy::HIVE_HEARTBEAT_FILE_NAME, "live heartbeat"),
            (legacy::HIVE_MEMORY_FILE_NAME, "live memory"),
            (legacy::HIVE_CHANNELS_FILE_NAME, "live channels"),
        ] {
            std::fs::write(old_hive.join(name), contents).expect("legacy profile document");
        }
        let nested_backup = old.join("backups").join("plugin-data");
        std::fs::create_dir_all(&nested_backup).expect("nested backup dir");
        for name in [legacy::DATABASE_FILE_NAME, "krusty.db-wal", "krusty.db-shm"] {
            std::fs::write(nested_backup.join(name), format!("nested {name}"))
                .expect("nested database-named file");
        }
        std::fs::write(old.join("sentinel"), b"rollback").expect("legacy sentinel");
        let db = rusqlite::Connection::open(old.join(legacy::DATABASE_FILE_NAME)).expect("db");
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE sample (id INTEGER PRIMARY KEY); INSERT INTO sample VALUES (1);")
            .expect("seed db");
        drop(db);

        let receipt = migrate_isolated(temp.path()).expect("migrate root");
        assert!(receipt.canonical_root.join(DATABASE_FILE_NAME).is_file());
        assert!(receipt.canonical_root.join(HIVE_DIR_NAME).is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&receipt.canonical_root)
                    .expect("canonical metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "published canonical authority must remain private"
            );
        }
        for (name, contents) in [
            (crate::paths::HIVE_SOUL_FILE, "live soul"),
            (crate::paths::HIVE_IDENTITY_FILE, "live identity"),
            (crate::paths::HIVE_HEARTBEAT_FILE, "live heartbeat"),
            (crate::paths::HIVE_MEMORY_FILE, "live memory"),
            (crate::paths::HIVE_CHANNELS_FILE, "live channels"),
        ] {
            assert_eq!(
                std::fs::read_to_string(receipt.canonical_root.join(HIVE_DIR_NAME).join(name))
                    .expect("canonical profile document"),
                contents
            );
        }
        assert!(
            !receipt.canonical_root.join("backups").exists(),
            "legacy recovery snapshots remain only in the preserved rollback root"
        );
        for name in [legacy::DATABASE_FILE_NAME, "krusty.db-wal", "krusty.db-shm"] {
            assert_eq!(
                std::fs::read_to_string(
                    receipt
                        .rollback_root
                        .join("backups")
                        .join("plugin-data")
                        .join(name)
                )
                .expect("rollback database-named recovery file"),
                format!("nested {name}")
            );
        }
        assert!(receipt.receipt_path.is_file());
        assert_eq!(
            std::fs::read(receipt.rollback_root.join("sentinel")).expect("rollback sentinel"),
            b"rollback"
        );
        assert_eq!(
            discover_config_for_home(temp.path()).expect("discover migrated roots"),
            ConfigDiscovery::MigratedWithRollback
        );
        let parsed = validated_migration_receipt(&receipt.canonical_root, &receipt.rollback_root)
            .expect("validated receipt");
        assert_eq!(parsed.source, receipt.rollback_root);
        assert!(parsed.created_unix > 0);

        let repeated = migrate_isolated(temp.path()).expect("idempotent migration");
        assert_eq!(repeated, receipt);
    }

    #[test]
    fn offline_migration_refuses_profile_filename_collisions_without_overwriting_rollback() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old_hive = temp
            .path()
            .join(legacy::CONFIG_DIR_NAME)
            .join(legacy::HIVE_DIR_NAME);
        std::fs::create_dir_all(&old_hive).expect("legacy Hive dir");
        std::fs::write(old_hive.join(legacy::HIVE_SOUL_FILE_NAME), "old soul")
            .expect("legacy soul");
        std::fs::write(
            old_hive.join(crate::paths::HIVE_SOUL_FILE),
            "canonical soul",
        )
        .expect("colliding canonical soul");

        let error = migrate_isolated(temp.path()).expect_err("profile collision must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!temp.path().join(CONFIG_DIR_NAME).exists());
        assert_eq!(
            std::fs::read_to_string(old_hive.join(legacy::HIVE_SOUL_FILE_NAME))
                .expect("legacy soul preserved"),
            "old soul"
        );
        assert_eq!(
            std::fs::read_to_string(old_hive.join(crate::paths::HIVE_SOUL_FILE))
                .expect("canonical collision preserved"),
            "canonical soul"
        );
    }

    #[test]
    fn sqlite_copy_refuses_a_competing_writer_and_never_publishes_a_partial_copy() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source_path = temp.path().join(legacy::DATABASE_FILE_NAME);
        let destination_path = temp.path().join(DATABASE_FILE_NAME);
        let writer = rusqlite::Connection::open(&source_path).expect("source database");
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE sample (id INTEGER PRIMARY KEY);
                 INSERT INTO sample VALUES (1);
                 BEGIN IMMEDIATE;",
            )
            .expect("hold competing writer");

        let error = LegacySqliteCutover::acquire(&source_path)
            .err()
            .expect("competing writer must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("cutover fence"));
        assert!(!destination_path.exists());
        writer.execute_batch("ROLLBACK").expect("release writer");
    }

    #[test]
    fn strict_v2_receipts_reject_truncation_reordering_duplicates_and_malformed_fields() {
        let temp = tempfile::tempdir().expect("temp dir");
        let canonical = temp.path().join(CONFIG_DIR_NAME);
        let old = create_legacy_root(temp.path());
        std::fs::create_dir(&canonical).expect("canonical root");
        let valid = valid_empty_v2_receipt(&old);
        let lines: Vec<&str> = valid.trim_end_matches('\n').split('\n').collect();
        let uppercase_hash = valid.replacen("tree_sha256=", "tree_sha256=G", 1);
        let oversized = "x".repeat(MIGRATION_RECEIPT_MAX_BYTES as usize + 1);
        let invalid_receipts = vec![
            String::new(),
            valid.trim_end_matches('\n').to_string(),
            valid.replace("version=2", "version=1"),
            valid.replace(
                &format!("source={}", old.display()),
                "source=/definitely/not/the/rollback/root",
            ),
            valid.replace("created_unix=1", "created_unix=not-a-number"),
            valid.replace("rollback_preserved=true", "rollback_preserved=false"),
            format!("{valid}extra=forged\n"),
            valid.replace('\n', "\r\n"),
            format!(
                "{}\n{}\n{}\n{}\n{}\n",
                lines[1], lines[0], lines[2], lines[3], lines[4]
            ),
            format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n",
                lines[0], lines[1], lines[1], lines[2], lines[3], lines[4]
            ),
            valid.replace("sqlite=absent", "sqlite=bad-digest"),
            uppercase_hash,
            valid.replace("tree_stat_sha256=", "tree_stat_sha256=abc"),
            oversized,
        ];

        for contents in invalid_receipts {
            std::fs::write(canonical.join(MIGRATION_RECEIPT_FILE_NAME), contents)
                .expect("invalid receipt");
            assert_eq!(
                discover_config_for_home(temp.path()).expect("discover invalid receipt roots"),
                ConfigDiscovery::UnreconciledCoexistence
            );
        }

        std::fs::write(canonical.join(MIGRATION_RECEIPT_FILE_NAME), valid).expect("valid receipt");
        assert_eq!(
            discover_config_for_home(temp.path()).expect("discover valid receipt roots"),
            ConfigDiscovery::MigratedWithRollback
        );
    }

    #[test]
    fn failed_pre_publish_migration_removes_owned_staging_tree() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = temp.path().join(legacy::CONFIG_DIR_NAME);
        std::fs::create_dir_all(old.join(legacy::HIVE_DIR_NAME)).expect("deprecated Hive dir");
        std::fs::create_dir_all(old.join(HIVE_DIR_NAME)).expect("conflicting canonical Hive dir");

        let error = migrate_isolated(temp.path()).expect_err("migration must fail");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!temp.path().join(CONFIG_DIR_NAME).exists());
        assert!(!temp
            .path()
            .join(format!(".mitsuro.migrating.{}", std::process::id()))
            .exists());
        assert!(old.join(legacy::HIVE_DIR_NAME).is_dir());
        assert!(old.join(HIVE_DIR_NAME).is_dir());
    }

    #[test]
    fn quarantine_moves_only_failed_canonical_root() {
        let temp = tempfile::tempdir().expect("temp dir");
        let canonical = temp.path().join(CONFIG_DIR_NAME);
        let old = temp.path().join(legacy::CONFIG_DIR_NAME);
        std::fs::create_dir(&canonical).expect("canonical root");
        std::fs::create_dir(&old).expect("rollback root");
        std::fs::write(canonical.join("failed"), "state").expect("failed state");
        std::fs::write(old.join("rollback"), "state").expect("rollback state");

        let quarantined =
            quarantine_failed_config_root_for_home(temp.path()).expect("quarantine root");
        assert!(!canonical.exists());
        assert_eq!(
            std::fs::read_to_string(quarantined.join("failed")).expect("quarantined state"),
            "state"
        );
        assert_eq!(
            std::fs::read_to_string(old.join("rollback")).expect("rollback state"),
            "state"
        );
    }

    #[test]
    fn a_post_publication_failure_quarantines_canonical_state_and_keeps_rollback_authoritative() {
        let temp = tempfile::tempdir().expect("temp dir");
        let canonical = temp.path().join(CONFIG_DIR_NAME);
        let old = create_legacy_root(temp.path());
        std::fs::create_dir(&canonical).expect("canonical root");
        std::fs::write(canonical.join("published"), "canonical").expect("canonical state");
        std::fs::write(old.join("rollback"), "legacy").expect("rollback state");

        let error = quarantine_failed_published_cutover(
            temp.path(),
            io::Error::other("parent directory fsync failed"),
        );
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(error.to_string().contains("failed after publication"));
        assert!(error.to_string().contains("was quarantined"));
        assert!(!canonical.exists());
        assert_eq!(
            std::fs::read_to_string(old.join("rollback")).expect("rollback survives"),
            "legacy"
        );
        let quarantined = std::fs::read_dir(temp.path())
            .expect("home entries")
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mitsuro.failed.")
            })
            .expect("quarantined root")
            .path();
        assert_eq!(
            std::fs::read_to_string(quarantined.join("published"))
                .expect("quarantined canonical state"),
            "canonical"
        );
    }

    #[test]
    fn startup_rejects_a_receipt_when_the_required_rollback_root_is_missing() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let canonical = temp.path().join(CONFIG_DIR_NAME);
        std::fs::create_dir(&canonical).expect("canonical root");
        std::fs::write(
            canonical.join(MIGRATION_RECEIPT_FILE_NAME),
            valid_empty_v2_receipt(&old),
        )
        .expect("receipt");
        std::fs::remove_dir(&old).expect("remove rollback root");

        let error = require_startup_identity_for_home_with_process_probe(temp.path(), false)
            .expect_err("missing rollback authority must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("preserved rollback root"));
    }

    #[cfg(unix)]
    #[test]
    fn state_root_symlinks_are_rejected_by_discovery_migration_and_quarantine() {
        use std::os::unix::fs::symlink;

        let legacy_home = tempfile::tempdir().expect("legacy home");
        let legacy_target = legacy_home.path().join("legacy-target");
        std::fs::create_dir(&legacy_target).expect("legacy target");
        symlink(
            &legacy_target,
            legacy_home.path().join(legacy::CONFIG_DIR_NAME),
        )
        .expect("legacy root symlink");
        let discovery_error = discover_config_for_home(legacy_home.path())
            .expect_err("legacy root symlink must fail discovery");
        assert_eq!(discovery_error.kind(), io::ErrorKind::InvalidData);
        assert!(migrate_isolated(legacy_home.path()).is_err());

        let canonical_home = tempfile::tempdir().expect("canonical home");
        let canonical_target = canonical_home.path().join("canonical-target");
        std::fs::create_dir(&canonical_target).expect("canonical target");
        symlink(
            &canonical_target,
            canonical_home.path().join(CONFIG_DIR_NAME),
        )
        .expect("canonical root symlink");
        let canonical_error = discover_config_for_home(canonical_home.path())
            .expect_err("canonical root symlink must fail discovery");
        assert_eq!(canonical_error.kind(), io::ErrorKind::InvalidData);
        let quarantine_error = quarantine_failed_config_root_for_home(canonical_home.path())
            .expect_err("quarantine must not follow a canonical symlink");
        assert_eq!(quarantine_error.kind(), io::ErrorKind::InvalidData);
        assert!(
            canonical_target.is_dir(),
            "symlink target remains untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_or_line_breaking_source_paths_are_rejected_before_copy() {
        use std::os::unix::ffi::OsStrExt;

        for name in [
            std::ffi::OsStr::from_bytes(b"invalid-\xff"),
            std::ffi::OsStr::from_bytes(b"line\nbreak"),
        ] {
            let temp = tempfile::tempdir().expect("temp dir");
            let home = temp.path().join(name);
            std::fs::create_dir(&home).expect("test home");
            std::fs::create_dir(home.join(legacy::CONFIG_DIR_NAME)).expect("legacy root");
            let error = migrate_isolated(&home).expect_err("unrepresentable source must fail");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(!home.join(CONFIG_DIR_NAME).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn sqlite_authority_rejects_symlinks_orphans_and_rollback_journals() {
        use std::os::unix::fs::symlink;

        let symlinked_main_home = tempfile::tempdir().expect("symlinked main home");
        let old = create_legacy_root(symlinked_main_home.path());
        let outside = symlinked_main_home.path().join("outside.db");
        std::fs::write(&outside, "not sqlite").expect("outside file");
        symlink(&outside, old.join(legacy::DATABASE_FILE_NAME)).expect("main symlink");
        let error = migrate_isolated(symlinked_main_home.path())
            .expect_err("symlinked main database must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("not a regular file"));

        let symlinked_wal_home = tempfile::tempdir().expect("symlinked WAL home");
        let old = create_legacy_root(symlinked_wal_home.path());
        let db = rusqlite::Connection::open(old.join(legacy::DATABASE_FILE_NAME)).expect("db");
        db.execute_batch("CREATE TABLE sample (id INTEGER PRIMARY KEY);")
            .expect("schema");
        drop(db);
        let outside = symlinked_wal_home.path().join("outside.wal");
        std::fs::write(&outside, "not a WAL").expect("outside WAL");
        symlink(
            &outside,
            sqlite_sidecar_path(&old.join(legacy::DATABASE_FILE_NAME), "-wal"),
        )
        .expect("WAL symlink");
        let error =
            migrate_isolated(symlinked_wal_home.path()).expect_err("symlinked WAL must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("not a regular file"));

        for suffix in ["-wal", "-shm"] {
            let orphan_home = tempfile::tempdir().expect("orphan sidecar home");
            let old = create_legacy_root(orphan_home.path());
            std::fs::write(
                sqlite_sidecar_path(&old.join(legacy::DATABASE_FILE_NAME), suffix),
                "orphan",
            )
            .expect("orphan sidecar");
            let error =
                migrate_isolated(orphan_home.path()).expect_err("orphan sidecar must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("without the main database"));
        }

        let journal_home = tempfile::tempdir().expect("journal home");
        let old = create_legacy_root(journal_home.path());
        let db = rusqlite::Connection::open(old.join(legacy::DATABASE_FILE_NAME)).expect("db");
        db.execute_batch("CREATE TABLE sample (id INTEGER PRIMARY KEY);")
            .expect("schema");
        drop(db);
        std::fs::write(
            sqlite_sidecar_path(&old.join(legacy::DATABASE_FILE_NAME), "-journal"),
            "hot-or-stale-journal",
        )
        .expect("rollback journal");
        let error =
            migrate_isolated(journal_home.path()).expect_err("rollback journal must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("must be recovered"));
    }

    #[test]
    fn identity_rename_sources_must_have_the_expected_types() {
        let hive_home = tempfile::tempdir().expect("Hive type home");
        let old = create_legacy_root(hive_home.path());
        std::fs::write(old.join(legacy::HIVE_DIR_NAME), "not a directory")
            .expect("Hive source file");
        let error = migrate_isolated(hive_home.path()).expect_err("Hive source type must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("must be a directory"));

        let profile_home = tempfile::tempdir().expect("profile type home");
        let old = create_legacy_root(profile_home.path());
        let hive = old.join(legacy::HIVE_DIR_NAME);
        std::fs::create_dir(&hive).expect("Hive dir");
        std::fs::create_dir(hive.join(legacy::HIVE_MEMORY_FILE_NAME)).expect("profile directory");
        let error =
            migrate_isolated(profile_home.path()).expect_err("profile source type must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("must be a regular file"));

        let key_home = tempfile::tempdir().expect("IPC key type home");
        let old = create_legacy_root(key_home.path());
        let run = old.join("run");
        std::fs::create_dir(&run).expect("run dir");
        std::fs::create_dir(run.join(legacy::HIVE_KEY_FILE_NAME)).expect("IPC key directory");
        let error = migrate_isolated(key_home.path()).expect_err("IPC key type must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("must be a regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn migration_omits_only_named_ephemera_and_preserves_durable_lock_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let logs = old.join("logs");
        let plugins = old.join("plugins");
        let tokens = old.join("tokens");
        let run = old.join("run");
        for directory in [&logs, &plugins, &tokens, &run] {
            std::fs::create_dir(directory).expect("legacy subdirectory");
        }
        std::fs::write(logs.join("runtime.log"), "runtime").expect("log");
        std::fs::write(old.join("krusty.log"), "terminal runtime").expect("root terminal log");
        std::fs::write(old.join("krusty-desktop-server.log"), "desktop runtime")
            .expect("root desktop log");
        std::fs::write(old.join("server.pid"), "999999999").expect("stale pid");
        std::fs::write(plugins.join(".mutation.lock"), "runtime").expect("plugin lock");
        std::fs::write(tokens.join("oauth.lock"), "runtime").expect("OAuth lock");
        std::fs::write(tokens.join("vapid_key.pem.lock"), "runtime").expect("VAPID lock");
        std::fs::write(tokens.join("oauth.openai.refresh.lock"), "runtime").expect("refresh lock");
        std::fs::write(tokens.join("credentials.lock"), "durable").expect("durable arbitrary lock");
        std::fs::write(run.join(legacy::HIVE_KEY_FILE_NAME), "ipc-secret").expect("legacy IPC key");
        let socket = run.join(legacy::HIVE_SOCKET_FILE_NAME);
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("legacy socket");
        drop(listener);

        let receipt = migrate_isolated(temp.path()).expect("migrate state");
        assert!(!receipt.canonical_root.join("logs").exists());
        assert!(!receipt.canonical_root.join("krusty.log").exists());
        assert!(!receipt
            .canonical_root
            .join("krusty-desktop-server.log")
            .exists());
        assert!(!receipt.canonical_root.join("server.pid").exists());
        assert!(!receipt
            .canonical_root
            .join("plugins/.mutation.lock")
            .exists());
        assert!(!receipt.canonical_root.join("tokens/oauth.lock").exists());
        assert!(!receipt
            .canonical_root
            .join("tokens/vapid_key.pem.lock")
            .exists());
        assert!(!receipt
            .canonical_root
            .join("tokens/oauth.openai.refresh.lock")
            .exists());
        assert_eq!(
            std::fs::read_to_string(receipt.canonical_root.join("tokens/credentials.lock"))
                .expect("durable arbitrary lock"),
            "durable"
        );
        assert_eq!(
            std::fs::read_to_string(receipt.canonical_root.join("run").join(HIVE_KEY_FILE_NAME))
                .expect("canonical IPC key"),
            "ipc-secret"
        );
        assert!(!receipt
            .canonical_root
            .join("run")
            .join(legacy::HIVE_KEY_FILE_NAME)
            .exists());
        assert!(socket.exists(), "rollback runtime socket remains untouched");
        assert!(old.join("logs/runtime.log").is_file());
        assert!(old.join("krusty.log").is_file());
        assert!(old.join("krusty-desktop-server.log").is_file());
        assert!(old.join("tokens/oauth.lock").is_file());
    }

    #[test]
    fn startup_and_idempotent_migration_reject_mutated_preserved_durable_state() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let tokens = old.join("tokens");
        std::fs::create_dir(&tokens).expect("tokens");
        std::fs::write(tokens.join("credentials.json"), "before").expect("credentials");
        migrate_isolated(temp.path()).expect("migration");
        std::fs::write(tokens.join("credentials.json"), "after").expect("mutated credentials");

        let startup_error =
            require_startup_identity_for_home_with_process_probe(temp.path(), false)
                .expect_err("mutated durable state must fail startup");
        assert_eq!(startup_error.kind(), io::ErrorKind::InvalidData);
        assert!(startup_error.to_string().contains("durable state diverged"));
        let migration_error =
            migrate_isolated(temp.path()).expect_err("idempotent migration must verify rollback");
        assert_eq!(migration_error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn startup_rejects_a_mutated_preserved_hive_document() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let hive = old.join(legacy::HIVE_DIR_NAME);
        std::fs::create_dir(&hive).expect("Hive directory");
        let memory = hive.join(legacy::HIVE_MEMORY_FILE_NAME);
        std::fs::write(&memory, "before").expect("Hive memory");
        migrate_isolated(temp.path()).expect("migration");
        std::fs::write(&memory, "after").expect("mutated Hive memory");

        let error = require_startup_identity_for_home_with_process_probe(temp.path(), false)
            .expect_err("mutated Hive document must fail startup");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("durable state diverged"));
    }

    #[test]
    fn excluded_runtime_artifact_changes_do_not_invalidate_preserved_authority() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let logs = old.join("logs");
        let backups = old.join("backups");
        let tokens = old.join("tokens");
        std::fs::create_dir(&logs).expect("logs");
        std::fs::create_dir(&backups).expect("backups");
        std::fs::create_dir(&tokens).expect("tokens");
        std::fs::write(logs.join("before.log"), "before").expect("old log");
        std::fs::write(backups.join("krusty-before.db"), "before").expect("old recovery snapshot");
        std::fs::write(old.join("krusty.log"), "before").expect("root terminal log");
        std::fs::write(old.join("krusty-desktop-server.log"), "before").expect("root desktop log");
        std::fs::write(tokens.join("oauth.lock"), "before").expect("old lock");
        let receipt = migrate_isolated(temp.path()).expect("migration");

        std::fs::write(logs.join("after.log"), "after").expect("new log");
        std::fs::write(backups.join("krusty-after.db"), "after").expect("new recovery snapshot");
        std::fs::write(old.join("krusty.log"), "after").expect("changed root terminal log");
        std::fs::write(old.join("krusty-desktop-server.log"), "after")
            .expect("changed root desktop log");
        std::fs::write(tokens.join("oauth.lock"), "after").expect("changed lock");
        assert_eq!(
            require_startup_identity_for_home_with_process_probe(temp.path(), false)
                .expect("excluded ephemera are not authority"),
            ConfigDiscovery::MigratedWithRollback
        );
        assert_eq!(
            migrate_isolated(temp.path()).expect("idempotent migration"),
            receipt
        );
    }

    #[test]
    fn sqlite_content_divergence_invalidates_preserved_authority() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let database = old.join(legacy::DATABASE_FILE_NAME);
        let db = rusqlite::Connection::open(&database).expect("legacy database");
        db.execute_batch(
            "CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT);\n\
             INSERT INTO sample VALUES (1, 'before');",
        )
        .expect("seed database");
        drop(db);
        migrate_isolated(temp.path()).expect("migration");

        let db = rusqlite::Connection::open(&database).expect("legacy database");
        db.execute("INSERT INTO sample VALUES (2, 'after')", [])
            .expect("mutate rollback database");
        drop(db);
        let error = require_startup_identity_for_home_with_process_probe(temp.path(), false)
            .expect_err("SQLite divergence must fail startup");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("SQLite authority diverged"));
    }

    #[test]
    fn equivalent_sqlite_checkpoint_layout_is_accepted_after_full_content_verification() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let database = old.join(legacy::DATABASE_FILE_NAME);
        let db = rusqlite::Connection::open(&database).expect("legacy database");
        db.execute_batch(
            "PRAGMA journal_mode=WAL;\n\
             PRAGMA wal_autocheckpoint=0;\n\
             CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT);\n\
             INSERT INTO sample VALUES (1, 'stable');",
        )
        .expect("seed WAL database");
        let before = sqlite_source_stat(&database)
            .expect("source stat")
            .expect("database stat");
        migrate_isolated(temp.path()).expect("migration");
        db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_row| Ok(()))
            .expect("checkpoint rollback database");
        drop(db);
        let after = sqlite_source_stat(&database)
            .expect("source stat")
            .expect("database stat");
        assert_ne!(before, after, "checkpoint must alter the cheap stat/layout");

        assert_eq!(
            require_startup_identity_for_home_with_process_probe(temp.path(), false)
                .expect("identical logical database is accepted"),
            ConfigDiscovery::MigratedWithRollback
        );
    }

    #[test]
    fn cache_socket_paths_match_both_macos_identity_generations() {
        let cache = Path::new("/cache-root");
        assert_eq!(
            cache_runtime_socket_at(cache, false),
            cache
                .join("mitsuro")
                .join("run")
                .join(HIVE_SOCKET_FILE_NAME)
        );
        assert_eq!(
            cache_runtime_socket_at(cache, true),
            cache
                .join(legacy::CLI_NAME)
                .join("run")
                .join(legacy::HIVE_SOCKET_FILE_NAME)
        );
    }

    #[test]
    fn startup_refuses_unreconciled_roots() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(temp.path().join(CONFIG_DIR_NAME)).expect("canonical root");
        std::fs::create_dir(temp.path().join(legacy::CONFIG_DIR_NAME)).expect("legacy root");
        let error = require_startup_identity_for_home(temp.path()).expect_err("must refuse");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn legacy_only_startup_names_both_canonical_and_compatibility_migration_commands() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(temp.path().join(legacy::CONFIG_DIR_NAME)).expect("legacy root");

        let error = require_startup_identity_for_home(temp.path()).expect_err("must migrate");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        let guidance = error.to_string();
        assert!(guidance.contains("`mitsuro migrate-identity --confirm-offline`"));
        assert!(guidance.contains("`krusty migrate-identity --confirm-offline`"));
        assert!(guidance.contains("install the canonical `mitsuro` command"));
    }

    #[cfg(unix)]
    #[test]
    fn migration_refuses_a_running_legacy_generation() {
        let temp = tempfile::tempdir().expect("temp dir");
        let run = temp.path().join(legacy::CONFIG_DIR_NAME).join("run");
        std::fs::create_dir_all(&run).expect("run dir");
        let _listener =
            std::os::unix::net::UnixListener::bind(run.join(legacy::HIVE_SOCKET_FILE_NAME))
                .expect("socket");
        let error = migrate_isolated(temp.path()).expect_err("must refuse");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!temp.path().join(CONFIG_DIR_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn migration_skips_only_a_disconnected_legacy_runtime_socket() {
        let temp = tempfile::tempdir().expect("temp dir");
        let old = temp.path().join(legacy::CONFIG_DIR_NAME);
        let run = old.join("run");
        std::fs::create_dir_all(&run).expect("run dir");
        let old_socket = run.join(legacy::HIVE_SOCKET_FILE_NAME);
        let listener = std::os::unix::net::UnixListener::bind(&old_socket).expect("legacy socket");
        drop(listener);

        let receipt = migrate_isolated(temp.path()).expect("migrate around stale socket");
        assert!(old_socket.exists(), "rollback socket remains untouched");
        assert!(receipt.canonical_root.join("run").is_dir());
        assert!(!receipt
            .canonical_root
            .join("run")
            .join(legacy::HIVE_SOCKET_FILE_NAME)
            .exists());
        assert!(!receipt
            .canonical_root
            .join("run")
            .join(HIVE_SOCKET_FILE_NAME)
            .exists());
    }

    #[cfg(unix)]
    #[test]
    fn migration_refuses_non_runtime_special_files_without_touching_rollback() {
        use std::os::unix::ffi::OsStrExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let old = temp.path().join(legacy::CONFIG_DIR_NAME);
        let data = old.join("data");
        std::fs::create_dir_all(&data).expect("data dir");
        let fifo = data.join("events.fifo");
        let fifo_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).expect("fifo path");
        let result = unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());

        let error = migrate_isolated(temp.path()).expect_err("special file must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("special durable-state entry"));
        assert!(fifo.exists(), "rollback FIFO remains untouched");
        assert!(!temp.path().join(CONFIG_DIR_NAME).exists());
        assert!(!temp
            .path()
            .join(format!(".mitsuro.migrating.{}", std::process::id()))
            .exists());
    }

    #[cfg(target_os = "linux")]
    fn seed_fake_linux_process(proc_root: &Path, pid: u32, uid: u32, executable: &Path) -> PathBuf {
        use std::os::unix::fs::symlink;

        let process = proc_root.join(pid.to_string());
        std::fs::create_dir_all(process.join("fd")).expect("fake process directories");
        std::fs::write(
            process.join("status"),
            format!("Name:\ttest\nState:\tS (sleeping)\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
        )
        .expect("fake status");
        symlink(executable, process.join("exe")).expect("fake executable link");
        process
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn procfs_quiescence_separates_offline_cutover_from_steady_state_policy() {
        for executable in ["/opt/archive/krusty", "/opt/current/mitsuro-hive"] {
            let temp = tempfile::tempdir().expect("temp dir");
            let old = create_legacy_root(temp.path());
            let proc_root = temp.path().join("proc");
            std::fs::create_dir(&proc_root).expect("proc root");
            seed_fake_linux_process(&proc_root, 101, 4242, Path::new(executable));

            let error = inspect_linux_legacy_processes(
                &proc_root,
                &old,
                4242,
                999,
                ProcessQuiescencePolicy::OfflineCutover,
            )
            .expect_err("every Mitsuro generation must block offline cutover");
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            assert!(error.to_string().contains("PID 101"));
        }

        let canonical_temp = tempfile::tempdir().expect("canonical temp dir");
        let canonical_old = create_legacy_root(canonical_temp.path());
        let canonical_proc = canonical_temp.path().join("proc");
        std::fs::create_dir(&canonical_proc).expect("proc root");
        seed_fake_linux_process(
            &canonical_proc,
            101,
            4242,
            Path::new("/opt/current/mitsuro-hive"),
        );
        inspect_linux_legacy_processes(
            &canonical_proc,
            &canonical_old,
            4242,
            999,
            ProcessQuiescencePolicy::PreservedLegacyAuthority,
        )
        .expect("canonical processes are valid during steady-state receipt verification");

        let legacy_temp = tempfile::tempdir().expect("legacy temp dir");
        let legacy_old = create_legacy_root(legacy_temp.path());
        let legacy_proc = legacy_temp.path().join("proc");
        std::fs::create_dir(&legacy_proc).expect("proc root");
        seed_fake_linux_process(&legacy_proc, 101, 4242, Path::new("/opt/archive/krusty"));
        let error = inspect_linux_legacy_processes(
            &legacy_proc,
            &legacy_old,
            4242,
            999,
            ProcessQuiescencePolicy::PreservedLegacyAuthority,
        )
        .expect_err("previous-generation process must block canonical steady state");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);

        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let proc_root = temp.path().join("proc");
        std::fs::create_dir(&proc_root).expect("proc root");
        seed_fake_linux_process(
            &proc_root,
            102,
            4242,
            Path::new("/opt/tools/mako-analysis-helper"),
        );
        inspect_linux_legacy_processes(
            &proc_root,
            &old,
            4242,
            999,
            ProcessQuiescencePolicy::OfflineCutover,
        )
        .expect("unrelated substring must not block cutover");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn procfs_quiescence_detects_any_same_user_process_holding_legacy_sqlite() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temp dir");
        let old = create_legacy_root(temp.path());
        let database = old.join(legacy::DATABASE_FILE_NAME);
        std::fs::write(&database, "database handle sentinel").expect("database sentinel");
        let proc_root = temp.path().join("proc");
        std::fs::create_dir(&proc_root).expect("proc root");
        let process = seed_fake_linux_process(
            &proc_root,
            103,
            4242,
            Path::new("/usr/bin/unrelated-editor"),
        );
        symlink(&database, process.join("fd/7")).expect("legacy database fd");

        let error = inspect_linux_legacy_processes(
            &proc_root,
            &old,
            4242,
            999,
            ProcessQuiescencePolicy::PreservedLegacyAuthority,
        )
        .expect_err("open legacy database descriptor must block steady state");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("previous Mitsuro state"));
    }

    #[test]
    fn executable_identity_matches_only_explicit_generation_names_and_release_roots() {
        for path in [
            "/opt/krusty",
            "/opt/krusty-mako",
            "/opt/mitsuro",
            "/opt/mitsuro-hive",
            "/srv/.krusty-releases/0.9.20/random-name",
            "/srv/.mitsuro-releases/0.9.21/random-name",
        ] {
            assert!(is_any_mitsuro_executable(Path::new(path)), "{path}");
        }
        for path in [
            "/opt/mako-analysis-helper",
            "/opt/krusty-helper",
            "/srv/not-.krusty-releases/random-name",
            "/opt/mitsurod",
        ] {
            assert!(!is_any_mitsuro_executable(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn pid_file_generation_matching_never_treats_a_canonical_pid_as_legacy() {
        assert!(executable_matches_generation(
            Path::new("/opt/krusty"),
            true
        ));
        assert!(!executable_matches_generation(
            Path::new("/opt/mitsuro"),
            true
        ));
        assert!(executable_matches_generation(
            Path::new("/opt/mitsuro"),
            false
        ));
        assert!(!executable_matches_generation(
            Path::new("/opt/krusty"),
            false
        ));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_socket_probe_matches_xdg_generation_layouts() {
        let temp = tempfile::tempdir().expect("runtime dir");
        let old_parent = temp.path().join(legacy::CLI_NAME);
        std::fs::create_dir(&old_parent).expect("deprecated runtime parent");
        let _old_listener =
            std::os::unix::net::UnixListener::bind(old_parent.join(legacy::HIVE_SOCKET_FILE_NAME))
                .expect("deprecated runtime socket");
        assert!(runtime_socket_is_live_at(temp.path(), true));

        let canonical_parent = temp.path().join("mitsuro");
        std::fs::create_dir(&canonical_parent).expect("canonical runtime parent");
        let _canonical_listener =
            std::os::unix::net::UnixListener::bind(canonical_parent.join(HIVE_SOCKET_FILE_NAME))
                .expect("canonical runtime socket");
        assert!(runtime_socket_is_live_at(temp.path(), false));
    }
}
