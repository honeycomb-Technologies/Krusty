//! Auto-updater module for Krusty
//!
//! Supports both dev mode (git pull + cargo build) and release mode (GitHub releases).
//! Automatically detects which mode based on whether running from a git repo.

mod checker;

pub use checker::{
    apply_pending_update, check_for_updates, cleanup_pending_update, detect_repo_path,
    download_update, has_pending_update, is_dev_mode, pending_update_path, UpdateInfo,
    UpdateStatus, VERSION,
};
