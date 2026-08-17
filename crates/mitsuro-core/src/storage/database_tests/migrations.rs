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
fn test_session_list_metadata_migration() {
    let (db, _temp) = create_test_db();
    let columns: Vec<String> = db
        .conn()
        .prepare("PRAGMA table_info(sessions)")
        .expect("prepare sessions columns")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query sessions columns")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect sessions columns");

    assert!(columns.contains(&"pinned_at".to_string()));
    assert!(columns.contains(&"archived_at".to_string()));
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
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
    assert_eq!(db.get_schema_version(), 67);
}

#[test]
fn migration_63_backfills_workers_from_crew_and_companion_and_renames_executor() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-62-workers.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");

    // Seed a schema-62-shaped world: crew profiles for two owners, the
    // durable Hive companion session per owner, a controller on the local
    // companion, and crew_slug assignments on runtime state and schedules.
    db.conn()
        .execute_batch(
            r#"
            INSERT INTO users (id, email) VALUES ('alice', 'alice@example.com');

            INSERT INTO sessions (id, title, created_at, updated_at, session_type, user_id)
            VALUES
                ('companion-local', 'Hive', '2026-08-01T00:00:00.000000Z',
                 '2026-08-01T00:00:00.000000Z', 'hive', NULL),
                ('companion-alice', 'Hive', '2026-08-01T00:00:00.000000Z',
                 '2026-08-01T00:00:01.000000Z', 'hive', 'alice'),
                ('work-1', 'Crew job', '2026-08-01T00:00:00.000000Z',
                 '2026-08-01T00:00:00.000000Z', 'hive', NULL);

            INSERT INTO hive_profiles (id, user_id, revision)
            VALUES ('local', NULL, 0), ('user:alice-hash', 'alice', 0);
            INSERT INTO hive_crew_profiles (profile_id, slug, revision)
            VALUES ('local', 'builder', 0), ('local', 'researcher', 0),
                   ('user:alice-hash', 'builder', 0);
            INSERT INTO hive_crew_documents (profile_id, slug, kind, content, updated_at)
            VALUES
                ('local', 'builder', 'identity', 'Local builder identity',
                 '2026-08-01T00:00:00.000000Z'),
                ('local', 'builder', 'soul', 'Local builder soul',
                 '2026-08-01T00:00:00.000000Z'),
                ('user:alice-hash', 'builder', 'identity', 'Alice builder identity',
                 '2026-08-01T00:00:00.000000Z');

            INSERT INTO hive_controllers (
                id, scope_key, user_id, session_id, status, timezone,
                max_concurrent_runs, created_at, updated_at
            ) VALUES (
                'controller-local', 'scope-local', NULL, 'companion-local',
                'active', 'UTC', 1, '2026-08-01T00:00:00.000000Z',
                '2026-08-01T00:00:00.000000Z'
            );

            INSERT INTO hive_runtime_state (session_id, status, crew_slug, priority, updated_at)
            VALUES ('work-1', 'idle', 'builder', 'normal', '2026-08-01T00:00:00.000000Z');

            INSERT INTO hive_schedules (
                id, controller_id, title, summary, objective, recurrence_kind,
                recurrence_json, timezone, gap_policy, fold_policy, status,
                crew_slug, misfire_policy, misfire_grace_secs, catch_up_limit,
                overlap_policy, max_attempts, retry_base_secs, retry_max_secs,
                retry_jitter, created_by, created_at, updated_at
            ) VALUES (
                'schedule-1', 'controller-local', 'Sweep', 'Sweep', 'Do the sweep',
                'interval', '{}', 'UTC', 'shift_forward', 'first', 'enabled',
                'researcher', 'skip', 0, 0,
                'skip', 1, 0, 0,
                'none', 'test', '2026-08-01T00:00:00.000000Z', '2026-08-01T00:00:00.000000Z'
            );

            INSERT INTO hive_runs (
                id, controller_id, session_id, kind, objective, config_json,
                status, available_at, max_attempts, created_at, updated_at
            ) VALUES (
                'run-1', 'controller-local', 'companion-local', 'dispatch', 'obj',
                '{}', 'succeeded', '2026-08-01T00:00:00.000000Z', 3,
                '2026-08-01T00:00:00.000000Z', '2026-08-01T00:00:00.000000Z'
            );
            INSERT INTO hive_run_attempts (
                id, run_id, attempt_no, executor_id, lease_token, lease_epoch,
                started_at, outcome
            ) VALUES (
                'attempt-1', 'run-1', 1, 'daemon-a', 'token-1', 1,
                '2026-08-01T00:00:00.000000Z', 'succeeded'
            );
            "#,
        )
        .expect("seed schema-62 fixture data");

    // Rewind the schema 63 surface so the migration re-applies on top of the
    // seeded rows: drop the worker tables/columns and restore the legacy
    // attempt claimant column name.
    db.conn()
        .execute_batch(
            r#"
            DROP TABLE hive_worker_documents;
            DROP INDEX idx_hive_controllers_worker;
            ALTER TABLE hive_controllers DROP COLUMN worker_id;
            DROP INDEX idx_hive_runs_worker;
            ALTER TABLE hive_runs DROP COLUMN worker_id;
            DROP INDEX idx_hive_runtime_state_worker;
            ALTER TABLE hive_runtime_state DROP COLUMN worker_id;
            DROP INDEX idx_hive_schedules_worker;
            ALTER TABLE hive_schedules DROP COLUMN worker_id;
            DROP TABLE hive_workers;
            ALTER TABLE hive_run_attempts RENAME COLUMN executor_id TO worker_id;
            DELETE FROM schema_version WHERE version >= 63;
            "#,
        )
        .expect("rewind worker identity migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 63");
    assert_eq!(db.get_schema_version(), 67);

    let count_workers = |predicate: &str| -> i64 {
        db.conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM hive_workers WHERE {predicate}"),
                [],
                |row| row.get(0),
            )
            .expect("count workers")
    };
    assert_eq!(count_workers("1 = 1"), 5, "3 crew + 2 companions");
    assert_eq!(
        count_workers(
            "slug = 'builder' AND user_id IS NULL AND memory_namespace_id = 'builder' \
         AND display_name = 'Builder' AND status = 'active' AND autonomy = 'manual'"
        ),
        1
    );
    assert_eq!(count_workers("slug = 'researcher' AND user_id IS NULL"), 1);
    assert_eq!(count_workers("slug = 'builder' AND user_id = 'alice'"), 1);
    assert_eq!(
        count_workers(
            "slug = 'assistant' AND user_id IS NULL AND dm_session_id = 'companion-local'"
        ),
        1
    );
    assert_eq!(
        count_workers(
            "slug = 'assistant' AND user_id = 'alice' AND dm_session_id = 'companion-alice'"
        ),
        1
    );

    // Crew persona documents were copied onto the backfilled workers.
    let local_builder_documents: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_worker_documents d
             JOIN hive_workers w ON w.id = d.worker_id
             WHERE w.slug = 'builder' AND w.user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("count local builder documents");
    assert_eq!(local_builder_documents, 2);
    let alice_builder_identity: String = db
        .conn()
        .query_row(
            "SELECT d.content FROM hive_worker_documents d
             JOIN hive_workers w ON w.id = d.worker_id
             WHERE w.slug = 'builder' AND w.user_id = 'alice' AND d.kind = 'identity'",
            [],
            |row| row.get(0),
        )
        .expect("read alice builder identity");
    assert_eq!(alice_builder_identity, "Alice builder identity");

    // The companion controller became the assistant worker's execution lane.
    let controller_link_matches: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_controllers c
             JOIN hive_workers w ON w.id = c.worker_id
             WHERE c.id = 'controller-local' AND w.slug = 'assistant'
               AND w.user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("read controller worker link");
    assert_eq!(controller_link_matches, 1);

    // crew_slug assignments resolved to worker ids while keeping the slug.
    let runtime_link_matches: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_runtime_state r
             JOIN hive_workers w ON w.id = r.worker_id
             WHERE r.session_id = 'work-1' AND r.crew_slug = 'builder'
               AND w.slug = 'builder' AND w.user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("read runtime state worker link");
    assert_eq!(runtime_link_matches, 1);
    let schedule_link_matches: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_schedules s
             JOIN hive_workers w ON w.id = s.worker_id
             WHERE s.id = 'schedule-1' AND s.crew_slug = 'researcher'
               AND w.slug = 'researcher' AND w.user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("read schedule worker link");
    assert_eq!(schedule_link_matches, 1);

    // The rename preserved attempt data under the new executor column.
    let executor: String = db
        .conn()
        .query_row(
            "SELECT executor_id FROM hive_run_attempts WHERE id = 'attempt-1'",
            [],
            |row| row.get(0),
        )
        .expect("read renamed executor column");
    assert_eq!(executor, "daemon-a");
    let attempt_columns: Vec<String> = db
        .conn()
        .prepare("PRAGMA table_info(hive_run_attempts)")
        .expect("prepare attempt columns")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query attempt columns")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect attempt columns");
    assert!(!attempt_columns.contains(&"worker_id".to_string()));

    // Re-running the migration is a no-op: no duplicate workers, links keep
    // their targets, and the rename guard skips cleanly.
    let assistant_id: String = db
        .conn()
        .query_row(
            "SELECT id FROM hive_workers WHERE slug = 'assistant' AND user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("read assistant id");
    db.conn()
        .execute("DELETE FROM schema_version WHERE version >= 63", [])
        .expect("rewind migration marker");
    db.run_migrations().expect("reapply migration 63");
    assert_eq!(db.get_schema_version(), 67);
    assert_eq!(count_workers("1 = 1"), 5, "idempotent backfill");
    let assistant_id_after: String = db
        .conn()
        .query_row(
            "SELECT id FROM hive_workers WHERE slug = 'assistant' AND user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("read assistant id after replay");
    assert_eq!(assistant_id, assistant_id_after);
}

#[test]
fn migration_63_fresh_database_uses_executor_id_and_worker_linkage_columns() {
    let (db, _temp) = create_test_db();

    let table_columns = |table: &str| -> Vec<String> {
        db.conn()
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare table columns")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect table columns")
    };

    let worker_columns = table_columns("hive_workers");
    for column in [
        "id",
        "user_id",
        "slug",
        "display_name",
        "avatar_color",
        "model",
        "model_key_json",
        "model_catalog_revision",
        "permission_mode",
        "autonomy",
        "heartbeat_interval_secs",
        "status",
        "dm_session_id",
        "memory_namespace_id",
    ] {
        assert!(
            worker_columns.contains(&column.to_string()),
            "hive_workers missing {column}"
        );
    }
    assert!(table_columns("hive_worker_documents").contains(&"kind".to_string()));

    let attempt_columns = table_columns("hive_run_attempts");
    assert!(attempt_columns.contains(&"executor_id".to_string()));
    assert!(!attempt_columns.contains(&"worker_id".to_string()));

    for table in [
        "hive_controllers",
        "hive_runs",
        "hive_runtime_state",
        "hive_schedules",
    ] {
        assert!(
            table_columns(table).contains(&"worker_id".to_string()),
            "{table} missing worker_id"
        );
    }
}

#[test]
fn migration_64_upgrades_schema_63_with_group_rooms_and_run_linkage() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-63-groups.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    // Rewind to a schema-63-shaped world: no group tables and no group
    // linkage on hive_runs. The partial index must go before its column.
    db.conn()
        .execute_batch(
            r#"
            DROP TABLE hive_member_cursors;
            DROP TABLE hive_group_turns;
            DROP TABLE hive_group_messages;
            DROP TABLE hive_group_members;
            DROP TABLE hive_groups;
            DROP INDEX idx_hive_runs_group_turn;
            ALTER TABLE hive_runs DROP COLUMN group_id;
            ALTER TABLE hive_runs DROP COLUMN group_turn_id;
            ALTER TABLE hive_runs DROP COLUMN trigger_message_id;
            DELETE FROM schema_version WHERE version >= 64;
            "#,
        )
        .expect("rewind group room migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 64");
    assert_eq!(db.get_schema_version(), 67);

    let table_exists = |table: &str| -> bool {
        db.conn()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .expect("read table existence")
    };
    for table in [
        "hive_groups",
        "hive_group_members",
        "hive_group_messages",
        "hive_group_turns",
        "hive_member_cursors",
    ] {
        assert!(table_exists(table), "missing table {table}");
    }
    let run_columns: Vec<String> = db
        .conn()
        .prepare("PRAGMA table_info(hive_runs)")
        .expect("prepare run columns")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query run columns")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect run columns");
    for column in ["group_id", "group_turn_id", "trigger_message_id"] {
        assert!(
            run_columns.contains(&column.to_string()),
            "hive_runs missing {column}"
        );
    }

    // The per-group sequence is unique and the mode CHECK holds.
    db.conn()
        .execute_batch(
            "INSERT INTO hive_groups (id, user_id, title, created_at, updated_at)
             VALUES ('group-1', NULL, 'Room', '2026-08-16T00:00:00Z', '2026-08-16T00:00:00Z');
             INSERT INTO hive_group_messages (
                 id, group_id, seq, sender_kind, content, created_at
             ) VALUES ('message-1', 'group-1', 1, 'user', 'hi', '2026-08-16T00:00:00Z');",
        )
        .expect("seed a group and message");
    let duplicate_seq = db.conn().execute(
        "INSERT INTO hive_group_messages (id, group_id, seq, sender_kind, content, created_at)
         VALUES ('message-2', 'group-1', 1, 'user', 'again', '2026-08-16T00:00:01Z')",
        [],
    );
    assert!(duplicate_seq.is_err(), "seq must be unique per group");
    let invalid_mode = db.conn().execute(
        "UPDATE hive_groups SET execution_mode = 'swarm' WHERE id = 'group-1'",
        [],
    );
    assert!(invalid_mode.is_err(), "execution mode CHECK must hold");

    db.run_migrations().expect("migration 64 is idempotent");
    assert_eq!(db.get_schema_version(), 67);
}

#[test]
fn migration_65_upgrades_schema_64_with_delivery_ledger() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-64-deliveries.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    db.conn()
        .execute_batch(
            r#"
            DROP TABLE hive_deliveries;
            DELETE FROM schema_version WHERE version >= 65;
            "#,
        )
        .expect("rewind delivery ledger migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 65");
    assert_eq!(db.get_schema_version(), 67);

    let exists: bool = db
        .conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'hive_deliveries')",
            [],
            |row| row.get(0),
        )
        .expect("read table existence");
    assert!(exists, "missing hive_deliveries");

    let runs_sql: String = db
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'hive_runs'",
            [],
            |row| row.get(0),
        )
        .expect("read hive_runs sql");
    assert!(
        runs_sql.contains("worker_message") || !runs_sql.contains("kind IN"),
        "hive_runs kind CHECK must accept worker_message"
    );

    db.run_migrations().expect("migration 65 is idempotent");
    assert_eq!(db.get_schema_version(), 67);
}

#[test]
fn migration_66_upgrades_schema_65_with_memory_acl_scopes() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-65-memory-acl.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    db.conn()
        .execute_batch(
            r#"
            INSERT INTO agent_memories (
                id, memory_type, title, content, namespace, namespace_id,
                status, source, confidence, sensitivity, pinned, access_count
            ) VALUES (
                'crew-1', 'project', 'Private', 'crew-private-marker',
                'crew', 'researcher', 'active', 'agent', 1.0, 'normal', 0, 0
            );
            INSERT INTO agent_memories (
                id, memory_type, title, content, namespace,
                status, source, confidence, sensitivity, pinned, access_count
            ) VALUES (
                'shared-1', 'project', 'Shared', 'shared-memory-marker',
                'shared', 'active', 'agent', 1.0, 'normal', 0, 0
            );
            DROP INDEX IF EXISTS idx_agent_memories_acl;
            ALTER TABLE agent_memories DROP COLUMN acl_scope;
            ALTER TABLE agent_memories DROP COLUMN conversation_id;
            DELETE FROM schema_version WHERE version >= 66;
            "#,
        )
        .expect("rewind memory ACL migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 66");
    assert_eq!(db.get_schema_version(), 67);

    let columns: Vec<String> = db
        .conn()
        .prepare("PRAGMA table_info(agent_memories)")
        .expect("prepare memory columns")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query memory columns")
        .filter_map(Result::ok)
        .collect();
    assert!(columns.contains(&"acl_scope".to_string()));
    assert!(columns.contains(&"conversation_id".to_string()));

    let crew_scope: String = db
        .conn()
        .query_row(
            "SELECT acl_scope FROM agent_memories WHERE id = 'crew-1'",
            [],
            |row| row.get(0),
        )
        .expect("read crew ACL");
    assert_eq!(crew_scope, "worker");
    let shared_scope: String = db
        .conn()
        .query_row(
            "SELECT acl_scope FROM agent_memories WHERE id = 'shared-1'",
            [],
            |row| row.get(0),
        )
        .expect("read shared ACL");
    assert_eq!(shared_scope, "owner");

    db.run_migrations().expect("migration 66 is idempotent");
    assert_eq!(db.get_schema_version(), 67);
}

#[test]
fn migration_67_upgrades_schema_66_with_worker_heartbeat_kind() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("schema-66-heartbeat.db");
    let db = crate::storage::database::Database::new(&path).expect("create current database");
    const DELIVERY_KINDS: &str = "('dispatch', 'scheduled', 'controller_child', 'legacy_resume', 'group_turn', 'worker_message')";
    const HEARTBEAT_KINDS: &str = "('dispatch', 'scheduled', 'controller_child', 'legacy_resume', 'group_turn', 'worker_message', 'worker_heartbeat')";
    db.conn()
        .pragma_update(None, "writable_schema", "ON")
        .expect("enable writable_schema");
    db.conn()
        .execute(
            "UPDATE sqlite_master SET sql = replace(sql, ?1, ?2)
             WHERE type = 'table' AND name = 'hive_runs'
               AND instr(sql, ?1) > 0",
            [HEARTBEAT_KINDS, DELIVERY_KINDS],
        )
        .expect("rewind hive_runs kind CHECK");
    db.conn()
        .pragma_update(None, "writable_schema", "RESET")
        .expect("reload schema after CHECK rewind");
    db.conn()
        .execute("DELETE FROM schema_version WHERE version >= 67", [])
        .expect("rewind heartbeat kind migration");
    drop(db);

    let db = crate::storage::database::Database::new(&path).expect("apply migration 67");
    assert_eq!(db.get_schema_version(), 67);

    let runs_sql: String = db
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'hive_runs'",
            [],
            |row| row.get(0),
        )
        .expect("read hive_runs sql");
    assert!(
        runs_sql.contains("worker_heartbeat") || !runs_sql.contains("kind IN"),
        "hive_runs kind CHECK must accept worker_heartbeat"
    );

    db.run_migrations().expect("migration 67 is idempotent");
    assert_eq!(db.get_schema_version(), 67);
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
