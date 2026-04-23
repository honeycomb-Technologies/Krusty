use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::storage::database::Database;

use super::model::StoredMessageRecord;

pub struct MessageStore<'a> {
    db: &'a Database,
}

impl<'a> MessageStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content_json, now],
        )?;

        self.db.conn().execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        Ok(())
    }

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

    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        self.load_session_messages_paginated(session_id, 0, None)
    }

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
            (None, 0) => {
                "SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id".to_string()
            }
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

    pub fn get_message_count(&self, session_id: &str) -> Result<usize> {
        let count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

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

    pub fn delete_session_messages(&self, session_id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
        Ok(())
    }
}
