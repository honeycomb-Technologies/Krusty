use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::storage::database::Database;

use super::model::PushSubscription;

const SELECT_SUBSCRIPTION_COLUMNS: &str = r#"
    SELECT id, user_id, endpoint, p256dh, auth, created_at, last_used_at,
           last_success_at, last_failure_at, last_failure_reason, failure_count
    FROM push_subscriptions
"#;

pub struct PushSubscriptionStore<'a> {
    db: &'a Database,
}

impl<'a> PushSubscriptionStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn upsert(
        &self,
        user_id: Option<&str>,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO push_subscriptions (id, user_id, endpoint, p256dh, auth, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(endpoint) DO UPDATE SET
                user_id = excluded.user_id,
                p256dh = excluded.p256dh,
                auth = excluded.auth,
                last_failure_at = NULL,
                last_failure_reason = NULL,
                failure_count = 0",
            params![id, user_id, endpoint, p256dh, auth, now],
        )?;

        Ok(id)
    }

    pub fn remove_by_endpoint(&self, endpoint: &str) -> Result<bool> {
        let rows = self.db.conn().execute(
            "DELETE FROM push_subscriptions WHERE endpoint = ?1",
            [endpoint],
        )?;
        Ok(rows > 0)
    }

    pub fn remove_by_endpoint_for_user(
        &self,
        user_id: Option<&str>,
        endpoint: &str,
    ) -> Result<bool> {
        let rows = match user_id {
            Some(uid) => self.db.conn().execute(
                "DELETE FROM push_subscriptions
                 WHERE endpoint = ?1 AND user_id = ?2",
                params![endpoint, uid],
            )?,
            None => self.db.conn().execute(
                "DELETE FROM push_subscriptions
                 WHERE endpoint = ?1 AND user_id IS NULL",
                [endpoint],
            )?,
        };
        Ok(rows > 0)
    }

    pub fn get_for_user(&self, user_id: &str) -> Result<Vec<PushSubscription>> {
        let sql = format!("{SELECT_SUBSCRIPTION_COLUMNS} WHERE user_id = ?1");
        let mut stmt = self.db.conn().prepare(&sql)?;
        let subs = stmt.query_map([user_id], map_subscription_row)?;
        subs.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_all(&self) -> Result<Vec<PushSubscription>> {
        let mut stmt = self.db.conn().prepare(SELECT_SUBSCRIPTION_COLUMNS)?;
        let subs = stmt.query_map([], map_subscription_row)?;
        subs.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn mark_success(&self, endpoint: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE push_subscriptions
             SET last_used_at = ?1,
                 last_success_at = ?1,
                 last_failure_at = NULL,
                 last_failure_reason = NULL,
                 failure_count = 0
             WHERE endpoint = ?2",
            params![now, endpoint],
        )?;
        Ok(())
    }

    pub fn mark_failure(&self, endpoint: &str, reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE push_subscriptions
             SET last_failure_at = ?1,
                 last_failure_reason = ?2,
                 failure_count = failure_count + 1
             WHERE endpoint = ?3",
            params![now, reason, endpoint],
        )?;
        Ok(())
    }

    pub fn count_for_user(&self, user_id: Option<&str>) -> Result<usize> {
        let count: i64 =
            match user_id {
                Some(uid) => self.db.conn().query_row(
                    "SELECT COUNT(*) FROM push_subscriptions WHERE user_id = ?1",
                    [uid],
                    |row| row.get(0),
                )?,
                None => self.db.conn().query_row(
                    "SELECT COUNT(*) FROM push_subscriptions",
                    [],
                    |row| row.get(0),
                )?,
            };

        Ok(count as usize)
    }
}

fn map_subscription_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PushSubscription> {
    Ok(PushSubscription {
        id: row.get(0)?,
        user_id: row.get(1)?,
        endpoint: row.get(2)?,
        p256dh: row.get(3)?,
        auth: row.get(4)?,
        created_at: row.get(5)?,
        last_used_at: row.get(6)?,
        last_success_at: row.get(7)?,
        last_failure_at: row.get(8)?,
        last_failure_reason: row.get(9)?,
        failure_count: row.get(10)?,
    })
}
