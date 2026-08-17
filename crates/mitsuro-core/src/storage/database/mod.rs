//! SQLite database wrapper with versioned migrations

mod migrations;

use anyhow::Result;
use rusqlite::{Connection, ErrorCode};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Current schema version
pub(crate) const SCHEMA_VERSION: i32 = 68;

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

        // Apply contention policy before any pragma that may need a write
        // lock. The server and Mako daemon commonly initialize together.
        // Startup can legitimately overlap between the HTTP server, the Mako
        // daemon, and short-lived clients. Give the migration winner enough
        // time to finish even on a busy developer or deployment host instead
        // of turning normal startup serialization into a transient failure.
        conn.busy_timeout(Duration::from_secs(30))?;

        // Enable WAL mode for better concurrent access
        // This prevents lock contention when multiple instances try to access the database
        retry_sqlite_busy(|| conn.pragma_update(None, "journal_mode", "WAL"))?;

        // Enable foreign key enforcement for referential integrity
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    /// Get the underlying connection
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Get the underlying connection mutably for atomic multi-row stores.
    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Create a shared database handle for connection reuse
    ///
    /// Use this when multiple components need to share a single connection.
    pub fn shared(path: &Path) -> Result<SharedDatabase> {
        Ok(Arc::new(Mutex::new(Self::new(path)?)))
    }
}

fn retry_sqlite_busy<T>(mut operation: impl FnMut() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match operation() {
            Err(error) if is_sqlite_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

fn is_sqlite_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(code.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}
