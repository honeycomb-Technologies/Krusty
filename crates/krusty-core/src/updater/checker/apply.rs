use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tracing::{debug, info, warn};

use super::paths::{pending_update_path, pending_version_path, update_marker_path};

pub fn apply_pending_update() -> Result<Option<String>> {
    let pending = pending_update_path();

    if !pending.exists() {
        return Ok(None);
    }

    validate_pending_update(&pending)?;
    info!("Found pending update at: {}", pending.display());

    let current_exe = std::env::current_exe()?;
    info!("Current binary: {}", current_exe.display());

    replace_current_exe(&pending, &current_exe)?;

    let version =
        fs::read_to_string(pending_version_path()).unwrap_or_else(|_| "latest".to_string());
    let _ = fs::remove_file(pending_version_path());

    let _ = fs::create_dir_all(crate::paths::config_dir());
    let _ = fs::write(update_marker_path(), &version);

    Ok(Some(version))
}

fn replace_current_exe(pending: &Path, current_exe: &Path) -> Result<()> {
    let backup = backup_path(current_exe);
    let staged = current_exe.with_extension("new");

    let _ = fs::remove_file(&staged);
    debug!("Staging pending update at: {}", staged.display());
    fs::copy(pending, &staged).context("failed to stage pending update")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&staged)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&staged, perms)?;
    }

    debug!("Renaming current to: {}", backup.display());
    fs::rename(current_exe, &backup).context("failed to back up current executable")?;

    debug!("Installing staged update to: {}", current_exe.display());
    if let Err(err) = fs::rename(&staged, current_exe) {
        if let Err(restore_err) = fs::rename(&backup, current_exe) {
            return Err(err).with_context(|| {
                format!(
                    "failed to install staged update and failed to restore backup: {}",
                    restore_err
                )
            });
        }
        return Err(err).context("failed to install staged update");
    }

    if let Err(err) = fs::remove_file(&backup) {
        warn!(
            "Failed to remove update backup {}: {}",
            backup.display(),
            err
        );
    }
    if let Err(err) = fs::remove_file(pending) {
        warn!(
            "Failed to remove pending update {}: {}",
            pending.display(),
            err
        );
    }

    info!("Update applied successfully");
    Ok(())
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
    if pending.exists() {
        let _ = fs::remove_file(&pending);
        let _ = fs::remove_file(pending_version_path());
        info!("Cleaned up pending update");
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
