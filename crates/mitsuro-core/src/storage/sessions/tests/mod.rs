use rusqlite::params;
use tempfile::TempDir;

use crate::storage::Database;

pub(super) fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db = Database::new(&db_path).expect("Failed to create database");
    (db, temp_dir)
}

pub(super) fn create_test_user(db: &Database, user_id: &str) {
    db.conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            params![user_id, format!("{user_id}@example.com"), "free"],
        )
        .expect("Failed to create user");
}

mod lifecycle;
mod listing;
mod ownership;
mod runtime;
