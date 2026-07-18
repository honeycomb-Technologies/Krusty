use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::{cmp::Ordering, io};

use anyhow::{anyhow, Context, Result};
use semver::Version;
use tracing::{debug, info, warn};

use super::paths::{pending_update_path, pending_version_path, update_marker_path};
use super::VERSION;

const MAX_PENDING_VERSION_BYTES: usize = 128;
const RELEASE_MARKER_PREFIX: &str = "release:";
const DEV_MARKER_PREFIX: &str = "dev:";

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingUpdateVersion {
    Release(Version),
    DevRevision(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseUpdateDisposition {
    Newer,
    Equal,
    Older,
}

impl PendingUpdateVersion {
    fn display_value(&self) -> String {
        match self {
            Self::Release(version) => version.to_string(),
            Self::DevRevision(revision) => revision.clone(),
        }
    }

    fn marker_value(&self) -> String {
        match self {
            Self::Release(version) => format!("{}{}", RELEASE_MARKER_PREFIX, version),
            Self::DevRevision(revision) => format!("{}{}", DEV_MARKER_PREFIX, revision),
        }
    }
}

pub fn apply_pending_update() -> Result<Option<String>> {
    let pending = pending_update_path();
    let version_path = pending_version_path();

    if !pending.exists() {
        if version_path.exists() {
            discard_pending_update(&pending, &version_path)
                .context("failed to discard orphaned pending update marker")?;
        }

        #[cfg(windows)]
        if let Ok(current_exe) = std::env::current_exe() {
            if let Err(error) = reconcile_replacement_artifacts(&current_exe) {
                warn!("Failed to clean stale updater artifacts: {}", error);
            }
        }

        return Ok(None);
    }

    if let Err(policy_error) = super::policy::require_safe_single_binary_update() {
        return match discard_pending_update(&pending, &version_path) {
            Ok(()) => Err(anyhow!(
                "{} The unsupported pending single-binary update was discarded.",
                policy_error
            )),
            Err(cleanup_error) => Err(anyhow!(
                "{} Failed to discard the unsupported pending update: {}",
                policy_error,
                cleanup_error
            )),
        };
    }

    let current_exe = std::env::current_exe().context("failed to locate current executable")?;
    if let Err(error) = reconcile_replacement_artifacts(&current_exe) {
        return Err(error_after_discarding_pending(
            error.context("failed to reconcile updater replacement state"),
            &pending,
            &version_path,
        ));
    }

    let version = require_pending_update_version(&pending, &version_path)?;
    if let PendingUpdateVersion::Release(pending_version) = &version {
        let current_version = Version::parse(VERSION)
            .with_context(|| format!("running package version '{}' is invalid", VERSION))?;
        match release_update_disposition(pending_version, &current_version) {
            ReleaseUpdateDisposition::Newer => {}
            disposition @ (ReleaseUpdateDisposition::Equal | ReleaseUpdateDisposition::Older) => {
                discard_pending_update(&pending, &version_path)
                    .context("failed to discard stale pending release update")?;
                info!(
                    "Discarded {:?} pending release v{} (running v{})",
                    disposition, pending_version, current_version
                );
                return Ok(None);
            }
        }
    }

    validate_pending_update_or_discard(&pending, &version_path)?;
    info!("Found pending update at: {}", pending.display());

    info!("Current binary: {}", current_exe.display());

    if let Err(error) = replace_current_exe(&pending, &current_exe) {
        return Err(error_after_discarding_pending(
            error.context("failed to replace current executable"),
            &pending,
            &version_path,
        ));
    }

    if let Err(error) = discard_pending_update(&pending, &version_path) {
        warn!(
            "Failed to clean applied pending update artifacts: {}",
            error
        );
    }

    let version = version.display_value();
    let _ = fs::create_dir_all(crate::paths::config_dir());
    let _ = fs::write(update_marker_path(), &version);

    Ok(Some(version))
}

fn release_update_disposition(pending: &Version, current: &Version) -> ReleaseUpdateDisposition {
    match pending.cmp_precedence(current) {
        Ordering::Greater => ReleaseUpdateDisposition::Newer,
        Ordering::Equal => ReleaseUpdateDisposition::Equal,
        Ordering::Less => ReleaseUpdateDisposition::Older,
    }
}

fn error_after_discarding_pending(
    primary: anyhow::Error,
    pending: &Path,
    version: &Path,
) -> anyhow::Error {
    match discard_pending_update(pending, version) {
        Ok(()) => anyhow!("{}. The pending update was discarded.", primary),
        Err(cleanup_error) => anyhow!(
            "{}. Failed to discard the pending update: {}",
            primary,
            cleanup_error
        ),
    }
}

fn validate_pending_update_or_discard(pending: &Path, version: &Path) -> Result<()> {
    validate_pending_update(pending).map_err(|error| {
        error_after_discarding_pending(
            error.context("pending update payload validation failed"),
            pending,
            version,
        )
    })
}

fn replace_current_exe(pending: &Path, current_exe: &Path) -> Result<()> {
    replace_current_exe_with(pending, current_exe, platform_replace_current_exe)
}

fn replace_current_exe_with<F>(pending: &Path, current_exe: &Path, replace: F) -> Result<()>
where
    F: FnOnce(&Path, &Path, &Path) -> std::io::Result<()>,
{
    let backup = backup_path(current_exe);
    let staged = current_exe.with_extension("new");

    reconcile_replacement_paths(current_exe, &backup, &staged)?;
    debug!("Staging pending update at: {}", staged.display());
    stage_pending_update(pending, &staged)?;

    debug!(
        "Atomically replacing executable at: {}",
        current_exe.display()
    );
    if let Err(error) = replace(&staged, current_exe, &backup) {
        return match recover_failed_replacement(current_exe, &backup, &staged) {
            Ok(()) => Err(error)
                .context("executable replacement failed; a runnable executable was preserved"),
            Err(recovery_error) => Err(error).with_context(|| {
                format!(
                    "executable replacement failed and recovery also failed: {}",
                    recovery_error
                )
            }),
        };
    }

    ensure_regular_file(current_exe, "installed executable")?;
    remove_regular_file_if_exists(&staged, "staged executable")?;

    // Keep the known-good backup until the updated executable starts once. The
    // next startup reconciles it after proving the replacement is runnable.
    info!(
        "Update applied successfully; rollback retained at {} until next launch",
        backup.display()
    );
    Ok(())
}

fn stage_pending_update(pending: &Path, staged: &Path) -> Result<()> {
    let result = (|| -> Result<()> {
        let mut source = File::open(pending).context("failed to open pending update")?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staged)
            .with_context(|| format!("failed to create staged update at {}", staged.display()))?;
        std::io::copy(&mut source, &mut output).context("failed to copy pending update")?;
        output.sync_all().context("failed to sync staged update")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = output.metadata()?.permissions();
            perms.set_mode(0o755);
            output.set_permissions(perms)?;
        }

        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

fn reconcile_replacement_artifacts(current_exe: &Path) -> Result<()> {
    reconcile_replacement_paths(
        current_exe,
        &backup_path(current_exe),
        &current_exe.with_extension("new"),
    )
}

fn reconcile_replacement_paths(current: &Path, backup: &Path, staged: &Path) -> Result<()> {
    if !regular_file_exists(current, "current executable")? {
        if regular_file_exists(backup, "rollback executable")? {
            fs::rename(backup, current).with_context(|| {
                format!(
                    "failed to restore rollback executable {} to {}",
                    backup.display(),
                    current.display()
                )
            })?;
            info!(
                "Restored interrupted updater rollback at {}",
                current.display()
            );
        } else {
            return Err(anyhow!(
                "current executable {} is missing and no rollback exists",
                current.display()
            ));
        }
    }

    remove_regular_file_if_exists(staged, "stale staged executable")?;
    remove_regular_file_if_exists(backup, "stale rollback executable")?;
    Ok(())
}

fn recover_failed_replacement(current: &Path, backup: &Path, staged: &Path) -> Result<()> {
    if regular_file_exists(current, "current executable")? {
        remove_regular_file_if_exists(staged, "failed staged executable")?;
        return Ok(());
    }

    if regular_file_exists(backup, "rollback executable")? {
        fs::rename(backup, current).with_context(|| {
            format!(
                "failed to restore rollback executable {} to {}",
                backup.display(),
                current.display()
            )
        })?;
        remove_regular_file_if_exists(staged, "failed staged executable")?;
        return Ok(());
    }

    if regular_file_exists(staged, "staged executable")? {
        fs::rename(staged, current).with_context(|| {
            format!(
                "failed to preserve staged executable {} at {}",
                staged.display(),
                current.display()
            )
        })?;
        return Ok(());
    }

    Err(anyhow!(
        "no current, rollback, or staged executable remains"
    ))
}

fn regular_file_exists(path: &Path, description: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(anyhow!(
            "{} {} is not a regular file",
            description,
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect {} {}", description, path.display())),
    }
}

fn ensure_regular_file(path: &Path, description: &str) -> Result<()> {
    if regular_file_exists(path, description)? {
        Ok(())
    } else {
        Err(anyhow!("{} {} is missing", description, path.display()))
    }
}

fn remove_regular_file_if_exists(path: &Path, description: &str) -> Result<()> {
    if regular_file_exists(path, description)? {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove {} {}", description, path.display()))?;
    }
    Ok(())
}

#[cfg(windows)]
fn platform_replace_current_exe(
    staged: &Path,
    current: &Path,
    backup: &Path,
) -> std::io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "ReplaceFileW"]
        fn replace_file_w(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let current = wide_path(current);
    let staged = wide_path(staged);
    let backup = wide_path(backup);
    let replaced = unsafe {
        replace_file_w(
            current.as_ptr(),
            staged.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn platform_replace_current_exe(
    staged: &Path,
    current: &Path,
    backup: &Path,
) -> std::io::Result<()> {
    fs::rename(current, backup)?;
    fs::rename(staged, current)
}

#[cfg(windows)]
fn backup_path(current_exe: &Path) -> PathBuf {
    current_exe.with_extension("exe.old")
}

#[cfg(not(windows))]
fn backup_path(current_exe: &Path) -> PathBuf {
    current_exe.with_extension("old")
}

fn validate_pending_update(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).context("failed to inspect pending update")?;

    if !metadata.file_type().is_file() {
        return Err(anyhow!("pending update is not a regular file"));
    }

    #[cfg(unix)]
    validate_pending_update_unix(path, &metadata)?;

    Ok(())
}

pub(super) fn discard_pending_update(pending: &Path, version: &Path) -> Result<()> {
    let mut failures = Vec::new();

    for path in [pending, version] {
        if let Err(error) = remove_or_quarantine_pending_artifact(path) {
            failures.push(format!("{}: {}", path.display(), error));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(failures.join("; ")))
    }
}

fn remove_or_quarantine_pending_artifact(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect pending artifact {}", path.display()));
        }
    };

    if !metadata.file_type().is_dir() {
        return fs::remove_file(path)
            .with_context(|| format!("failed to remove pending artifact {}", path.display()));
    }

    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("pending-update");
            let quarantine =
                path.with_file_name(format!("{}.rejected-{}", file_name, uuid::Uuid::new_v4()));
            fs::rename(path, &quarantine).with_context(|| {
                format!(
                    "failed to quarantine invalid pending directory {} at {}",
                    path.display(),
                    quarantine.display()
                )
            })?;
            warn!(
                "Quarantined invalid pending updater directory at {}",
                quarantine.display()
            );
            Ok(())
        }
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove pending directory {}", path.display())),
    }
}

pub(super) fn write_pending_release_marker(version: &str) -> Result<()> {
    let version = Version::parse(version)
        .with_context(|| format!("release version '{}' is not valid semver", version))?;
    write_pending_version_marker_at(
        &pending_version_path(),
        &PendingUpdateVersion::Release(version),
    )
}

pub(super) fn write_pending_dev_marker(revision: &str) -> Result<()> {
    validate_dev_revision(revision)?;
    write_pending_version_marker_at(
        &pending_version_path(),
        &PendingUpdateVersion::DevRevision(revision.to_string()),
    )
}

fn write_pending_version_marker_at(path: &Path, version: &PendingUpdateVersion) -> Result<()> {
    if let PendingUpdateVersion::DevRevision(revision) = version {
        validate_dev_revision(revision)?;
    }
    let marker = version.marker_value();
    if marker.len() > MAX_PENDING_VERSION_BYTES {
        return Err(anyhow!("pending update marker has an invalid length"));
    }
    let staged = path.with_extension("version.tmp");
    let _ = fs::remove_file(&staged);

    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .with_context(|| format!("failed to stage update marker at {}", staged.display()))?;
        file.write_all(marker.as_bytes())
            .context("failed to write update marker")?;
        file.sync_all().context("failed to sync update marker")?;
        fs::rename(&staged, path)
            .with_context(|| format!("failed to publish update marker at {}", path.display()))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn require_pending_update_version(
    pending: &Path,
    version_path: &Path,
) -> Result<PendingUpdateVersion> {
    match read_pending_version_marker(version_path) {
        Ok(version) => Ok(version),
        Err(marker_error) => match discard_pending_update(pending, version_path) {
            Ok(()) => Err(anyhow!(
                "Pending update is incomplete or invalid and was discarded: {}",
                marker_error
            )),
            Err(cleanup_error) => Err(anyhow!(
                "Pending update is incomplete or invalid: {}. Failed to discard it: {}",
                marker_error,
                cleanup_error
            )),
        },
    }
}

fn read_pending_version_marker(path: &Path) -> Result<PendingUpdateVersion> {
    let mut file = File::open(path)
        .with_context(|| format!("pending update marker is missing at {}", path.display()))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_PENDING_VERSION_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("failed to read pending update marker")?;
    if bytes.len() > MAX_PENDING_VERSION_BYTES {
        return Err(anyhow!(
            "pending update marker exceeds {} bytes",
            MAX_PENDING_VERSION_BYTES
        ));
    }
    let marker = String::from_utf8(bytes).context("pending update marker is not UTF-8")?;
    parse_pending_version_marker(&marker)
}

fn parse_pending_version_marker(marker: &str) -> Result<PendingUpdateVersion> {
    if marker.is_empty() || marker.len() > MAX_PENDING_VERSION_BYTES || !marker.is_ascii() {
        return Err(anyhow!("pending update marker has an invalid encoding"));
    }

    if let Some(version) = marker.strip_prefix(RELEASE_MARKER_PREFIX) {
        return Version::parse(version)
            .map(PendingUpdateVersion::Release)
            .context("pending release marker is not valid semver");
    }
    if let Some(revision) = marker.strip_prefix(DEV_MARKER_PREFIX) {
        validate_dev_revision(revision)?;
        return Ok(PendingUpdateVersion::DevRevision(revision.to_string()));
    }

    // Accept legacy release markers written before markers were explicitly typed.
    Version::parse(marker)
        .map(PendingUpdateVersion::Release)
        .context("pending update marker has no recognized type")
}

fn validate_dev_revision(revision: &str) -> Result<()> {
    if !(7..=64).contains(&revision.len()) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(anyhow!(
            "pending development revision must be a 7-64 character hexadecimal object ID"
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_pending_update_unix(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let current_uid = unsafe { libc::geteuid() };
    if metadata.uid() != current_uid {
        return Err(anyhow!(
            "pending update is owned by uid {}, expected {}",
            metadata.uid(),
            current_uid
        ));
    }

    let mode = metadata.permissions().mode();
    if mode & 0o022 != 0 {
        return Err(anyhow!(
            "pending update permissions are too broad: {:o}",
            mode & 0o777
        ));
    }

    if let Some(parent) = path.parent() {
        let parent_metadata = fs::symlink_metadata(parent)
            .with_context(|| format!("failed to inspect update directory {}", parent.display()))?;
        if !parent_metadata.file_type().is_dir() {
            return Err(anyhow!("pending update directory is not a directory"));
        }
        if parent_metadata.uid() != current_uid {
            return Err(anyhow!(
                "pending update directory is owned by uid {}, expected {}",
                parent_metadata.uid(),
                current_uid
            ));
        }
        let parent_mode = parent_metadata.permissions().mode();
        if parent_mode & 0o022 != 0 {
            return Err(anyhow!(
                "pending update directory permissions are too broad: {:o}",
                parent_mode & 0o777
            ));
        }
    }

    Ok(())
}

pub fn read_update_marker() -> Option<String> {
    let path = update_marker_path();
    let version = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    let trimmed = version.trim().to_string();
    (!trimmed.is_empty() && trimmed != "latest").then_some(trimmed)
}

pub fn cleanup_pending_update() {
    let pending = pending_update_path();
    let version = pending_version_path();
    if pending.exists() || version.exists() {
        match discard_pending_update(&pending, &version) {
            Ok(()) => info!("Cleaned up pending update"),
            Err(error) => warn!("Failed to clean up pending update: {}", error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_updates_use_private_config_directory() {
        let pending = pending_update_path();
        assert!(pending.starts_with(crate::paths::config_dir()));
        assert!(!pending.starts_with(std::env::temp_dir()));
        assert_eq!(
            pending.file_name().and_then(|name| name.to_str()),
            Some("krusty-pending-update")
        );
    }

    #[test]
    fn rejects_non_regular_pending_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pending_dir = dir.path().join("pending-dir");
        fs::create_dir(&pending_dir).expect("create dir");

        let err = validate_pending_update(&pending_dir).expect_err("directory should be rejected");
        assert!(err.to_string().contains("not a regular file"));
    }

    #[test]
    fn discards_pending_binary_and_version_marker_together() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pending = dir.path().join("pending");
        let version = dir.path().join("pending.version");
        fs::write(&pending, b"binary").expect("write pending");
        fs::write(&version, b"1.2.3").expect("write version");

        discard_pending_update(&pending, &version).expect("discard pending update");

        assert!(!pending.exists());
        assert!(!version.exists());
        discard_pending_update(&pending, &version).expect("discard remains idempotent");
    }

    #[test]
    fn incomplete_or_invalid_pending_update_is_discarded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pending = dir.path().join("pending");
        let version = dir.path().join("pending.version");

        fs::write(&pending, b"binary").expect("write pending");
        let missing_error = require_pending_update_version(&pending, &version)
            .expect_err("missing ready marker must fail");
        assert!(missing_error.to_string().contains("was discarded"));
        assert!(!pending.exists());

        fs::write(&pending, b"binary").expect("rewrite pending");
        fs::write(&version, b"invalid version\n").expect("write invalid marker");
        let invalid_error = require_pending_update_version(&pending, &version)
            .expect_err("invalid ready marker must fail");
        assert!(invalid_error.to_string().contains("was discarded"));
        assert!(!pending.exists());
        assert!(!version.exists());
    }

    #[test]
    fn pending_version_marker_is_atomic_and_strict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("pending.version");
        let release = PendingUpdateVersion::Release(
            Version::parse("0.8.0-rc.1+build").expect("valid semver"),
        );

        write_pending_version_marker_at(&marker, &release).expect("write valid marker");
        assert_eq!(read_pending_version_marker(&marker).unwrap(), release);
        assert_eq!(
            fs::read_to_string(&marker).expect("read marker text"),
            "release:0.8.0-rc.1+build"
        );
        assert!(!marker.with_extension("version.tmp").exists());

        let invalid = dir.path().join("invalid.version");
        let invalid_dev = PendingUpdateVersion::DevRevision("not-hex".to_string());
        assert!(write_pending_version_marker_at(&invalid, &invalid_dev).is_err());
        assert!(!invalid.exists());
        assert!(!invalid.with_extension("version.tmp").exists());
    }

    #[test]
    fn marker_parser_accepts_typed_dev_and_legacy_release_only() {
        assert_eq!(
            parse_pending_version_marker("dev:0123456789abcdef").unwrap(),
            PendingUpdateVersion::DevRevision("0123456789abcdef".to_string())
        );
        assert_eq!(
            parse_pending_version_marker("1.2.3").unwrap(),
            PendingUpdateVersion::Release(Version::parse("1.2.3").unwrap())
        );
        assert!(parse_pending_version_marker("abcdef0").is_err());
        assert!(parse_pending_version_marker("dev:short").is_err());
    }

    #[test]
    fn release_versions_never_downgrade_or_reapply_equal_version() {
        let current = Version::parse("2.4.0-beta.2").unwrap();

        assert_eq!(
            release_update_disposition(&Version::parse("2.3.99").unwrap(), &current),
            ReleaseUpdateDisposition::Older
        );
        assert_eq!(
            release_update_disposition(&Version::parse("2.4.0-beta.2").unwrap(), &current),
            ReleaseUpdateDisposition::Equal
        );
        assert_eq!(
            release_update_disposition(
                &Version::parse("2.4.0-beta.2+different-build").unwrap(),
                &current
            ),
            ReleaseUpdateDisposition::Equal
        );
        assert_eq!(
            release_update_disposition(&Version::parse("2.4.0").unwrap(), &current),
            ReleaseUpdateDisposition::Newer
        );
    }

    #[test]
    fn invalid_marked_payload_is_removed_from_active_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pending = dir.path().join("pending");
        let marker = dir.path().join("pending.version");
        fs::create_dir(&pending).expect("create invalid payload directory");
        fs::write(&marker, b"release:9.0.0").expect("write marker");

        let error = validate_pending_update_or_discard(&pending, &marker)
            .expect_err("invalid payload must fail");

        assert!(error.to_string().contains("was discarded"));
        assert!(!pending.exists());
        assert!(!marker.exists());
    }

    #[test]
    fn stale_replacement_artifacts_are_reconciled_idempotently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("krusty.exe");
        let backup = dir.path().join("krusty.exe.old");
        let staged = dir.path().join("krusty.new");
        fs::write(&current, b"current").expect("write current");
        fs::write(&backup, b"stale backup").expect("write backup");
        fs::write(&staged, b"stale staged").expect("write staged");

        reconcile_replacement_paths(&current, &backup, &staged).expect("reconcile stale files");
        reconcile_replacement_paths(&current, &backup, &staged).expect("reconcile idempotently");

        assert_eq!(fs::read(&current).unwrap(), b"current");
        assert!(!backup.exists());
        assert!(!staged.exists());
    }

    #[test]
    fn interrupted_replacement_restores_known_good_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("krusty.exe");
        let backup = dir.path().join("krusty.exe.old");
        let staged = dir.path().join("krusty.new");
        fs::write(&backup, b"known good").expect("write backup");
        fs::write(&staged, b"partial update").expect("write staged");

        reconcile_replacement_paths(&current, &backup, &staged).expect("restore backup");

        assert_eq!(fs::read(&current).unwrap(), b"known good");
        assert!(!backup.exists());
        assert!(!staged.exists());
    }

    #[test]
    fn replacement_failure_restores_current_and_cleans_staging() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("krusty.exe");
        let pending = dir.path().join("pending.exe");
        fs::write(&current, b"known good").expect("write current");
        fs::write(&pending, b"new update").expect("write pending");

        let error = replace_current_exe_with(&pending, &current, |staged, current, backup| {
            fs::rename(current, backup)?;
            assert!(staged.exists());
            Err(io::Error::other("simulated replacement failure"))
        })
        .expect_err("replacement must fail");

        assert!(error
            .to_string()
            .contains("runnable executable was preserved"));
        assert_eq!(fs::read(&current).unwrap(), b"known good");
        assert!(!current.with_extension("new").exists());
        assert!(!backup_path(&current).exists());
    }

    #[test]
    fn successful_replacement_keeps_one_launch_rollback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("krusty.exe");
        let pending = dir.path().join("pending.exe");
        let backup = backup_path(&current);
        fs::write(&current, b"known good").expect("write current");
        fs::write(&pending, b"new update").expect("write pending");
        fs::write(&backup, b"stale backup").expect("write stale backup");
        fs::write(current.with_extension("new"), b"stale staged").expect("write stale staged");

        replace_current_exe_with(&pending, &current, |staged, current, backup| {
            fs::rename(current, backup)?;
            fs::rename(staged, current)
        })
        .expect("replace executable");

        assert_eq!(fs::read(&current).unwrap(), b"new update");
        assert_eq!(fs::read(&backup).unwrap(), b"known good");
        assert!(!current.with_extension("new").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_or_world_writable_pending_update() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("chmod dir");
        let pending = dir.path().join("pending");
        fs::write(&pending, b"binary").expect("write pending");
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o666)).expect("chmod pending");

        let err = validate_pending_update(&pending).expect_err("writable file should be rejected");
        assert!(err.to_string().contains("permissions are too broad"));
    }

    #[cfg(unix)]
    #[test]
    fn accepts_owner_only_regular_pending_update() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("chmod dir");
        let pending = dir.path().join("pending");
        fs::write(&pending, b"binary").expect("write pending");
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).expect("chmod pending");

        validate_pending_update(&pending).expect("owner-only file should be accepted");
    }
}
