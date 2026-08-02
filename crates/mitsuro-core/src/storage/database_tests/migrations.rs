use super::{create_test_db, seed_legacy_delegated_runs_schema};

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
        .prepare("PRAGMA table_info(hive_schedules)")
        .expect("prepare Hive schedule columns")
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
    seed_legacy_delegated_runs_schema(&conn);
    drop(conn);

    let db = crate::storage::database::Database::new(&path).expect("migrate schema 44");
    assert_eq!(db.get_schema_version(), 55);
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
    seed_legacy_delegated_runs_schema(&conn);
    drop(conn);

    let db = crate::storage::database::Database::new(&path).expect("migrate schema 45");
    assert_eq!(db.get_schema_version(), 55);
    let row: (Option<String>, Option<String>, Option<String>) = db
        .conn()
        .query_row(
            "SELECT model, model_key_json, model_catalog_revision
               FROM hive_schedules WHERE id = 'legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read migrated schedule");
    assert_eq!(row.0.as_deref(), Some("shared-model"));
    assert!(row.1.is_none());
    assert!(row.2.is_none());
}

#[test]
fn migration_52_backfills_one_deterministic_claim_and_is_idempotent() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-51-continuations.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    db.conn()
        .execute_batch(
            r#"
            INSERT INTO sessions (id, title, created_at, updated_at)
            VALUES ('parent', 'Parent', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z');

            DROP TABLE delegated_run_continuations;
            DELETE FROM schema_version WHERE version >= 52;

            INSERT INTO delegated_runs (
                delegated_run_id, parent_session_id, role, stage, resumable,
                resumed_from_run_id, target_scope_key, target_scope_json,
                created_at, updated_at
            ) VALUES (
                'origin', 'parent', 'explore', 'complete', 1,
                NULL, 'project', '[]',
                '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z'
            );
            INSERT INTO delegated_runs (
                delegated_run_id, parent_session_id, role, stage, resumable,
                resumed_from_run_id, target_scope_key, target_scope_json,
                created_at, updated_at
            ) VALUES (
                'continuation-later', 'parent', 'explore', 'complete', 1,
                'origin', 'project', '[]',
                '2026-08-01T00:02:00Z', '2026-08-01T00:02:00Z'
            );
            INSERT INTO delegated_runs (
                delegated_run_id, parent_session_id, role, stage, resumable,
                resumed_from_run_id, target_scope_key, target_scope_json,
                created_at, updated_at
            ) VALUES (
                'continuation-first', 'parent', 'explore', 'complete', 1,
                'origin', 'project', '[]',
                '2026-08-01T00:01:00Z', '2026-08-01T00:01:00Z'
            );
            "#,
        )
        .expect("seed schema-51 duplicate continuation history");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 52");
    assert_eq!(db.get_schema_version(), 55);
    let claimed: String = db
        .conn()
        .query_row(
            "SELECT delegated_run_id FROM delegated_run_continuations WHERE resumed_from_run_id = 'origin'",
            [],
            |row| row.get(0),
        )
        .expect("read migrated continuation claim");
    assert_eq!(claimed, "continuation-first");

    let duplicate = db.conn().execute(
        "INSERT INTO delegated_run_continuations (resumed_from_run_id, delegated_run_id, created_at) VALUES ('origin', 'continuation-later', '2026-08-01T00:03:00Z')",
        [],
    );
    assert!(duplicate.is_err(), "one origin must have only one claim");

    db.conn()
        .execute("DELETE FROM schema_version WHERE version >= 52", [])
        .expect("rewind migration marker");
    db.run_migrations().expect("reapply migration 52");
    let claim_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM delegated_run_continuations WHERE resumed_from_run_id = 'origin'",
            [],
            |row| row.get(0),
        )
        .expect("count idempotent claims");
    assert_eq!(claim_count, 1);
    assert_eq!(db.get_schema_version(), 55);
}

#[test]
fn migration_53_adds_durable_background_wake_intent_idempotently() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-52-wake-intent.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    db.conn()
        .execute_batch(
            r#"
            INSERT INTO sessions (id, title, created_at, updated_at)
            VALUES ('parent-wake', 'Parent', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z');
            INSERT INTO delegated_runs (
                delegated_run_id, parent_session_id, role, stage, resumable,
                target_scope_key, target_scope_json, created_at, updated_at
            ) VALUES (
                'legacy-foreground', 'parent-wake', 'explore', 'complete', 1,
                'project', '[]', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z'
            );
            DROP INDEX idx_delegated_runs_expired_host_lease;
            ALTER TABLE delegated_runs DROP COLUMN host_lease_expires_at_ms;
            ALTER TABLE delegated_runs DROP COLUMN host_owner_id;
            DROP INDEX idx_delegated_runs_unqueued_wake;
            ALTER TABLE delegated_runs DROP COLUMN wake_parent;
            DELETE FROM schema_version WHERE version >= 53;
            "#,
        )
        .expect("rewind durable wake intent migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 53");
    assert_eq!(db.get_schema_version(), 55);
    let wake_parent: i64 = db
        .conn()
        .query_row(
            "SELECT wake_parent FROM delegated_runs WHERE delegated_run_id = 'legacy-foreground'",
            [],
            |row| row.get(0),
        )
        .expect("read migrated wake intent");
    assert_eq!(
        wake_parent, 0,
        "legacy runs must not invent background intent"
    );
    let index_exists: i64 = db
        .conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_delegated_runs_unqueued_wake')",
            [],
            |row| row.get(0),
        )
        .expect("read wake index");
    assert_eq!(index_exists, 1);
    db.run_migrations().expect("migration 53 is idempotent");
    assert_eq!(db.get_schema_version(), 55);
}

#[test]
fn migration_54_adds_conservative_background_host_leases_idempotently() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-53-host-lease.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    db.conn()
        .execute_batch(
            r#"
            INSERT INTO sessions (id, title, created_at, updated_at)
            VALUES ('lease-parent', 'Parent', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z');
            INSERT INTO delegated_runs (
                delegated_run_id, parent_session_id, role, stage, resumable,
                target_scope_key, target_scope_json, wake_parent, created_at, updated_at
            ) VALUES (
                'legacy-background', 'lease-parent', 'explore', 'running', 1,
                'project', '[]', 1, '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z'
            );
            DROP INDEX idx_delegated_runs_expired_host_lease;
            ALTER TABLE delegated_runs DROP COLUMN host_lease_expires_at_ms;
            ALTER TABLE delegated_runs DROP COLUMN host_owner_id;
            DELETE FROM schema_version WHERE version >= 54;
            "#,
        )
        .expect("rewind background host lease migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 54");
    assert_eq!(db.get_schema_version(), 55);
    let (owner, expiry): (Option<String>, Option<i64>) = db
        .conn()
        .query_row(
            "SELECT host_owner_id, host_lease_expires_at_ms FROM delegated_runs WHERE delegated_run_id = 'legacy-background'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read migrated host lease");
    assert!(owner.is_none());
    assert!(
        expiry.is_none(),
        "migration must not steal mixed-version work"
    );
    let index_exists: i64 = db
        .conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_delegated_runs_expired_host_lease')",
            [],
            |row| row.get(0),
        )
        .expect("read host lease index");
    assert_eq!(index_exists, 1);
    db.run_migrations().expect("migration 54 is idempotent");
    assert_eq!(db.get_schema_version(), 55);
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
