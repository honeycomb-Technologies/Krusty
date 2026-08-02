use anyhow::Result;
use rusqlite::params;

use super::SessionManager;

impl SessionManager {
    /// Delete a session and all its messages
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        // First, clear parent_session_id references from children (orphan them)
        // This prevents foreign key constraint violations
        self.db.conn().execute(
            "UPDATE sessions SET parent_session_id = NULL WHERE parent_session_id = ?1",
            params![session_id],
        )?;

        // Clear pinch_metadata references
        self.db.conn().execute(
            "DELETE FROM pinch_metadata WHERE source_session_id = ?1 OR target_session_id = ?1",
            params![session_id],
        )?;

        // Clear file_activity for this session
        self.db.conn().execute(
            "DELETE FROM file_activity WHERE session_id = ?1",
            params![session_id],
        )?;

        // Clear block_ui_state for this session
        self.db.conn().execute(
            "DELETE FROM block_ui_state WHERE session_id = ?1",
            params![session_id],
        )?;

        // Messages will be deleted via ON DELETE CASCADE
        self.db
            .conn()
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;

        tracing::info!(session_id = %session_id, "Session deleted from database");
        Ok(())
    }
}
