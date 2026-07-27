use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use crate::storage::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpoPushDevice {
    pub id: String,
    pub user_id: Option<String>,
    pub expo_push_token: String,
    pub platform: String,
    pub notification_level: String,
    pub enabled: bool,
    pub created_at: String,
    pub last_registered_at: String,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_failure_reason: Option<String>,
    pub failure_count: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct ExpoPushDeviceRegistration<'a> {
    pub user_id: Option<&'a str>,
    pub expo_push_token: &'a str,
    pub platform: &'a str,
    pub notification_level: &'a str,
}

pub struct ExpoPushDeviceStore<'a> {
    db: &'a Database,
}

impl<'a> ExpoPushDeviceStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn upsert(&self, registration: ExpoPushDeviceRegistration<'_>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "INSERT INTO expo_push_devices (
                id, user_id, expo_push_token, platform, notification_level,
                enabled, created_at, last_registered_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)
             ON CONFLICT(expo_push_token) DO UPDATE SET
                user_id = excluded.user_id,
                platform = excluded.platform,
                notification_level = excluded.notification_level,
                enabled = 1,
                last_registered_at = excluded.last_registered_at,
                last_failure_at = NULL,
                last_failure_reason = NULL,
                failure_count = 0",
            params![
                id,
                registration.user_id,
                registration.expo_push_token,
                registration.platform,
                registration.notification_level,
                now,
            ],
        )?;
        self.db
            .conn()
            .query_row(
                "SELECT id FROM expo_push_devices WHERE expo_push_token = ?1",
                [registration.expo_push_token],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn get_for_user(&self, user_id: Option<&str>) -> Result<Vec<ExpoPushDevice>> {
        const COLUMNS: &str = "id, user_id, expo_push_token, platform, notification_level, enabled,
             created_at, last_registered_at, last_success_at, last_failure_at,
             last_failure_reason, failure_count";
        let sql = if user_id.is_some() {
            format!(
                "SELECT {COLUMNS} FROM expo_push_devices
                 WHERE user_id = ?1 AND enabled = 1 ORDER BY created_at DESC"
            )
        } else {
            format!(
                "SELECT {COLUMNS} FROM expo_push_devices
                 WHERE user_id IS NULL AND enabled = 1 ORDER BY created_at DESC"
            )
        };
        let mut statement = self.db.conn().prepare(&sql)?;
        let rows = if let Some(user_id) = user_id {
            statement.query_map([user_id], map_row)?
        } else {
            statement.query_map([], map_row)?
        };
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn mark_success(&self, token: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE expo_push_devices
             SET last_success_at = ?1, failure_count = 0
             WHERE expo_push_token = ?2",
            params![now, token],
        )?;
        Ok(())
    }

    pub fn mark_failure(&self, token: &str, reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE expo_push_devices
             SET last_failure_at = ?1, last_failure_reason = ?2,
                 failure_count = failure_count + 1
             WHERE expo_push_token = ?3",
            params![now, reason, token],
        )?;
        Ok(())
    }

    pub fn remove_for_user(&self, user_id: Option<&str>, token: &str) -> Result<bool> {
        let removed = if let Some(user_id) = user_id {
            self.db.conn().execute(
                "DELETE FROM expo_push_devices
                 WHERE expo_push_token = ?1 AND user_id = ?2",
                params![token, user_id],
            )?
        } else {
            self.db.conn().execute(
                "DELETE FROM expo_push_devices
                 WHERE expo_push_token = ?1 AND user_id IS NULL",
                [token],
            )?
        };
        Ok(removed > 0)
    }

    pub fn count_for_user(&self, user_id: Option<&str>) -> Result<usize> {
        let count: i64 = if let Some(user_id) = user_id {
            self.db.conn().query_row(
                "SELECT COUNT(*) FROM expo_push_devices
                 WHERE user_id = ?1 AND enabled = 1",
                [user_id],
                |row| row.get(0),
            )?
        } else {
            self.db.conn().query_row(
                "SELECT COUNT(*) FROM expo_push_devices
                 WHERE user_id IS NULL AND enabled = 1",
                [],
                |row| row.get(0),
            )?
        };
        Ok(count.max(0) as usize)
    }

    pub fn remove_platform_for_user(&self, user_id: Option<&str>, platform: &str) -> Result<usize> {
        let removed = if let Some(user_id) = user_id {
            self.db.conn().execute(
                "DELETE FROM expo_push_devices
                 WHERE user_id = ?1 AND platform = ?2",
                params![user_id, platform],
            )?
        } else {
            self.db.conn().execute(
                "DELETE FROM expo_push_devices
                 WHERE user_id IS NULL AND platform = ?1",
                [platform],
            )?
        };
        Ok(removed)
    }
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<ExpoPushDevice> {
    Ok(ExpoPushDevice {
        id: row.get(0)?,
        user_id: row.get(1)?,
        expo_push_token: row.get(2)?,
        platform: row.get(3)?,
        notification_level: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
        last_registered_at: row.get(7)?,
        last_success_at: row.get(8)?,
        last_failure_at: row.get(9)?,
        last_failure_reason: row.get(10)?,
        failure_count: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{ExpoPushDeviceRegistration, ExpoPushDeviceStore};
    use crate::storage::Database;
    use tempfile::TempDir;

    #[test]
    fn platform_cleanup_is_owner_scoped() {
        let temp = TempDir::new().unwrap();
        let db = Database::new(&temp.path().join("expo-devices.db")).unwrap();
        let store = ExpoPushDeviceStore::new(&db);
        for (user_id, token, platform) in [
            (Some("user-1"), "ExpoPushToken[user-1-ios]", "ios"),
            (Some("user-1"), "ExpoPushToken[user-1-android]", "android"),
            (Some("user-2"), "ExpoPushToken[user-2-ios]", "ios"),
        ] {
            store
                .upsert(ExpoPushDeviceRegistration {
                    user_id,
                    expo_push_token: token,
                    platform,
                    notification_level: "important",
                })
                .unwrap();
        }

        assert_eq!(
            store
                .remove_platform_for_user(Some("user-1"), "ios")
                .unwrap(),
            1
        );
        let user_one = store.get_for_user(Some("user-1")).unwrap();
        assert_eq!(user_one.len(), 1);
        assert_eq!(user_one[0].platform, "android");
        assert_eq!(store.get_for_user(Some("user-2")).unwrap().len(), 1);
    }
}
