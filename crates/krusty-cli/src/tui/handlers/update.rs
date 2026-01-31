//! Auto-update handlers
//!
//! Background update checking and downloading from GitHub releases.
//! Updates are downloaded but NOT applied while running - user must restart.

use crate::tui::app::App;
use crate::tui::components::Toast;
use krusty_core::updater::{
    check_for_updates, cleanup_pending_update, download_update, has_pending_update, UpdateStatus,
    VERSION,
};
use tokio::sync::mpsc;

impl App {
    /// Check if there's a pending update that was downloaded previously.
    /// Called early in startup - if we get here, apply_pending_update() was already
    /// called in main.rs. If a pending file still exists, it means apply failed,
    /// so we clean it up rather than showing a toast every restart.
    pub fn check_pending_update(&mut self) {
        if has_pending_update() {
            cleanup_pending_update();
            tracing::info!("Cleaned up stale pending update file");
        }
    }

    /// Start background update check
    pub fn start_update_check(&mut self) {
        // Skip if we just applied an update this session — the in-memory VERSION
        // constant is stale and would trigger a redundant re-download loop
        if self.just_updated {
            return;
        }

        // Don't start if already checking
        if self.channels.update_status.is_some() {
            return;
        }

        // Don't check if we already have an update ready
        if has_pending_update() {
            return; // Already handled by check_pending_update
        }

        // Create channel for status updates
        let (tx, rx) = mpsc::unbounded_channel();
        self.channels.update_status = Some(rx);

        // Spawn background task to check for updates
        tokio::spawn(async move {
            let _ = tx.send(UpdateStatus::Checking);

            match check_for_updates().await {
                Ok(Some(info)) => {
                    // Only notify and download if actually newer
                    if info.new_version != VERSION {
                        let _ = tx.send(UpdateStatus::Available(info.clone()));

                        // Auto-download the update (but don't apply)
                        match download_update(&info, tx.clone()).await {
                            Ok(()) => {
                                // Ready status already sent by download_update
                            }
                            Err(e) => {
                                let _ = tx.send(UpdateStatus::Error(e.to_string()));
                            }
                        }
                    } else {
                        // Same version - clean up any stale pending update
                        cleanup_pending_update();
                        let _ = tx.send(UpdateStatus::UpToDate);
                    }
                }
                Ok(None) => {
                    let _ = tx.send(UpdateStatus::UpToDate);
                }
                Err(e) => {
                    tracing::debug!("Update check failed: {}", e);
                    // Silent fail on network errors - don't spam user
                }
            }
        });
    }

    /// Poll update status channel and show toasts
    pub fn poll_update_status(&mut self) {
        let statuses: Vec<UpdateStatus> = if let Some(ref mut rx) = self.channels.update_status {
            let mut collected = Vec::new();
            while let Ok(status) = rx.try_recv() {
                collected.push(status);
            }
            collected
        } else {
            return;
        };

        let mut clear_channel = false;

        for status in statuses {
            match &status {
                UpdateStatus::Checking => {
                    // Silent - don't spam user
                }
                UpdateStatus::UpToDate => {
                    // Silent on startup - clean up any stale state
                    self.update_status = None;
                    clear_channel = true;
                }
                UpdateStatus::Available(_) => {
                    // Silent - download in progress, toast shown when ready
                }
                UpdateStatus::Downloading { progress: _ } => {
                    // Silent - don't spam user with progress
                }
                UpdateStatus::Ready { version } => {
                    self.show_toast(Toast::success(format!(
                        "v{} ready — restart to install",
                        version
                    )));
                    clear_channel = true;
                }
                UpdateStatus::Error(e) => {
                    // Only show error for non-network issues
                    if !e.contains("timeout") && !e.contains("connection") {
                        tracing::warn!("Update check failed: {}", e);
                    }
                    self.update_status = None;
                    clear_channel = true;
                }
            }
            self.update_status = Some(status);
        }

        if clear_channel {
            self.channels.update_status = None;
        }
    }
}
