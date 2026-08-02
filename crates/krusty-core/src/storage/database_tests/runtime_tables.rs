use super::create_test_db;

#[test]
fn test_runtime_traces_table_exists() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='runtime_traces'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"runtime_traces".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(runtime_traces)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"session_id".to_string()));
    assert!(columns.contains(&"run_id".to_string()));
    assert!(columns.contains(&"sequence".to_string()));
    assert!(columns.contains(&"turn".to_string()));
    assert!(columns.contains(&"event_type".to_string()));
    assert!(columns.contains(&"payload_json".to_string()));
    assert!(columns.contains(&"failure_category".to_string()));
    assert!(columns.contains(&"stop_reason".to_string()));
}

#[test]
fn test_delegated_runs_table_exists() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='delegated_runs'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"delegated_runs".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(delegated_runs)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"delegated_run_id".to_string()));
    assert!(columns.contains(&"parent_session_id".to_string()));
    assert!(columns.contains(&"role".to_string()));
    assert!(columns.contains(&"stage".to_string()));
    assert!(columns.contains(&"target_scope_json".to_string()));
    assert!(columns.contains(&"snapshot_json".to_string()));
    assert!(columns.contains(&"artifact_json".to_string()));

    let continuation_table: String = conn
        .query_row(
            "SELECT name FROM sqlite_master
              WHERE type = 'table' AND name = 'delegated_run_continuations'",
            [],
            |row| row.get(0),
        )
        .expect("delegated continuation claim table should exist");
    assert_eq!(continuation_table, "delegated_run_continuations");
}

#[test]
fn test_mako_runtime_state_table_exists() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='mako_runtime_state'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"mako_runtime_state".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(mako_runtime_state)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"session_id".to_string()));
    assert!(columns.contains(&"status".to_string()));
    assert!(columns.contains(&"next_wake_at".to_string()));
    assert!(columns.contains(&"sleep_reason".to_string()));
    assert!(columns.contains(&"last_error".to_string()));
    assert!(columns.contains(&"current_run_id".to_string()));
    assert!(columns.contains(&"last_wake_reason".to_string()));
    assert!(columns.contains(&"crew_slug".to_string()));
    assert!(columns.contains(&"priority".to_string()));
    assert!(columns.contains(&"updated_at".to_string()));
}

#[test]
fn test_mako_attention_state_table_exists() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='mako_attention_state'",
        )
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"mako_attention_state".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(mako_attention_state)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"user_scope".to_string()));
    assert!(columns.contains(&"item_id".to_string()));
    assert!(columns.contains(&"read".to_string()));
    assert!(columns.contains(&"cleared".to_string()));
    assert!(columns.contains(&"updated_at".to_string()));
}
