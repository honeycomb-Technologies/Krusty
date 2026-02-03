//! Block UI state storage
//!
//! Handles persistence of block UI state (collapsed, scroll_offset) for session restoration.

use anyhow::Result;
use rusqlite::params;

use super::database::Database;

/// Block UI state for session restoration
#[derive(Debug, Clone)]
pub struct BlockUiState {
    pub block_id: String,
    pub collapsed: bool,
    pub scroll_offset: u16,
}

/// Block UI state store
pub struct BlockUiStore<'a> {
    db: &'a Database,
}

impl<'a> BlockUiStore<'a> {
    /// Create a new block UI state store with database reference
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Save block UI state (collapsed, scroll_offset) for a block
    pub fn save_block_ui_state(
        &self,
        session_id: &str,
        block_id: &str,
        collapsed: bool,
        scroll_offset: u16,
    ) -> Result<()> {
        self.db.conn().execute(
            "INSERT OR REPLACE INTO block_ui_state (session_id, block_id, block_type, collapsed, scroll_offset)
             VALUES (?1, ?2, '', ?3, ?4)",
            params![session_id, block_id, collapsed as i32, scroll_offset as i32],
        )?;
        Ok(())
    }

    /// Load all block UI states for a session
    pub fn load_block_ui_states(&self, session_id: &str) -> Vec<BlockUiState> {
        let result = (|| -> Result<Vec<BlockUiState>> {
            let mut stmt = self.db.conn().prepare(
                "SELECT block_id, collapsed, scroll_offset
                 FROM block_ui_state WHERE session_id = ?1",
            )?;

            let states = stmt.query_map([session_id], |row| {
                Ok(BlockUiState {
                    block_id: row.get(0)?,
                    collapsed: row.get::<_, i32>(1)? != 0,
                    scroll_offset: row.get::<_, i32>(2)? as u16,
                })
            })?;

            states.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })();

        result.unwrap_or_default()
    }

    /// Delete all block UI states for a session
    /// Called automatically when session is deleted via CASCADE
    #[allow(dead_code)]
    pub fn delete_session_block_states(&self, session_id: &str) -> Result<()> {
        self.db.conn().execute(
            "DELETE FROM block_ui_state WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::storage::Database;

    use super::BlockUiStore;

    /// Helper to create a temporary database for testing
    fn create_test_db() -> (Database, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        (db, temp_dir)
    }

    #[test]
    fn test_save_and_load_block_ui_state() {
        let (db, _temp) = create_test_db();
        let store = BlockUiStore::new(&db);

        // Create a session first
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Test", now, now],
            )
            .expect("Failed to create session");

        // Save block UI state
        store
            .save_block_ui_state(&session_id, "block-1", true, 42)
            .expect("Failed to save block UI state");
        store
            .save_block_ui_state(&session_id, "block-2", false, 0)
            .expect("Failed to save block UI state");

        // Load states
        let states = store.load_block_ui_states(&session_id);

        assert_eq!(states.len(), 2);
        assert_eq!(states[0].block_id, "block-1");
        assert_eq!(states[0].collapsed, true);
        assert_eq!(states[0].scroll_offset, 42);
        assert_eq!(states[1].block_id, "block-2");
        assert_eq!(states[1].collapsed, false);
    }
}
