//! Message persistence storage
//!
//! Handles saving and loading messages for sessions.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::database::Database;

/// Message persistence store
pub struct MessageStore<'a> {
    db: &'a Database,
}

impl<'a> MessageStore<'a> {
    /// Create a new message store with database reference
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Save a message to a session
    /// The content field stores JSON-serialized Vec<Content> for full fidelity
    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content_json, now],
        )?;

        // Update session timestamp
        self.db.conn().execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        Ok(())
    }

    /// Load all messages for a session
    /// Returns (role, content_json) pairs where content_json can be deserialized to Vec<Content>
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        self.load_session_messages_paginated(session_id, 0, None)
    }

    /// Load messages for a session with paging support
    ///
    /// # Arguments
    /// * `session_id` - Session to load messages from
    /// * `offset` - Number of messages to skip (for paging)
    /// * `limit` - Maximum number of messages to return (None = no limit)
    ///
    /// Returns (role, content_json) pairs where content_json can be deserialized to Vec<Content>
    pub fn load_session_messages_paginated(
        &self,
        session_id: &str,
        offset: usize,
        limit: Option<usize>,
    ) -> Result<Vec<(String, String)>> {
        let sql = match (limit, offset) {
            (Some(limit_value), _) => format!(
                "SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id LIMIT {} OFFSET {}",
                limit_value, offset
            ),
            (None, 0) => "SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id".to_string(),
            (None, _) => format!(
                "SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id LIMIT -1 OFFSET {}",
                offset
            ),
        };

        let mut stmt = self.db.conn().prepare(&sql)?;

        let messages = stmt.query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        messages.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get total message count for a session (for paging UI)
    pub fn get_message_count(&self, session_id: &str) -> Result<usize> {
        let count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Delete all messages for a session
    /// Called automatically when session is deleted via CASCADE
    pub fn delete_session_messages(&self, session_id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;

    use crate::storage::Database;

    use super::MessageStore;

    /// Helper to create a temporary database for testing
    fn create_test_db() -> (Database, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        (db, temp_dir)
    }

    #[test]
    fn test_save_and_load_messages() {
        let (db, _temp) = create_test_db();
        let store = MessageStore::new(&db);

        // Create a session first
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Test", now, now],
            )
            .expect("Failed to create session");

        // Save messages
        store
            .save_message(&session_id, "user", r#"[{"type":"text","text":"Hello"}]"#)
            .expect("Failed to save message");
        store
            .save_message(
                &session_id,
                "assistant",
                r#"[{"type":"text","text":"Hi there"}]"#,
            )
            .expect("Failed to save message");

        // Load messages
        let messages = store
            .load_session_messages(&session_id)
            .expect("Failed to load messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, "user");
        assert_eq!(messages[1].0, "assistant");
    }
}
