//! SQLite database wrapper with versioned migrations

mod migrations;

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Current schema version
const SCHEMA_VERSION: i32 = 39;

/// Shared database handle for connection reuse
///
/// Wraps a Database in Arc<Mutex> for safe sharing across components.
/// Use this instead of creating multiple Database instances.
pub type SharedDatabase = Arc<Mutex<Database>>;

/// SQLite database wrapper
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Create a new database at the given path
    pub fn new(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;

        // Enable WAL mode for better concurrent access
        // This prevents lock contention when multiple instances try to access the database
        conn.pragma_update(None, "journal_mode", "WAL")?;

        // Enable foreign key enforcement for referential integrity
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // Set busy timeout to avoid immediate failures on lock contention
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    /// Get the underlying connection
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Create a shared database handle for connection reuse
    ///
    /// Use this when multiple components need to share a single connection.
    pub fn shared(path: &Path) -> Result<SharedDatabase> {
        Ok(Arc::new(Mutex::new(Self::new(path)?)))
    }
}
