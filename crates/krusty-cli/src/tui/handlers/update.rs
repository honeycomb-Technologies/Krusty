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
    /// Called early in startup - just shows notification, doesn't apply.
    pub fn check_pending_update(&mut self) {
        if has_pending_update() {
            self.show_toast(
                Toast::success("Update ready - restart krusty to apply").persistent(),
            );
        }
    }

    /// Start background update check
    pub fn start_update_check(&mut self) {
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
                UpdateStatus::Available(info) => {
                    // Only show if not already showing a toast for this
                    self.show_toast(Toast::info(format!(
                        "Downloading v{}...",
                        info.new_version
                    )));
                }
                UpdateStatus::Downloading { progress: _ } => {
                    // Silent - don't spam user with progress
                }
                UpdateStatus::Ready { version } => {
                    // Update is downloaded and ready - user needs to restart
                    self.show_toast(
                        Toast::success(format!("v{} ready - restart to update", version))
                            .persistent(),
                    );
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
