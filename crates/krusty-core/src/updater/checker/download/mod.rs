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
    super::policy::require_safe_single_binary_update()?;

    if info.is_dev_mode {
        let repo_path = detect_repo_path().ok_or_else(|| anyhow!("No repo path for dev mode"))?;
        dev::download_update_dev(&repo_path, &info.new_version, progress_tx).await
    } else {
        release::download_update_release(&info.new_version, progress_tx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_dev_and_release_download_paths_fail_before_mutation() {
        for is_dev_mode in [true, false] {
            let info = UpdateInfo {
                current_version: "old".to_string(),
                new_version: "new".to_string(),
                release_notes: String::new(),
                is_dev_mode,
            };
            let (progress_tx, _progress_rx) = mpsc::unbounded_channel();

            let error = download_update(&info, progress_tx)
                .await
                .expect_err("Unix updater must fail closed");

            assert!(error.to_string().contains("krusty-mako"));
        }
    }
}
