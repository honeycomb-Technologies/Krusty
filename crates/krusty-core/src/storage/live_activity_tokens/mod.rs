use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use crate::storage::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveActivityToken {
    pub id: String,
    pub user_id: Option<String>,
    pub session_id: String,
    pub push_token: String,
    pub bundle_id: String,
    pub environment: String,
    pub content_state: serde_json::Value,
    pub started_at_ms: i64,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LiveActivityTokenRegistration<'a> {
    pub user_id: Option<&'a str>,
    pub session_id: &'a str,
    pub push_token: &'a str,
    pub bundle_id: &'a str,
    pub environment: &'a str,
    pub content_state: &'a serde_json::Value,
    pub started_at_ms: i64,
}

pub struct LiveActivityTokenStore<'a> {
    db: &'a Database,
}

impl<'a> LiveActivityTokenStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn upsert(&self, registration: LiveActivityTokenRegistration<'_>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let content_state_json = serde_json::to_string(registration.content_state)?;
        self.db.conn().execute(
            "INSERT INTO live_activity_tokens (
                id, user_id, session_id, push_token, bundle_id, environment,
                content_state_json, started_at_ms, active, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?9)
             ON CONFLICT(push_token) DO UPDATE SET
                user_id = excluded.user_id,
                session_id = excluded.session_id,
                bundle_id = excluded.bundle_id,
                environment = excluded.environment,
                content_state_json = excluded.content_state_json,
                started_at_ms = excluded.started_at_ms,
                active = 1,
                updated_at = excluded.updated_at,
                ended_at = NULL",
            params![
                id,
                registration.user_id,
                registration.session_id,
                registration.push_token,
                registration.bundle_id,
                registration.environment,
                content_state_json,
                registration.started_at_ms,
                now,
            ],
        )?;
        self.db
            .conn()
            .query_row(
                "SELECT id FROM live_activity_tokens WHERE push_token = ?1",
                [registration.push_token],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn active_for_session(
        &self,
        user_id: Option<&str>,
        session_id: &str,
    ) -> Result<Vec<LiveActivityToken>> {
        const COLUMNS: &str = "id, user_id, session_id, push_token, bundle_id, environment,
             content_state_json, started_at_ms, active, created_at, updated_at, ended_at";
        let sql = if user_id.is_some() {
            format!(
                "SELECT {COLUMNS} FROM live_activity_tokens
                 WHERE user_id = ?1 AND session_id = ?2 AND active = 1
                 ORDER BY created_at DESC"
            )
        } else {
            format!(
                "SELECT {COLUMNS} FROM live_activity_tokens
                 WHERE user_id IS NULL AND session_id = ?1 AND active = 1
                 ORDER BY created_at DESC"
            )
        };
        let mut statement = self.db.conn().prepare(&sql)?;
        if let Some(user_id) = user_id {
            statement
                .query_map(params![user_id, session_id], map_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        } else {
            statement
                .query_map([session_id], map_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        }
    }

    pub fn update_state_for_user(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        push_token: &str,
        content_state: &serde_json::Value,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let content_state_json = serde_json::to_string(content_state)?;
        let updated = if let Some(user_id) = user_id {
            self.db.conn().execute(
                "UPDATE live_activity_tokens
                 SET content_state_json = ?1, updated_at = ?2
                 WHERE push_token = ?3 AND session_id = ?4 AND user_id = ?5 AND active = 1",
                params![content_state_json, now, push_token, session_id, user_id],
            )?
        } else {
            self.db.conn().execute(
                "UPDATE live_activity_tokens
                 SET content_state_json = ?1, updated_at = ?2
                 WHERE push_token = ?3 AND session_id = ?4 AND user_id IS NULL AND active = 1",
                params![content_state_json, now, push_token, session_id],
            )?
        };
        Ok(updated > 0)
    }

    pub fn end_for_user(
        &self,
        user_id: Option<&str>,
        session_id: &str,
        push_token: &str,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let updated = if let Some(user_id) = user_id {
            self.db.conn().execute(
                "UPDATE live_activity_tokens
                 SET active = 0, updated_at = ?1, ended_at = ?1
                 WHERE push_token = ?2 AND session_id = ?3 AND user_id = ?4",
                params![now, push_token, session_id, user_id],
            )?
        } else {
            self.db.conn().execute(
                "UPDATE live_activity_tokens
                 SET active = 0, updated_at = ?1, ended_at = ?1
                 WHERE push_token = ?2 AND session_id = ?3 AND user_id IS NULL",
                params![now, push_token, session_id],
            )?
        };
        Ok(updated > 0)
    }

    pub fn end_token(&self, push_token: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE live_activity_tokens
             SET active = 0, updated_at = ?1, ended_at = ?1
             WHERE push_token = ?2",
            params![now, push_token],
        )?;
        Ok(())
    }
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<LiveActivityToken> {
    let content_state_json: String = row.get(6)?;
    let content_state = serde_json::from_str(&content_state_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            content_state_json.len(),
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    Ok(LiveActivityToken {
        id: row.get(0)?,
        user_id: row.get(1)?,
        session_id: row.get(2)?,
        push_token: row.get(3)?,
        bundle_id: row.get(4)?,
        environment: row.get(5)?,
        content_state,
        started_at_ms: row.get(7)?,
        active: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        ended_at: row.get(11)?,
    })
}
