//! Persistence layer
//!
//! SQLite-based storage for:
//! - Session storage and management
//! - Plan storage with session linkage
//! - User preferences
//! - File activity tracking for context
//! - API credentials

use std::time::{SystemTime, UNIX_EPOCH};

mod agent_state;
mod block_ui;
pub mod credentials;
mod database;
mod file_activity;
mod messages;
mod plans;
mod preferences;
mod sessions;

pub use agent_state::AgentState;
pub use block_ui::BlockUiState;
pub use credentials::CredentialStore;
pub use database::{Database, SharedDatabase};
pub use file_activity::{FileActivityTracker, RankedFile};
pub use messages::MessageStore;
pub use plans::{PlanStore, PlanSummary};
pub use preferences::Preferences;
pub use sessions::{SessionInfo, SessionManager};

/// Get current Unix timestamp in seconds
#[inline]
pub fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
