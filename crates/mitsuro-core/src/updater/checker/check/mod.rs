use anyhow::Result;
use tracing::{debug, info};

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
    debug!("Checking GitHub releases");
    release::check_for_updates_release().await
}
