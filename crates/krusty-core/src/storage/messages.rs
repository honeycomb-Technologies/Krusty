//! Message persistence storage
//!
//! Handles saving and loading messages for sessions.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::database::Database;

/// Message persistence store
pub struct MessageStore<'a> {
    db: &'a Database,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessageRecord {
    pub role: String,
    pub content_json: String,
    pub created_at: String,
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

    /// Replace every persisted message for a session with a new ordered set.
    pub fn replace_session_messages(
        &self,
        session_id: &str,
        messages: &[(String, String)],
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let tx = self.db.conn().unchecked_transaction()?;

        tx.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;

        for (role, content_json) in messages {
            tx.execute(
                "INSERT INTO messages (session_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, role, content_json, now],
            )?;
        }

        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        tx.commit()?;

        Ok(())
    }

    /// Load all messages for a session
    /// Returns (role, content_json) pairs where content_json can be deserialized to Vec<Content>
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        self.load_session_messages_paginated(session_id, 0, None)
    }

    /// Load all stored message rows with created_at metadata for a session.
    pub fn load_session_message_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<StoredMessageRecord>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT role, content, created_at
             FROM messages
             WHERE session_id = ?1
             ORDER BY id",
        )?;

        let rows = stmt.query_map([session_id], |row| {
            Ok(StoredMessageRecord {
                role: row.get(0)?,
                content_json: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

    /// Update the most recent message of a given role in a session
    pub fn update_last_message(
        &self,
        session_id: &str,
        role: &str,
        content_json: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE messages SET content = ?1
             WHERE id = (
                 SELECT id FROM messages
                 WHERE session_id = ?2 AND role = ?3
                 ORDER BY id DESC LIMIT 1
             )",
            params![content_json, session_id, role],
        )?;
        if affected == 0 {
            anyhow::bail!(
                "No {} message found to update in session {}",
                role,
                session_id
            );
        }
        self.db.conn().execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
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

    #[test]
    fn test_update_last_message_preserves_created_at() {
        let (db, _temp) = create_test_db();
        let store = MessageStore::new(&db);

        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Test", now, now],
            )
            .expect("Failed to create session");

        store
            .save_message(&session_id, "user", r#"[{"type":"text","text":"first"}]"#)
            .expect("Failed to save first message");
        store
            .save_message(
                &session_id,
                "assistant",
                r#"[{"type":"text","text":"reply"}]"#,
            )
            .expect("Failed to save assistant message");
        store
            .save_message(&session_id, "user", r#"[{"type":"text","text":"second"}]"#)
            .expect("Failed to save second message");

        let before: String = db
            .conn()
            .query_row(
                "SELECT created_at FROM messages
                 WHERE session_id = ?1 AND role = 'user'
                 ORDER BY id DESC LIMIT 1",
                [session_id.as_str()],
                |row| row.get(0),
            )
            .expect("Failed to read created_at before update");

        store
            .update_last_message(&session_id, "user", r#"[{"type":"text","text":"updated"}]"#)
            .expect("Failed to update last user message");

        let (content, after): (String, String) = db
            .conn()
            .query_row(
                "SELECT content, created_at FROM messages
                 WHERE session_id = ?1 AND role = 'user'
                 ORDER BY id DESC LIMIT 1",
                [session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("Failed to read updated message");

        assert_eq!(content, r#"[{"type":"text","text":"updated"}]"#);
        assert_eq!(after, before);
    }

    #[test]
    fn test_replace_session_messages_rewrites_history() {
        let (db, _temp) = create_test_db();
        let store = MessageStore::new(&db);

        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Test", now, now],
            )
            .expect("Failed to create session");

        store
            .save_message(&session_id, "user", r#"[{"type":"text","text":"old"}]"#)
            .expect("Failed to save message");
        store
            .save_message(
                &session_id,
                "assistant",
                r#"[{"type":"text","text":"reply"}]"#,
            )
            .expect("Failed to save message");

        store
            .replace_session_messages(
                &session_id,
                &[
                    (
                        "system".to_string(),
                        r#"[{"type":"text","text":"summary"}]"#.to_string(),
                    ),
                    (
                        "user".to_string(),
                        r#"[{"type":"text","text":"continue"}]"#.to_string(),
                    ),
                ],
            )
            .expect("Failed to replace messages");

        let messages = store
            .load_session_messages(&session_id)
            .expect("Failed to load replaced messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, "system");
        assert_eq!(messages[1].0, "user");
    }
}
