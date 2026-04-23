use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

use super::paths::detect_repo_path;
use super::types::{UpdateInfo, UpdateStatus};

mod dev;
mod release;

pub use super::paths::pending_update_path;

pub async fn download_update(
    info: &UpdateInfo,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    if info.is_dev_mode {
        let repo_path = detect_repo_path().ok_or_else(|| anyhow!("No repo path for dev mode"))?;
        dev::download_update_dev(&repo_path, &info.new_version, progress_tx).await
    } else {
        release::download_update_release(&info.new_version, progress_tx).await
    }
}
