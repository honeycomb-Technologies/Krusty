use anyhow::{anyhow, Result};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::updater::checker::extract::{extract_tar_gz, extract_zip};
use crate::updater::checker::paths::{detect_platform, pending_update_path, pending_version_path};
use crate::updater::checker::types::UpdateStatus;
use crate::updater::checker::GITHUB_REPO;

pub(super) async fn download_update_release(
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: format!("Downloading v{}...", version),
    });

    let platform = detect_platform()?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };

    let url = format!(
        "https://github.com/{}/releases/download/v{}/krusty-{}.{}",
        GITHUB_REPO, version, platform, ext
    );
    info!("Downloading: {}", url);

    let client = reqwest::Client::builder()
        .user_agent("krusty-updater")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!("Download failed: HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;
    info!("Downloaded {} bytes", bytes.len());

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Extracting...".into(),
    });

    let temp_dir = std::env::temp_dir();
    let archive_path = temp_dir.join(format!("krusty-download.{}", ext));
    std::fs::write(&archive_path, &bytes)?;
    debug!("Saved archive to: {}", archive_path.display());

    let binary_path = pending_update_path();

    if cfg!(windows) {
        extract_zip(&archive_path, &binary_path)?;
    } else {
        extract_tar_gz(&archive_path, &binary_path)?;
    }

    if !binary_path.exists() {
        return Err(anyhow!("Extraction failed - binary not found"));
    }

    let metadata = std::fs::metadata(&binary_path)?;
    info!("Extracted binary: {} bytes", metadata.len());

    let _ = std::fs::remove_file(&archive_path);
    let _ = std::fs::write(pending_version_path(), version);

    let _ = progress_tx.send(UpdateStatus::Ready {
        version: version.to_string(),
    });

    info!("Update ready at: {}", binary_path.display());
    Ok(())
}
