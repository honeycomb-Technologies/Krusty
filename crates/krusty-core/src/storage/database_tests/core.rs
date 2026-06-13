use rusqlite::Connection;
use tempfile::TempDir;

use crate::storage::database::Database;

use super::create_test_db;

#[test]
fn test_database_creation() {
    let (db, _temp) = create_test_db();
    let version = db.get_schema_version();
    assert_eq!(version, 33, "Expected current schema version to be 33");
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

    assert_eq!(version, 33, "Expected final schema version");
}

#[test]
fn test_migration_idempotency() {
    let (db, _temp) = create_test_db();

    let version1 = db.get_schema_version();
    db.run_migrations().expect("Re-running migrations failed");
    let version2 = db.get_schema_version();

    assert_eq!(version1, version2, "Schema version should not change");
}

#[test]
fn migration_33_removes_legacy_compaction_memory_and_duplicate_history() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let conn = Connection::open(&db_path).expect("open seed db");
    conn.execute_batch(
        "CREATE TABLE schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO schema_version (version) VALUES (32);
        CREATE TABLE agent_memories (
            id TEXT PRIMARY KEY,
            memory_type TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            project_dir TEXT,
            user_id TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO agent_memories (id, memory_type, title, content)
            VALUES ('flush', 'project', 'Compaction flush #1', 'old transcript');
        INSERT INTO agent_memories (id, memory_type, title, content)
            VALUES ('keep', 'project', 'Architecture', 'durable fact');
        CREATE TABLE compaction_checkpoints (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            prompt_index_at_compaction INTEGER NOT NULL,
            pre_compact_message_ids_json TEXT NOT NULL,
            compacted_history_json TEXT NOT NULL,
            original_user_info TEXT,
            reread_file_paths_json TEXT NOT NULL DEFAULT '[]',
            schema_version INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL
        );
        INSERT INTO compaction_checkpoints (
            id, session_id, prompt_index_at_compaction, pre_compact_message_ids_json,
            compacted_history_json, reread_file_paths_json, created_at
        ) VALUES ('checkpoint', 'session', 1, '[1]', 'old history', '[]', CURRENT_TIMESTAMP);",
    )
    .expect("seed schema");
    drop(conn);

    let db = Database::new(&db_path).expect("migrate db");
    assert_eq!(db.get_schema_version(), 33);

    let flush_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM agent_memories WHERE title LIKE 'Compaction flush #%';",
            [],
            |row| row.get(0),
        )
        .expect("flush count");
    assert_eq!(flush_count, 0);

    let kept_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM agent_memories WHERE title = 'Architecture';",
            [],
            |row| row.get(0),
        )
        .expect("kept count");
    assert_eq!(kept_count, 1);

    let compacted_history: String = db
        .conn()
        .query_row(
            "SELECT compacted_history_json FROM compaction_checkpoints WHERE id = 'checkpoint';",
            [],
            |row| row.get(0),
        )
        .expect("history json");
    assert_eq!(compacted_history, "[]");
}
