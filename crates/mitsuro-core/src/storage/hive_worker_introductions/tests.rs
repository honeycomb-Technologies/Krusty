use tempfile::TempDir;

use crate::ai::models::{ApiFormat, ModelKey};
use crate::ai::providers::ProviderId;
use crate::storage::{Database, HiveWorkerStore, NewHiveWorker};

use super::{
    save_worker_introduction_opening_once, HiveWorkerIntroductionStatus,
    HiveWorkerIntroductionStore,
};

struct Fixture {
    db: Database,
    worker_id: String,
    run_id: String,
    session_id: String,
    _temp: TempDir,
}

fn fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("introductions.db");
    let mut worker_input = NewHiveWorker::new("curious-worker");
    worker_input.dm_session_id = Some("worker-dm".into());
    worker_input.model = Some("test-model".into());
    let model_key = ModelKey::new(ProviderId::OpenAI, "test-model", ApiFormat::OpenAIResponses);
    worker_input.model_key = Some(model_key.clone());
    worker_input.model_catalog_revision = Some("catalog-test".into());
    let db = Database::new(&path).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                 id, title, created_at, updated_at, session_type
             ) VALUES ('worker-dm', 'Worker DM', ?1, ?1, 'hive')",
            [&now],
        )
        .unwrap();
    let worker = HiveWorkerStore::new(Database::new(&path).unwrap())
        .create(&worker_input)
        .unwrap();
    let model_key_json = serde_json::to_string(&model_key).unwrap();
    db.conn()
        .execute(
            "UPDATE sessions
             SET model = 'test-model', model_key_json = ?2,
                 model_catalog_revision = 'catalog-test'
             WHERE id = ?1",
            rusqlite::params!["worker-dm", model_key_json],
        )
        .unwrap();
    let run_id = "introduction-run".to_string();
    db.conn()
        .execute_batch(&format!(
            r#"
            INSERT INTO hive_controllers (
                id, scope_key, session_id, status, timezone,
                max_concurrent_runs, created_at, updated_at, worker_id
            ) VALUES (
                'introduction-controller', 'local:introduction', 'worker-dm',
                'active', 'UTC', 1, '{now}', '{now}', '{}'
            );
            INSERT INTO hive_runs (
                id, controller_id, session_id, kind, objective, config_json,
                status, available_at, max_attempts, created_at, updated_at,
                worker_id, governor_origin, governor_lane_key,
                execution_context_json
            ) VALUES (
                '{run_id}', 'introduction-controller', 'worker-dm',
                'worker_introduction', 'meet the user', '{{}}', 'queued',
                '{now}', 2, '{now}', '{now}', '{}',
                'user_lifecycle_action', 'dm',
                '{{"schema_version":1,"mode":{{"kind":"worker_conversation_neutral","worker_id":"{}","worker_revision":1,"lane":{{"kind":"direct_message"}}}}}}'
            );
            INSERT INTO hive_worker_introductions (
                worker_id, run_id, status, prompt_version, created_at, updated_at
            ) VALUES ('{}', '{run_id}', 'queued', 1, '{now}', '{now}');
            "#,
            worker.id, worker.id, worker.id, worker.id
        ))
        .unwrap();
    db.conn()
        .execute(
            "UPDATE hive_runs SET config_json = ?2 WHERE id = ?1",
            rusqlite::params![
                run_id,
                serde_json::json!({
                    "worker_id": worker.id,
                    "model": "test-model",
                    "model_key": model_key,
                    "model_catalog_revision": "catalog-test",
                    "permission_mode": "autonomous",
                })
                .to_string()
            ],
        )
        .unwrap();
    Fixture {
        db,
        worker_id: worker.id,
        run_id,
        session_id: "worker-dm".into(),
        _temp: temp,
    }
}

#[test]
fn reads_by_worker_and_run_and_advances_to_opened_idempotently() {
    let fixture = fixture();
    let store = HiveWorkerIntroductionStore::new(&fixture.db);
    let queued = store.get_by_worker(&fixture.worker_id).unwrap().unwrap();
    assert_eq!(queued.status, HiveWorkerIntroductionStatus::Queued);
    assert_eq!(store.get_by_run(&fixture.run_id).unwrap(), Some(queued));

    let running = store
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .unwrap();
    assert_eq!(running.status, HiveWorkerIntroductionStatus::Running);
    let now = chrono::Utc::now().to_rfc3339();
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, 'assistant', '[]', ?2)",
            rusqlite::params![fixture.session_id, now],
        )
        .unwrap();
    let opening_message_id = fixture.db.conn().last_insert_rowid();
    let opened = store
        .mark_opened(&fixture.worker_id, &fixture.run_id, opening_message_id)
        .unwrap();
    assert_eq!(opened.status, HiveWorkerIntroductionStatus::AwaitingContext);
    assert_eq!(opened.opening_message_id, Some(opening_message_id));
    assert_eq!(
        store
            .mark_opened(&fixture.worker_id, &fixture.run_id, opening_message_id)
            .unwrap()
            .opening_message_id,
        Some(opening_message_id)
    );
}

#[test]
fn stale_runs_and_non_dm_openings_fail_closed() {
    let fixture = fixture();
    let store = HiveWorkerIntroductionStore::new(&fixture.db);
    assert!(store
        .mark_running(&fixture.worker_id, "stale-run")
        .unwrap_err()
        .to_string()
        .contains("current state: queued"));
    store
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES ('foreign-session', 'Foreign', ?1, ?1)",
            [&now],
        )
        .unwrap();
    fixture
        .db
        .conn()
        .execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES ('foreign-session', 'assistant', '[]', ?1)",
            [&now],
        )
        .unwrap();
    let foreign_message_id = fixture.db.conn().last_insert_rowid();
    assert!(store
        .mark_opened(&fixture.worker_id, &fixture.run_id, foreign_message_id)
        .unwrap_err()
        .to_string()
        .contains("current state: running"));
}

#[test]
fn skip_exposes_explicit_autonomy_semantics() {
    let fixture = fixture();
    let skipped = HiveWorkerIntroductionStore::new(&fixture.db)
        .skip(&fixture.worker_id)
        .unwrap();
    assert_eq!(skipped.status, HiveWorkerIntroductionStatus::Skipped);
    assert!(skipped.status.allows_autonomy());
}

#[test]
fn recovery_can_resume_but_terminal_failure_cannot() {
    let fixture = fixture();
    let store = HiveWorkerIntroductionStore::new(&fixture.db);
    store
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .unwrap();
    let recovery = store
        .mark_needs_recovery(
            &fixture.worker_id,
            &fixture.run_id,
            "ambiguous provider result",
        )
        .unwrap();
    assert_eq!(recovery.status, HiveWorkerIntroductionStatus::NeedsRecovery);
    assert!(recovery.completed_at.is_none());
    assert_eq!(
        store
            .mark_running(&fixture.worker_id, &fixture.run_id)
            .unwrap()
            .status,
        HiveWorkerIntroductionStatus::Running
    );
    let failed = store
        .mark_failed(
            &fixture.worker_id,
            &fixture.run_id,
            "provider rejected request",
        )
        .unwrap();
    assert_eq!(failed.status, HiveWorkerIntroductionStatus::Failed);
    assert!(failed.completed_at.is_some());
    assert!(store
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .is_err());
}

#[test]
fn opening_commit_fences_lifecycle_and_is_the_first_message() {
    let fixture = fixture();
    fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_runs SET status = 'running' WHERE id = ?1",
            [&fixture.run_id],
        )
        .unwrap();
    HiveWorkerIntroductionStore::new(&fixture.db)
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .unwrap();
    let content = r#"[{"type":"text","text":"What should we build together?"}]"#;
    let message_id = save_worker_introduction_opening_once(
        &fixture.db,
        &fixture.worker_id,
        &fixture.run_id,
        &fixture.session_id,
        content,
        "introduction:introduction-run:opening",
    )
    .unwrap();

    let introduction = HiveWorkerIntroductionStore::new(&fixture.db)
        .get_by_worker(&fixture.worker_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        introduction.status,
        HiveWorkerIntroductionStatus::AwaitingContext
    );
    assert_eq!(introduction.opening_message_id, Some(message_id));
    let messages = crate::storage::MessageStore::new(&fixture.db)
        .load_session_message_records(&fixture.session_id)
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message_id);
    assert_eq!(messages[0].role, "assistant");
}

#[test]
fn skipped_lifecycle_rejects_a_late_opening() {
    let fixture = fixture();
    fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_runs SET status = 'running' WHERE id = ?1",
            [&fixture.run_id],
        )
        .unwrap();
    let store = HiveWorkerIntroductionStore::new(&fixture.db);
    store
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .unwrap();
    store.skip(&fixture.worker_id).unwrap();

    let error = save_worker_introduction_opening_once(
        &fixture.db,
        &fixture.worker_id,
        &fixture.run_id,
        &fixture.session_id,
        r#"[{"type":"text","text":"Too late?"}]"#,
        "introduction:introduction-run:opening",
    )
    .expect_err("skip must fence the opening");
    assert!(error.to_string().contains("no longer running"), "{error:#}");
    assert_eq!(
        crate::storage::MessageStore::new(&fixture.db)
            .get_message_count(&fixture.session_id)
            .unwrap(),
        0
    );
}

#[test]
fn paused_or_archived_worker_fences_a_late_opening_atomically() {
    for status in ["paused", "archived"] {
        let fixture = fixture();
        fixture
            .db
            .conn()
            .execute(
                "UPDATE hive_runs SET status = 'running' WHERE id = ?1",
                [&fixture.run_id],
            )
            .unwrap();
        HiveWorkerIntroductionStore::new(&fixture.db)
            .mark_running(&fixture.worker_id, &fixture.run_id)
            .unwrap();
        fixture
            .db
            .conn()
            .execute(
                "UPDATE hive_workers SET status = ?2 WHERE id = ?1",
                rusqlite::params![fixture.worker_id, status],
            )
            .unwrap();

        let error = save_worker_introduction_opening_once(
            &fixture.db,
            &fixture.worker_id,
            &fixture.run_id,
            &fixture.session_id,
            r#"[{"type":"text","text":"Too late?"}]"#,
            "introduction:introduction-run:opening",
        )
        .expect_err("inactive Worker must fence the opening");
        assert!(error.to_string().contains("active Worker"), "{error:#}");
        assert_eq!(
            crate::storage::MessageStore::new(&fixture.db)
                .get_message_count(&fixture.session_id)
                .unwrap(),
            0
        );
    }
}

#[test]
fn model_drift_fences_a_late_opening_atomically() {
    let fixture = fixture();
    fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_runs SET status = 'running' WHERE id = ?1",
            [&fixture.run_id],
        )
        .unwrap();
    HiveWorkerIntroductionStore::new(&fixture.db)
        .mark_running(&fixture.worker_id, &fixture.run_id)
        .unwrap();
    let replacement_key = ModelKey::new(
        ProviderId::OpenAI,
        "replacement-model",
        ApiFormat::OpenAIResponses,
    );
    fixture
        .db
        .conn()
        .execute(
            "UPDATE hive_workers
             SET model = 'replacement-model', model_key_json = ?2
             WHERE id = ?1",
            rusqlite::params![
                fixture.worker_id,
                serde_json::to_string(&replacement_key).unwrap()
            ],
        )
        .unwrap();

    save_worker_introduction_opening_once(
        &fixture.db,
        &fixture.worker_id,
        &fixture.run_id,
        &fixture.session_id,
        r#"[{"type":"text","text":"Too late?"}]"#,
        "introduction:introduction-run:opening",
    )
    .expect_err("model drift must fence the opening");
    assert_eq!(
        crate::storage::MessageStore::new(&fixture.db)
            .get_message_count(&fixture.session_id)
            .unwrap(),
        0
    );
}
