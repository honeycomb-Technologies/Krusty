use anyhow::Result;

use crate::storage::database::Database;

pub struct Preferences {
    pub(super) db: Database,
    pub(super) user_id: Option<String>,
}

impl Preferences {
    pub fn new(db: Database) -> Self {
        Self { db, user_id: None }
    }

    pub fn for_user(db: Database, user_id: &str) -> Self {
        Self {
            db,
            user_id: Some(user_id.to_string()),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(uid) = self.user_id.as_deref() {
            self.db
                .conn()
                .query_row(
                    "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = ?2",
                    rusqlite::params![uid, key],
                    |row| row.get(0),
                )
                .ok()
        } else {
            self.db
                .conn()
                .query_row(
                    "SELECT value FROM user_preferences WHERE user_id IS NULL AND key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .ok()
        }
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let user_id = self.user_id.as_deref();
        self.db.conn().execute(
            "INSERT INTO user_preferences (key, value, updated_at, user_id)
             VALUES (?1, ?2, strftime('%s', 'now'), ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = strftime('%s', 'now'), user_id = ?3",
            rusqlite::params![key, value, user_id],
        )?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        if let Some(uid) = self.user_id.as_deref() {
            self.db.conn().execute(
                "DELETE FROM user_preferences WHERE user_id = ?1 AND key = ?2",
                rusqlite::params![uid, key],
            )?;
        } else {
            self.db.conn().execute(
                "DELETE FROM user_preferences WHERE user_id IS NULL AND key = ?1",
                [key],
            )?;
        }

        Ok(())
    }
}
