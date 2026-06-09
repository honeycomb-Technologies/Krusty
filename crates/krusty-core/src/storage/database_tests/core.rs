use tempfile::TempDir;

use crate::storage::database::Database;

use super::create_test_db;

#[test]
fn test_database_creation() {
    let (db, _temp) = create_test_db();
    let version = db.get_schema_version();
    assert_eq!(version, 31, "Expected current schema version to be 31");
}

#[test]
fn test_foreign_keys_enabled() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("PRAGMA foreign_keys")
        .expect("Failed to prepare PRAGMA");

    let fk_enabled: i32 = stmt
        .query_row([], |row| row.get(0))
        .expect("Failed to get foreign_keys setting");

    assert_eq!(fk_enabled, 1, "Foreign keys should be enabled");
}

#[test]
fn test_wal_mode_enabled() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("PRAGMA journal_mode")
        .expect("Failed to prepare PRAGMA");

    let journal_mode: String = stmt
        .query_row([], |row| row.get(0))
        .expect("Failed to get journal_mode");

    assert_eq!(
        journal_mode.to_lowercase(),
        "wal",
        "WAL mode should be enabled"
    );
}

#[test]
fn test_schema_version_increments() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");

    let db = Database::new(&db_path).expect("Failed to create database");
    let version = db.get_schema_version();

    assert_eq!(version, 31, "Expected final schema version");
}

#[test]
fn test_migration_idempotency() {
    let (db, _temp) = create_test_db();

    let version1 = db.get_schema_version();
    db.run_migrations().expect("Re-running migrations failed");
    let version2 = db.get_schema_version();

    assert_eq!(version1, version2, "Schema version should not change");
}
