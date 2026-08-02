//! File activity tracking for pinch
//!
//! Tracks read/write/edit operations on files during a session
//! to determine which files are most important for context preservation.

mod model;
mod tracker;

pub use model::RankedFile;
pub use tracker::FileActivityTracker;
