use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc;

use crate::updater::checker::apply::{discard_pending_update, write_pending_dev_marker};
use crate::updater::checker::paths::{
    ensure_pending_update_dir, pending_update_path, pending_version_path,
};
use crate::updater::checker::types::UpdateStatus;

pub(super) async fn download_update_dev(
    repo_path: &std::path::Path,
    version: &str,
    progress_tx: mpsc::UnboundedSender<UpdateStatus>,
) -> Result<()> {
    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Pulling latest changes...".into(),
    });

    let pull = tokio::process::Command::new("git")
        .args(["pull", "origin", "main"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !pull.status.success() {
        let err = String::from_utf8_lossy(&pull.stderr);
        return Err(anyhow!("Git pull failed: {}", err));
    }

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Building release binary...".into(),
    });

    let build = tokio::process::Command::new("cargo")
        .args(["build", "--release", "-p", "krusty"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !build.status.success() {
        let err = String::from_utf8_lossy(&build.stderr);
        return Err(anyhow!("Cargo build failed: {}", err));
    }

    let _ = progress_tx.send(UpdateStatus::Downloading {
        progress: "Preparing update...".into(),
    });

    ensure_pending_update_dir()?;
    let source = repo_path.join("target/release/krusty");
    let dest = pending_update_path();
    let version_path = pending_version_path();
    discard_pending_update(&dest, &version_path)
        .context("failed to clear stale pending update artifacts")?;
    let pending_cleanup =
        scopeguard::guard((dest.clone(), version_path), |(dest, version_path)| {
            let _ = discard_pending_update(&dest, &version_path);
        });

    std::fs::copy(&source, &dest)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&dest, perms)?;
    }

    write_pending_dev_marker(version)?;
    scopeguard::ScopeGuard::into_inner(pending_cleanup);

    let _ = progress_tx.send(UpdateStatus::Ready {
        version: version.to_string(),
    });

    Ok(())
}
