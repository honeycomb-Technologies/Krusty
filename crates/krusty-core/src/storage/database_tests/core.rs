use rusqlite::Connection;
use tempfile::TempDir;

use crate::storage::database::Database;

use super::create_test_db;

#[test]
fn test_database_creation() {
    let (db, _temp) = create_test_db();
    let version = db.get_schema_version();
    assert_eq!(version, 39, "Expected current schema version to be 39");
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

    assert_eq!(version, 39, "Expected final schema version");
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
        r#"CREATE TABLE schema_version (
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
        ) VALUES ('checkpoint', 'session', 1, '[1]', 'old history', '[]', CURRENT_TIMESTAMP);"#,
    )
    .expect("seed schema");
    drop(conn);

    let db = Database::new(&db_path).expect("migrate db");
    assert_eq!(db.get_schema_version(), 39);

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

#[test]
fn migration_34_backfills_provider_call_classification() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("provider-traces.db");
    let conn = Connection::open(&db_path).expect("open seed db");
    conn.execute_batch(
        r#"CREATE TABLE schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO schema_version (version) VALUES (33);
        CREATE TABLE runtime_traces (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            sequence INTEGER NOT NULL,
            turn INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            failure_category TEXT,
            stop_reason TEXT,
            created_at TEXT NOT NULL,
            UNIQUE(session_id, sequence)
        );
        INSERT INTO runtime_traces (
            session_id, run_id, sequence, turn, event_type, payload_json, created_at
        ) VALUES (
            'session', 'run', 1, 1, 'provider_call',
            '{"call_kind":"auxiliary","operation":"compaction_summary"}',
            CURRENT_TIMESTAMP
        );"#,
    )
    .expect("seed schema");
    drop(conn);

    let db = Database::new(&db_path).expect("migrate db");
    assert_eq!(db.get_schema_version(), 39);
    let (call_kind, operation): (Option<String>, Option<String>) = db
        .conn()
        .query_row(
            "SELECT call_kind, operation FROM runtime_traces WHERE sequence = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("classification");
    assert_eq!(call_kind.as_deref(), Some("auxiliary"));
    assert_eq!(operation.as_deref(), Some("compaction_summary"));
}

#[test]
fn migrations_35_through_39_create_mako_backend_contracts() {
    let (db, _temp) = create_test_db();
    let expected_tables = [
        "mako_profiles",
        "mako_profile_documents",
        "mako_controllers",
        "mako_schedules",
        "mako_schedule_occurrences",
        "mako_runs",
        "mako_run_attempts",
        "mako_daemon_leases",
        "mako_idempotency_keys",
        "mako_controller_events",
        "conversation_episodes",
        "mako_learning_runs",
        "mako_learning_candidates",
        "agent_memory_revisions",
        "knowledge_snapshots",
    ];

    for table in expected_tables {
        let exists: i64 = db
            .conn()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap_or_else(|error| panic!("query table {table}: {error}"));
        assert_eq!(exists, 1, "missing migrated table {table}");
    }
}

#[test]
fn migration_39_upgrades_legacy_memories_and_separates_generated_snapshot() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("legacy-memory.db");
    let conn = Connection::open(&db_path).expect("open seed db");
    conn.execute_batch(
        r#"CREATE TABLE schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO schema_version (version) VALUES (38);
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
        INSERT INTO agent_memories (id, memory_type, title, content, project_dir)
            VALUES ('fact', 'project', 'Architecture', 'Use durable leases', '/repo');
        INSERT INTO agent_memories (id, memory_type, title, content, project_dir)
            VALUES ('snapshot', 'project', 'Current Snapshot', 'generated', '/repo');"#,
    )
    .expect("seed legacy memories");
    drop(conn);

    let db = Database::new(&db_path).expect("migrate legacy memories");
    assert_eq!(db.get_schema_version(), 39);

    let fact_metadata: (String, String, f64) = db
        .conn()
        .query_row(
            "SELECT namespace, status, confidence FROM agent_memories WHERE id = 'fact'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("legacy fact metadata");
    assert_eq!(fact_metadata, ("shared".into(), "active".into(), 1.0));

    let snapshot_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM knowledge_snapshots WHERE id = 'snapshot'",
            [],
            |row| row.get(0),
        )
        .expect("migrated snapshot");
    assert_eq!(snapshot_count, 1);

    let legacy_snapshot_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM agent_memories WHERE id = 'snapshot'",
            [],
            |row| row.get(0),
        )
        .expect("legacy snapshot removal");
    assert_eq!(legacy_snapshot_count, 0);

    let revision_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM agent_memory_revisions WHERE memory_id = 'fact'",
            [],
            |row| row.get(0),
        )
        .expect("legacy revision seed");
    assert_eq!(revision_count, 1);
}
