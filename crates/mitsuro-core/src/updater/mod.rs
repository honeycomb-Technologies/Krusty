//! Auto-updater module for Mitsuro
//!
//! Product updates install the complete release set (CLI/TUI, Hive, shims, and
//! units). Single-binary replacement stays fail-closed.

mod channel;
mod checker;
mod managed;

pub use channel::{UpdateApplyPolicy, UpdateChannel};
pub use checker::{
    apply_pending_update, check_for_updates, cleanup_pending_update, detect_repo_path,
    download_update, has_pending_update, is_dev_mode, pending_update_path, read_update_marker,
    self_update_guidance, UpdateInfo, UpdateStatus, VERSION,
};
pub use managed::{apply_managed_release_update, apply_managed_release_update_to};
