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
