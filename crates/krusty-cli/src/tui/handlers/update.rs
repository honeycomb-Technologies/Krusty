//! Auto-update handlers
//!
//! Background update checking and building.

use crate::tui::app::App;
use crate::tui::components::Toast;
use krusty_core::updater::UpdateStatus;

impl App {
    /// Check for persisted update on startup
    pub fn check_persisted_update(&mut self) {
        // TODO: Check if there's a pending update binary to apply
    }

    /// Start background update check
    pub fn start_update_check(&mut self) {
        // Don't start if already checking
        if self.channels.update_status.is_some() {
            return;
        }

        // Don't check if we already have an update ready
        if matches!(self.update_status, Some(UpdateStatus::Ready { .. })) {
            self.show_toast(Toast::info("Update ready - restart to apply"));
            return;
        }

        // TODO: Spawn background task to check for updates
        // For now, just show a message
        self.show_toast(Toast::info("Update checking not yet wired up"));
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
                UpdateStatus::Checking => {}
                UpdateStatus::UpToDate => {
                    self.show_toast(Toast::success("Up to date!"));
                    self.update_status = None;
                    clear_channel = true;
                }
                UpdateStatus::Available(info) => {
                    self.show_toast(Toast::info(format!(
                        "Update available: v{}",
                        info.new_version
                    )));
                }
                UpdateStatus::Downloading { progress } => {
                    tracing::debug!("Update progress: {}", progress);
                }
                UpdateStatus::Ready { version, .. } => {
                    self.show_toast(
                        Toast::success(format!("Updated to v{} - restart to apply", version))
                            .persistent(),
                    );
                    clear_channel = true;
                }
                UpdateStatus::Error(e) => {
                    self.show_toast(Toast::info(format!("Update failed: {}", e)));
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
