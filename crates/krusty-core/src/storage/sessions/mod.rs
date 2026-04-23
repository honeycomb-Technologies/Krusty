//! Session CRUD operations

mod block_ui;
mod creation;
mod lifecycle;
mod messages;
mod metadata;
mod queries;
mod recovery;
mod runtime;
#[cfg(test)]
mod tests;
mod types;

use super::database::Database;

pub use types::{SessionInfo, SessionType, WorkMode, WorkspaceMode};

/// Session manager for CRUD operations
pub struct SessionManager {
    db: Database,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Get reference to underlying database
    pub fn db(&self) -> &Database {
        &self.db
    }
}
