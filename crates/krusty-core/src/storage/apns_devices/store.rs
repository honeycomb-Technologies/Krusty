use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Row};

use super::model::{ApnsDevice, ApnsDeviceRegistration};
use crate::storage::database::Database;

pub struct ApnsDeviceStore<'a> {
    db: &'a Database,
}

impl<'a> ApnsDeviceStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert or update a device registration (upsert on device_token).
    pub fn upsert(&self, registration: ApnsDeviceRegistration<'_>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO apns_devices (
                id, user_id, device_token, bundle_id, notification_level,
                environment, created_at, last_registered_at, enabled
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 1)
             ON CONFLICT(device_token) DO UPDATE SET
                user_id = excluded.user_id,
                bundle_id = excluded.bundle_id,
                notification_level = excluded.notification_level,
                environment = excluded.environment,
                last_registered_at = excluded.last_registered_at,
                enabled = 1,
                last_failure_at = NULL,
                last_failure_reason = NULL,
                failure_count = 0",
            params![
                id,
                registration.user_id,
                registration.device_token,
                registration.bundle_id,
                registration.notification_level,
                registration.environment,
                now
            ],
        )?;

        self.db
            .conn()
            .query_row(
                "SELECT id FROM apns_devices WHERE device_token = ?1",
                [registration.device_token],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Get all devices for a given user (or all if user_id is None).
    pub fn get_for_user(&self, user_id: Option<&str>) -> Result<Vec<ApnsDevice>> {
        let conn = self.db.conn();

        let (sql, bind_user) = if user_id.is_some() {
            (
                "SELECT id, user_id, device_token, bundle_id,
                        notification_level, environment, last_registered_at, enabled,
                        created_at,
                        last_used_at, last_success_at, last_failure_at,
                        last_failure_reason, failure_count
                 FROM apns_devices WHERE user_id = ?1 AND enabled = 1
                 ORDER BY created_at DESC",
                true,
            )
        } else {
            (
                "SELECT id, user_id, device_token, bundle_id,
                        notification_level, environment, last_registered_at, enabled,
                        created_at,
                        last_used_at, last_success_at, last_failure_at,
                        last_failure_reason, failure_count
                 FROM apns_devices WHERE user_id IS NULL AND enabled = 1
                 ORDER BY created_at DESC",
                false,
            )
        };

        let mut stmt = conn.prepare(sql)?;

        let rows: Vec<ApnsDevice> = if bind_user {
            stmt.query_map(params![user_id], map_device_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(params![], map_device_row)?
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(rows)
    }

    /// Remove a device by token.
    pub fn remove_by_token_for_user(
        &self,
        user_id: Option<&str>,
        device_token: &str,
    ) -> Result<bool> {
        let removed = match user_id {
            Some(user_id) => self.db.conn().execute(
                "DELETE FROM apns_devices WHERE device_token = ?1 AND user_id = ?2",
                params![device_token, user_id],
            )?,
            None => self.db.conn().execute(
                "DELETE FROM apns_devices
                 WHERE device_token = ?1 AND user_id IS NULL",
                params![device_token],
            )?,
        };
        Ok(removed > 0)
    }

    /// Record a successful delivery.
    pub fn mark_success(&self, device_token: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE apns_devices SET last_used_at = ?1, last_success_at = ?1,
             failure_count = 0 WHERE device_token = ?2",
            params![now, device_token],
        )?;
        Ok(())
    }

    /// Record a delivery failure.
    pub fn mark_failure(&self, device_token: &str, reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE apns_devices SET last_used_at = ?1, last_failure_at = ?1,
             last_failure_reason = ?2, failure_count = failure_count + 1
             WHERE device_token = ?3",
            params![now, reason, device_token],
        )?;
        Ok(())
    }

    /// Remove stale devices (too many consecutive failures).
    pub fn remove_stale(&self, max_failures: i64) -> Result<usize> {
        let removed = self.db.conn().execute(
            "DELETE FROM apns_devices WHERE failure_count >= ?1",
            params![max_failures],
        )?;
        Ok(removed)
    }

    pub fn count_for_user(&self, user_id: Option<&str>) -> Result<usize> {
        let count: i64 = if let Some(uid) = user_id {
            self.db.conn().query_row(
                "SELECT COUNT(*) FROM apns_devices WHERE user_id = ?1 AND enabled = 1",
                params![uid],
                |row| row.get(0),
            )?
        } else {
            self.db.conn().query_row(
                "SELECT COUNT(*) FROM apns_devices WHERE user_id IS NULL AND enabled = 1",
                params![],
                |row| row.get(0),
            )?
        };
        Ok(count as usize)
    }
}

fn map_device_row(row: &Row<'_>) -> rusqlite::Result<ApnsDevice> {
    Ok(ApnsDevice {
        id: row.get(0)?,
        user_id: row.get(1)?,
        device_token: row.get(2)?,
        bundle_id: row.get(3)?,
        notification_level: row.get(4)?,
        environment: row.get(5)?,
        last_registered_at: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        last_used_at: row.get(9)?,
        last_success_at: row.get(10)?,
        last_failure_at: row.get(11)?,
        last_failure_reason: row.get(12)?,
        failure_count: row.get(13)?,
    })
}
