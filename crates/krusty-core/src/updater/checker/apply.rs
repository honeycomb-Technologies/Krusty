use anyhow::Result;
use tracing::{debug, info};

use super::paths::{pending_update_path, pending_version_path, update_marker_path};

pub fn apply_pending_update() -> Result<Option<String>> {
    let pending = pending_update_path();

    if !pending.exists() {
        return Ok(None);
    }

    info!("Found pending update at: {}", pending.display());

    let current_exe = std::env::current_exe()?;
    info!("Current binary: {}", current_exe.display());

    #[cfg(unix)]
    {
        let backup = current_exe.with_extension("old");
        debug!("Renaming current to: {}", backup.display());
        std::fs::rename(&current_exe, &backup)?;
        debug!("Copying new binary to: {}", current_exe.display());
        std::fs::copy(&pending, &current_exe)?;

        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms)?;

        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::remove_file(&pending);

        info!("Update applied successfully");
    }

    #[cfg(windows)]
    {
        let backup = current_exe.with_extension("exe.old");
        std::fs::rename(&current_exe, &backup)?;
        std::fs::copy(&pending, &current_exe)?;
        let _ = std::fs::remove_file(&pending);
        info!("Update applied successfully");
    }

    let version =
        std::fs::read_to_string(pending_version_path()).unwrap_or_else(|_| "latest".to_string());
    let _ = std::fs::remove_file(pending_version_path());

    let _ = std::fs::create_dir_all(crate::paths::config_dir());
    let _ = std::fs::write(update_marker_path(), &version);

    Ok(Some(version))
}

pub fn read_update_marker() -> Option<String> {
    let path = update_marker_path();
    let version = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let trimmed = version.trim().to_string();
    (!trimmed.is_empty() && trimmed != "latest").then_some(trimmed)
}

pub fn cleanup_pending_update() {
    let pending = pending_update_path();
    if pending.exists() {
        let _ = std::fs::remove_file(&pending);
        let _ = std::fs::remove_file(pending_version_path());
        info!("Cleaned up pending update");
    }
}
