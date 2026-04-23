use std::path::PathBuf;

use anyhow::{anyhow, Result};

pub fn pending_update_path() -> PathBuf {
    std::env::temp_dir().join("krusty-pending-update")
}

pub(super) fn pending_version_path() -> PathBuf {
    std::env::temp_dir().join("krusty-pending-update.version")
}

pub(super) fn update_marker_path() -> PathBuf {
    crate::paths::config_dir().join("last-update-version")
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
