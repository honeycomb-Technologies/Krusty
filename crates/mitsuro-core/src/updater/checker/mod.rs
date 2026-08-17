//! Update checker - supports both dev (git) and release (GitHub) modes.

mod apply;
mod check;
pub(crate) mod checksum;
mod download;
mod extract;
pub(crate) mod paths;
pub(crate) mod policy;
#[cfg(test)]
mod tests;
mod types;

pub use apply::{apply_pending_update, cleanup_pending_update, read_update_marker};
pub use check::check_for_updates;
pub use download::{download_update, pending_update_path};
pub use paths::{detect_repo_path, has_pending_update, is_dev_mode};
pub use policy::self_update_guidance;
pub use types::{UpdateInfo, UpdateStatus};

/// Current version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repo for releases
pub(super) const GITHUB_REPO: &str = "honeycomb-Technologies/Mitsuro";
