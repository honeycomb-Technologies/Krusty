use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub(super) fn pending_update_dir() -> PathBuf {
    crate::paths::config_dir().join("updates")
}

pub fn pending_update_path() -> PathBuf {
    pending_update_dir().join("pending-update")
}

pub(super) fn pending_archive_path(ext: &str) -> PathBuf {
    pending_update_dir().join(format!("download.{}", ext))
}

pub(super) fn pending_version_path() -> PathBuf {
    pending_update_dir().join("pending-update.version")
}

pub(super) fn update_marker_path() -> PathBuf {
    crate::paths::config_dir().join("last-update-version")
}

pub(super) fn ensure_pending_update_dir() -> Result<PathBuf> {
    let dir = pending_update_dir();
    std::fs::create_dir_all(&dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = std::fs::metadata(&dir)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&dir, perms)?;
    }

    Ok(dir)
}

pub fn has_pending_update() -> bool {
    pending_update_path().exists()
}

pub fn is_dev_mode() -> bool {
    detect_repo_path().is_some()
}

pub fn detect_repo_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if parent.ends_with("release") {
                if let Some(target) = parent.parent() {
                    if target.ends_with("target") {
                        if let Some(repo) = target.parent() {
                            if repo.join("Cargo.toml").exists() {
                                return Some(repo.to_path_buf());
                            }
                        }
                    }
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() && cwd.join("crates/krusty-cli").exists() {
            return Some(cwd);
        }
    }

    None
}

pub(super) fn detect_platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => Err(anyhow!("Unsupported platform")),
    }
}
