use super::create_test_db;

#[test]
fn test_sessions_table_exists() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"sessions".to_string()));

    let mut stmt = conn
        .prepare("PRAGMA table_info(sessions)")
        .expect("Failed to prepare PRAGMA");

    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Failed to get columns")
        .filter_map(Result::ok)
        .collect();

    assert!(columns.contains(&"id".to_string()));
    assert!(columns.contains(&"title".to_string()));
    assert!(columns.contains(&"created_at".to_string()));
    assert!(columns.contains(&"updated_at".to_string()));
    assert!(columns.contains(&"user_id".to_string()));
    assert!(columns.contains(&"working_dir".to_string()));
    assert!(columns.contains(&"session_type".to_string()));
    assert!(columns.contains(&"work_mode".to_string()));
    assert!(columns.contains(&"target_branch".to_string()));
    assert!(columns.contains(&"context_ledger_json".to_string()));
    assert!(columns.contains(&"continuation_json".to_string()));
    assert!(columns.contains(&"recovery_json".to_string()));
}

#[test]
fn test_messages_table_exists() {
    let (db, _temp) = create_test_db();

    let conn = db.conn();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='messages'")
        .expect("Failed to prepare query");

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to query tables")
        .filter_map(Result::ok)
        .collect();

    assert!(tables.contains(&"messages".to_string()));

    let mut stmt = conn
        .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='messages'")
        .expect("Failed to get table DDL");

    let ddl: Option<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("Failed to get DDL")
        .filter_map(Result::ok)
        .next();

    let ddl = ddl.expect("Messages table should exist");
    assert!(ddl.contains("FOREIGN KEY"));
    assert!(ddl.contains("ON DELETE CASCADE"));
}

#[test]
fn canonical_workflow_tables_and_invariants_exist() {
    let (db, _temp) = create_test_db();
    let required_tables = [
        "workflow_goals",
        "workflow_goal_criteria",
        "workflow_plan_revisions",
        "workflow_plan_steps",
        "workflow_step_dependencies",
        "workflow_execution_attempts",
        "workflow_events",
        "workflow_idempotency",
    ];
    for table in required_tables {
        let exists: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query workflow table");
        assert_eq!(exists, 1, "missing workflow table {table}");
    }

    let indexes: Vec<String> = db
        .conn()
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'index' AND name LIKE 'idx_workflow_%'",
        )
        .expect("prepare workflow indexes")
        .query_map([], |row| row.get(0))
        .expect("query workflow indexes")
        .collect::<rusqlite::Result<_>>()
        .expect("collect workflow indexes");
    assert!(indexes.contains(&"idx_workflow_goals_one_unfinished".to_string()));
    assert!(indexes.contains(&"idx_workflow_one_serial_step".to_string()));
    assert!(indexes.contains(&"idx_workflow_one_running_attempt".to_string()));
}
