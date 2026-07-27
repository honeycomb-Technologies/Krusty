use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::{params, Row};

use crate::storage::database::Database;

#[derive(Debug, Clone)]
pub struct NotificationIntent {
    pub id: String,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
}

pub struct NotificationIntentStore<'a> {
    db: &'a Database,
}

impl<'a> NotificationIntentStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn enqueue(
        &self,
        transport: &str,
        user_id: Option<&str>,
        session_id: Option<&str>,
        event_type: &str,
        payload: &serde_json::Value,
        ttl_seconds: i64,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let operation_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(ttl_seconds.max(60));
        let qualified_event = format!("{transport}:{event_type}");
        self.db.conn().execute(
            "INSERT INTO notification_intents (
                id, operation_id, user_id, session_id, event_type, payload_json,
                status, attempt_count, available_at, expires_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?8, ?7, ?7)",
            params![
                id,
                operation_id,
                user_id,
                session_id,
                qualified_event,
                serde_json::to_string(payload)?,
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
            ],
        )?;
        Ok(id)
    }

    pub fn recoverable(&self, transport: &str, limit: usize) -> Result<Vec<NotificationIntent>> {
        let pattern = format!("{transport}:%");
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE notification_intents
             SET status = 'expired', updated_at = ?1
             WHERE event_type LIKE ?2
               AND status IN ('pending', 'dispatching')
               AND expires_at IS NOT NULL
               AND expires_at <= ?1",
            params![now, pattern],
        )?;
        let mut statement = self.db.conn().prepare(
            "SELECT id, user_id, session_id, event_type, payload_json
             FROM notification_intents
             WHERE event_type LIKE ?1
               AND status IN ('pending', 'dispatching')
               AND available_at <= ?2
               AND (expires_at IS NULL OR expires_at > ?2)
             ORDER BY created_at ASC
             LIMIT ?3",
        )?;
        let intents: rusqlite::Result<Vec<_>> = statement
            .query_map(params![pattern, now, limit as i64], map_row)?
            .collect();
        Ok(intents?)
    }

    pub fn mark_dispatching(&self, id: &str) -> Result<()> {
        self.db.conn().execute(
            "UPDATE notification_intents
             SET status = 'dispatching', attempt_count = attempt_count + 1,
                 updated_at = ?1
             WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn mark_accepted(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE notification_intents
             SET status = 'accepted', updated_at = ?1, accepted_at = ?1,
                 last_error = NULL
             WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, id: &str, error: &str) -> Result<()> {
        self.db.conn().execute(
            "UPDATE notification_intents
             SET status = 'failed', updated_at = ?1, last_error = ?2
             WHERE id = ?3",
            params![Utc::now().to_rfc3339(), error, id],
        )?;
        Ok(())
    }

    pub fn mark_cancelled(&self, id: &str, reason: &str) -> Result<()> {
        self.db.conn().execute(
            "UPDATE notification_intents
             SET status = 'cancelled', updated_at = ?1, last_error = ?2
             WHERE id = ?3",
            params![Utc::now().to_rfc3339(), reason, id],
        )?;
        Ok(())
    }
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<NotificationIntent> {
    let payload_json: String = row.get(4)?;
    let payload = serde_json::from_str(&payload_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            payload_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(NotificationIntent {
        id: row.get(0)?,
        user_id: row.get(1)?,
        session_id: row.get(2)?,
        event_type: row.get(3)?,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::NotificationIntentStore;
    use crate::storage::Database;
    use tempfile::TempDir;

    #[test]
    fn pending_intent_survives_reopen_and_leaves_recovery_after_acceptance() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("notification-intents.db");
        let id = {
            let db = Database::new(&path).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO sessions (id, title, created_at, updated_at)
                     VALUES ('session-1', 'Notification test', datetime('now'), datetime('now'))",
                    [],
                )
                .unwrap();
            NotificationIntentStore::new(&db)
                .enqueue(
                    "apns",
                    Some("user-1"),
                    Some("session-1"),
                    "completion",
                    &serde_json::json!({ "title": "Complete" }),
                    600,
                )
                .unwrap()
        };

        let db = Database::new(&path).unwrap();
        let store = NotificationIntentStore::new(&db);
        let pending = store.recoverable("apns", 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].user_id.as_deref(), Some("user-1"));
        assert_eq!(pending[0].session_id.as_deref(), Some("session-1"));

        store.mark_dispatching(&id).unwrap();
        store.mark_accepted(&id).unwrap();
        assert!(store.recoverable("apns", 10).unwrap().is_empty());
    }
}
