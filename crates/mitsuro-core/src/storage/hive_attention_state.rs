use anyhow::Result;
use chrono::Utc;
use rusqlite::params;
use std::collections::HashMap;

use super::database::Database;

#[derive(Debug, Clone)]
pub struct HiveAttentionItemState {
    pub item_id: String,
    pub read: bool,
    pub cleared: bool,
    pub updated_at: String,
}

pub struct HiveAttentionStateStore<'a> {
    db: &'a Database,
}

impl<'a> HiveAttentionStateStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn list_for_user(
        &self,
        user_id: Option<&str>,
    ) -> Result<HashMap<String, HiveAttentionItemState>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT item_id, read, cleared, updated_at
             FROM hive_attention_state
             WHERE user_scope = ?1",
        )?;

        let rows = stmt.query_map([user_scope(user_id)], |row| {
            Ok(HiveAttentionItemState {
                item_id: row.get(0)?,
                read: row.get::<_, i64>(1)? != 0,
                cleared: row.get::<_, i64>(2)? != 0,
                updated_at: row.get(3)?,
            })
        })?;

        let items = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(anyhow::Error::from)?;

        Ok(items
            .into_iter()
            .map(|item| (item.item_id.clone(), item))
            .collect())
    }

    pub fn set_read(&self, user_id: Option<&str>, item_id: &str, read: bool) -> Result<()> {
        self.upsert_state(user_id, item_id, Some(read), None)
    }

    pub fn set_cleared(&self, user_id: Option<&str>, item_id: &str, cleared: bool) -> Result<()> {
        self.upsert_state(user_id, item_id, None, Some(cleared))
    }

    fn upsert_state(
        &self,
        user_id: Option<&str>,
        item_id: &str,
        read: Option<bool>,
        cleared: Option<bool>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.conn().execute(
            "INSERT INTO hive_attention_state (user_scope, item_id, read, cleared, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user_scope, item_id) DO UPDATE SET
                read = COALESCE(?6, hive_attention_state.read),
                cleared = COALESCE(?7, hive_attention_state.cleared),
                updated_at = excluded.updated_at",
            params![
                user_scope(user_id),
                item_id,
                bool_to_i64(read.unwrap_or(false)),
                bool_to_i64(cleared.unwrap_or(false)),
                now,
                read.map(bool_to_i64),
                cleared.map(bool_to_i64),
            ],
        )?;

        Ok(())
    }
}

fn user_scope(user_id: Option<&str>) -> &str {
    user_id.unwrap_or("")
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::storage::Database;

    use super::HiveAttentionStateStore;

    fn create_test_db() -> (Database, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("db should open");
        (db, temp_dir)
    }

    #[test]
    fn persists_read_and_cleared_state_per_user() {
        let (db, _temp_dir) = create_test_db();
        let store = HiveAttentionStateStore::new(&db);

        store
            .set_read(Some("alice"), "item-1", true)
            .expect("read should persist");
        store
            .set_cleared(Some("alice"), "item-1", true)
            .expect("clear should persist");
        store
            .set_read(Some("bob"), "item-1", false)
            .expect("other user should persist");

        let alice = store
            .list_for_user(Some("alice"))
            .expect("alice should load");
        let bob = store.list_for_user(Some("bob")).expect("bob should load");

        assert!(alice.get("item-1").expect("alice state should exist").read);
        assert!(
            alice
                .get("item-1")
                .expect("alice state should exist")
                .cleared
        );
        assert!(!bob.get("item-1").expect("bob state should exist").read);
        assert!(!bob.get("item-1").expect("bob state should exist").cleared);
    }
}
