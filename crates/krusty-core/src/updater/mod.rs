//! Auto-updater module for Krusty
//!
//! Supports both dev mode (git pull + cargo build) and release mode (GitHub releases).
//! Automatically detects which mode based on whether running from a git repo.

mod checker;

pub use checker::{
    apply_update, check_for_updates, detect_repo_path, download_update, is_dev_mode, UpdateInfo,
    UpdateStatus, VERSION,
};
