use super::create_test_db;

#[test]
fn test_pinch_metadata_table_migration() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='pinch_metadata'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"pinch_metadata".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(pinch_metadata)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"source_session_id".to_string()));
    assert!(columns.contains(&"target_session_id".to_string()));
    assert!(columns.contains(&"summary".to_string()));
}

#[test]
fn test_block_ui_state_table_migration() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='block_ui_state'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"block_ui_state".to_string()));
}

#[test]
fn test_file_activity_table_migration() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='file_activity'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"file_activity".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(file_activity)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"read_count".to_string()));
    assert!(columns.contains(&"write_count".to_string()));
    assert!(columns.contains(&"edit_count".to_string()));
    assert!(columns.contains(&"last_accessed".to_string()));
}

#[test]
fn test_agent_state_columns_migration() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("PRAGMA table_info(sessions)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"agent_state".to_string()));
    assert!(columns.contains(&"agent_started_at".to_string()));
    assert!(columns.contains(&"agent_last_event_at".to_string()));
}

#[test]
fn test_token_count_column_migration() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("PRAGMA table_info(sessions)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"token_count".to_string()));
}

#[test]
fn provider_aware_model_identity_columns_are_additive() {
    let (db, _temp) = create_test_db();
    let columns: Vec<String> = db
        .conn()
        .prepare("PRAGMA table_info(sessions)")
        .expect("prepare session columns")
        .query_map([], |row| row.get(1))
        .expect("query session columns")
        .collect::<rusqlite::Result<_>>()
        .expect("collect session columns");

    assert!(columns.contains(&"model".to_string()));
    assert!(columns.contains(&"model_key_json".to_string()));
    assert!(columns.contains(&"model_catalog_revision".to_string()));

    let schedule_columns: Vec<String> = db
        .conn()
        .prepare("PRAGMA table_info(mako_schedules)")
        .expect("prepare Mako schedule columns")
        .query_map([], |row| row.get(1))
        .expect("query Mako schedule columns")
        .collect::<rusqlite::Result<_>>()
        .expect("collect Mako schedule columns");
    assert!(schedule_columns.contains(&"model".to_string()));
    assert!(schedule_columns.contains(&"model_key_json".to_string()));
    assert!(schedule_columns.contains(&"model_catalog_revision".to_string()));
}

#[test]
fn migration_45_upgrades_schema_44_without_rewriting_legacy_model() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-44.db");
    let conn = rusqlite::Connection::open(&path).expect("seed database");
    conn.execute_batch(
        "CREATE TABLE schema_version (
             version INTEGER PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         INSERT INTO schema_version (version) VALUES (44);
         CREATE TABLE sessions (
             id TEXT PRIMARY KEY,
             model TEXT
         );
         INSERT INTO sessions (id, model) VALUES ('legacy', 'shared-model');",
    )
    .expect("seed schema 44");
    drop(conn);

    let db = crate::storage::database::Database::new(&path).expect("migrate schema 44");
    assert_eq!(db.get_schema_version(), 46);
    let row: (Option<String>, Option<String>, Option<String>) = db
        .conn()
        .query_row(
            "SELECT model, model_key_json, model_catalog_revision FROM sessions WHERE id = 'legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read migrated session");
    assert_eq!(row.0.as_deref(), Some("shared-model"));
    assert!(row.1.is_none());
    assert!(row.2.is_none());
}

#[test]
fn migration_46_upgrades_schema_45_without_guessing_legacy_schedule_identity() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-45.db");
    let conn = rusqlite::Connection::open(&path).expect("seed database");
    conn.execute_batch(
        "CREATE TABLE schema_version (
             version INTEGER PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         INSERT INTO schema_version (version) VALUES (45);
         CREATE TABLE mako_schedules (
             id TEXT PRIMARY KEY,
             model TEXT
         );
         INSERT INTO mako_schedules (id, model) VALUES ('legacy', 'shared-model');",
    )
    .expect("seed schema 45");
    drop(conn);

    let db = crate::storage::database::Database::new(&path).expect("migrate schema 45");
    assert_eq!(db.get_schema_version(), 46);
    let row: (Option<String>, Option<String>, Option<String>) = db
        .conn()
        .query_row(
            "SELECT model, model_key_json, model_catalog_revision
               FROM mako_schedules WHERE id = 'legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read migrated schedule");
    assert_eq!(row.0.as_deref(), Some("shared-model"));
    assert!(row.1.is_none());
    assert!(row.2.is_none());
}

#[test]
fn test_push_delivery_tables_migration() {
    let (db, _temp) = create_test_db();
    let conn = db.conn();

    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='push_delivery_attempts'",
        )
        .expect("Failed to prepare query");
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();
    assert!(tables.contains(&"push_delivery_attempts".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(push_subscriptions)")
        .expect("Failed to prepare PRAGMA");
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"last_success_at".to_string()));
    assert!(columns.contains(&"last_failure_at".to_string()));
    assert!(columns.contains(&"last_failure_reason".to_string()));
    assert!(columns.contains(&"failure_count".to_string()));
}
