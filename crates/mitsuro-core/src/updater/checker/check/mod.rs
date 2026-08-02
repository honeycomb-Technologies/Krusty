use anyhow::Result;
use tracing::{debug, info};

use super::paths::detect_repo_path;
use super::types::UpdateInfo;
use super::VERSION;

mod git;
mod release;

#[cfg(test)]
pub(crate) fn is_newer_version(new: &str, current: &str) -> bool {
    git::is_newer_version(new, current)
}

pub async fn check_for_updates() -> Result<Option<UpdateInfo>> {
    info!("Checking for updates (current version: {})", VERSION);

    if let Some(repo_path) = detect_repo_path() {
        debug!("Dev mode detected, checking git for updates");
        git::check_for_updates_dev(&repo_path)
    } else {
        debug!("Release mode, checking GitHub releases");
        release::check_for_updates_release().await
    }
}
