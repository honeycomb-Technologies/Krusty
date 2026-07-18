use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use tempfile::TempDir;

use crate::mako::MakoRunStatus;
use crate::storage::Database;

use super::{
    ClaimRunRequest, DaemonFence, MakoRun, MakoRunAttemptOutcome, MakoRunKind, MakoRunStore,
    RunCompletion,
};
use crate::storage::{DaemonLeaseAcquire, MakoDaemonLeaseStore};

fn instant(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, second)
        .single()
        .unwrap()
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn store() -> (MakoRunStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("runs.db")).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('session-1', 'Mako controller', '2026-07-01T00:00:00.000000Z',
                     '2026-07-01T00:00:00.000000Z', 'mako');
             INSERT INTO mako_controllers (
                 id, scope_key, user_id, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES (
                 'controller-1', 'local:test', NULL, 'session-1', 'active', 'UTC', 1,
                 '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z'
             );",
        )
        .unwrap();
    (MakoRunStore::new(db), temp)
}

fn run(id: &str, priority: i32, max_attempts: u32) -> MakoRun {
    let now = instant(0);
    MakoRun {
        id: id.into(),
        controller_id: "controller-1".into(),
        session_id: None,
        schedule_id: None,
        occurrence_id: None,
        kind: MakoRunKind::Dispatch,
        objective: format!("execute {id}"),
        config: serde_json::json!({"permission_mode": "ask"}),
        status: MakoRunStatus::Queued,
        priority,
        concurrency_key: None,
        scheduled_for: None,
        available_at: timestamp(now),
        wake_at: None,
        attempt_count: 0,
        max_attempts,
        lease_owner: None,
        lease_token: None,
        lease_epoch: None,
        lease_expires_at: None,
        heartbeat_at: None,
        last_stop_reason: None,
        last_error: None,
        outcome: None,
        created_at: timestamp(now),
        started_at: None,
        finished_at: None,
        updated_at: timestamp(now),
    }
}

fn claim_request(now: DateTime<Utc>, epoch: u64) -> ClaimRunRequest {
    ClaimRunRequest {
        worker_id: "worker-1".into(),
        lease_epoch: epoch,
        now,
        lease_duration: Duration::from_secs(10),
        global_concurrency_limit: 8,
    }
}

fn completion(target_status: MakoRunStatus, now: DateTime<Utc>) -> RunCompletion {
    RunCompletion {
        target_status,
        now,
        available_at: None,
        wake_at: None,
        stop_reason: Some("completed".into()),
        error: None,
        outcome: Some(serde_json::json!({"ok": true})),
        trace_sequence_end: Some(42),
    }
}

#[test]
fn claim_is_priority_ordered_and_honors_controller_concurrency() {
    let (store, _temp) = store();
    store.insert_run(&run("low", 1, 3)).unwrap();
    store.insert_run(&run("high", 50, 3)).unwrap();

    let claimed = store
        .claim_next(&claim_request(instant(0), 7))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.run.id, "high");
    assert_eq!(claimed.attempt_no, 1);
    assert!(store
        .claim_next(&claim_request(instant(1), 7))
        .unwrap()
        .is_none());

    assert!(store
        .mark_running("high", &claimed.lease_token, 7, instant(1))
        .unwrap());
    assert_eq!(
        store
            .finish_claimed(
                "high",
                &claimed.lease_token,
                7,
                &completion(MakoRunStatus::Succeeded, instant(2)),
            )
            .unwrap(),
        Some(MakoRunStatus::Succeeded)
    );
    assert_eq!(
        store
            .claim_next(&claim_request(instant(3), 7))
            .unwrap()
            .unwrap()
            .run
            .id,
        "low"
    );
}

#[test]
fn first_transition_is_journaled_for_an_empty_controller_event_stream() {
    let (store, temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();

    store
        .claim_next(&claim_request(instant(0), 7))
        .unwrap()
        .expect("run should be claimed");

    let db = Database::new(&temp.path().join("runs.db")).unwrap();
    let event: (i64, String, String, String) = db
        .conn()
        .query_row(
            "SELECT sequence, event_type, dedupe_key, payload_json
             FROM mako_controller_events WHERE controller_id = 'controller-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("the first transition must create the first controller event");
    assert_eq!(event.0, 1);
    assert_eq!(event.1, "run_leased");
    assert_eq!(event.2, "transition:run-1:1:leased");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&event.3).unwrap()["status"],
        "leased"
    );
}

#[test]
fn transition_rolls_back_if_its_authoritative_event_cannot_be_written() {
    let (store, temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let db = Database::new(&temp.path().join("runs.db")).unwrap();
    db.conn()
        .execute_batch(
            "CREATE TRIGGER reject_mako_event
             BEFORE INSERT ON mako_controller_events
             BEGIN
                 SELECT RAISE(ABORT, 'simulated journal failure');
             END;",
        )
        .unwrap();
    drop(db);

    let error = store
        .claim_next(&claim_request(instant(0), 7))
        .expect_err("claim must fail when its transition event cannot commit");
    assert!(error.to_string().contains("simulated journal failure"));
    let persisted = store.get_run("run-1").unwrap().unwrap();
    assert_eq!(persisted.status, MakoRunStatus::Queued);
    assert_eq!(persisted.attempt_count, 0);
    assert!(store.list_attempts("run-1").unwrap().is_empty());
}

#[test]
fn lease_token_and_epoch_fence_heartbeats_and_completion() {
    let (store, _temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let claimed = store
        .claim_next(&claim_request(instant(0), 11))
        .unwrap()
        .unwrap();

    assert!(!store
        .heartbeat(
            "run-1",
            "stale-token",
            11,
            instant(1),
            Duration::from_secs(10)
        )
        .unwrap());
    assert!(!store
        .mark_running("run-1", &claimed.lease_token, 10, instant(1))
        .unwrap());
    assert!(store
        .mark_running_with_trace("run-1", &claimed.lease_token, 11, instant(1), Some(8))
        .unwrap());
    assert!(store
        .heartbeat(
            "run-1",
            &claimed.lease_token,
            11,
            instant(2),
            Duration::from_secs(10),
        )
        .unwrap());
    assert_eq!(
        store
            .finish_claimed(
                "run-1",
                &claimed.lease_token,
                10,
                &completion(MakoRunStatus::Succeeded, instant(3)),
            )
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .finish_claimed(
                "run-1",
                &claimed.lease_token,
                11,
                &completion(MakoRunStatus::Succeeded, instant(3)),
            )
            .unwrap(),
        Some(MakoRunStatus::Succeeded)
    );

    let attempts = store.list_attempts("run-1").unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, MakoRunAttemptOutcome::Succeeded);
    assert_eq!(attempts[0].trace_sequence_start, Some(8));
    assert_eq!(attempts[0].trace_sequence_end, Some(42));
}

#[test]
fn disabled_controller_fences_mark_running_before_backend_side_effects() {
    let (store, temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let claimed = store
        .claim_next(&claim_request(instant(0), 11))
        .unwrap()
        .unwrap();
    Database::new(&temp.path().join("runs.db"))
        .unwrap()
        .conn()
        .execute(
            "UPDATE mako_controllers SET status = 'disabled' WHERE id = 'controller-1'",
            [],
        )
        .unwrap();

    assert!(!store
        .mark_running("run-1", &claimed.lease_token, 11, instant(1))
        .unwrap());
    assert!(!store
        .heartbeat(
            "run-1",
            &claimed.lease_token,
            11,
            instant(1),
            Duration::from_secs(10),
        )
        .unwrap());
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        MakoRunStatus::Leased
    );
}

#[test]
fn expired_unstarted_lease_is_requeued() {
    let (store, _temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();

    store
        .claim_next(&claim_request(instant(0), 1))
        .unwrap()
        .unwrap();
    let reconciled = store.reconcile_expired_leases(instant(11)).unwrap();
    assert_eq!(reconciled.requeued_unstarted, 1);
    assert_eq!(reconciled.recovery_required, 0);
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        MakoRunStatus::Queued
    );
    assert_eq!(
        store.list_attempts("run-1").unwrap()[0].outcome,
        MakoRunAttemptOutcome::Abandoned
    );
}

#[test]
fn expired_running_delivery_requires_recovery() {
    let (store, _temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let claimed = store
        .claim_next(&claim_request(instant(0), 1))
        .unwrap()
        .unwrap();
    assert!(store
        .mark_running("run-1", &claimed.lease_token, 1, instant(1))
        .unwrap());
    store.insert_run(&run("queued-sibling", 0, 3)).unwrap();

    let reconciled = store.reconcile_expired_leases(instant(11)).unwrap();
    assert_eq!(reconciled.requeued_unstarted, 0);
    assert_eq!(reconciled.recovery_required, 1);
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        MakoRunStatus::RecoveryRequired
    );
    assert!(store
        .claim_next(&claim_request(instant(12), 2))
        .unwrap()
        .is_none());
}

#[test]
fn second_daemon_takeover_rejects_stale_completion() {
    let (store, temp) = store();
    let lease_store = MakoDaemonLeaseStore::new(
        Database::new(&temp.path().join("runs.db")).expect("daemon lease database"),
    );
    let first = match lease_store
        .acquire(
            "mako-scheduler",
            "daemon-a",
            instant(0),
            Duration::from_secs(10),
        )
        .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        held => panic!("unexpected lease result: {held:?}"),
    };
    let first_fence = DaemonFence {
        lease_name: first.lease_name,
        owner_id: first.owner_id,
        fencing_token: first.fencing_token,
    };
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let mut request = claim_request(instant(0), first_fence.fencing_token);
    request.lease_duration = Duration::from_secs(100);
    let claimed = store
        .claim_next_fenced(&request, &first_fence)
        .unwrap()
        .unwrap();
    assert!(store
        .mark_running_fenced(
            "run-1",
            &claimed.lease_token,
            first_fence.fencing_token,
            instant(1),
            &first_fence,
        )
        .unwrap());

    let second = match lease_store
        .acquire(
            "mako-scheduler",
            "daemon-b",
            instant(11),
            Duration::from_secs(10),
        )
        .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        held => panic!("unexpected lease result: {held:?}"),
    };
    assert!(second.fencing_token > first_fence.fencing_token);

    assert_eq!(
        store
            .finish_claimed_fenced(
                "run-1",
                &claimed.lease_token,
                first_fence.fencing_token,
                &completion(MakoRunStatus::Succeeded, instant(12)),
                &first_fence,
            )
            .unwrap(),
        None
    );
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        MakoRunStatus::Running
    );
    let journal_db = Database::new(&temp.path().join("runs.db")).unwrap();
    let journal = journal_db
        .conn()
        .prepare(
            "SELECT event_type FROM mako_controller_events
             WHERE run_id = 'run-1' ORDER BY sequence",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(journal, vec!["run_leased", "run_started"]);
}

#[test]
fn execution_host_revalidates_exact_claim_and_immutable_inputs() {
    let (store, temp) = store();
    let lease_store = MakoDaemonLeaseStore::new(
        Database::new(&temp.path().join("runs.db")).expect("daemon lease database"),
    );
    let lease = match lease_store
        .acquire(
            "mako-scheduler",
            "worker-1",
            instant(0),
            Duration::from_secs(10),
        )
        .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        held => panic!("unexpected lease result: {held:?}"),
    };
    let fence = DaemonFence {
        lease_name: lease.lease_name,
        owner_id: lease.owner_id,
        fencing_token: lease.fencing_token,
    };
    let mut scheduled = run("fenced", 0, 3);
    scheduled.session_id = Some("session-1".into());
    scheduled.kind = MakoRunKind::Scheduled;
    scheduled.objective = "Inspect the immutable deployment target".into();
    scheduled.config = serde_json::json!({
        "working_dir": "/work/original",
        "project_dir": "/work/original/project",
        "model": "provider:claimed-model",
        "crew_slug": "release",
    });
    store.insert_run(&scheduled).unwrap();
    let mut request = claim_request(instant(0), fence.fencing_token);
    request.lease_duration = Duration::from_secs(20);
    let claim = store.claim_next_fenced(&request, &fence).unwrap().unwrap();
    assert!(store
        .mark_running_fenced(
            &claim.run.id,
            &claim.lease_token,
            fence.fencing_token,
            instant(1),
            &fence,
        )
        .unwrap());
    assert!(store
        .validate_claimed_execution_fenced(&claim, &fence, instant(2))
        .unwrap());

    let db = Database::new(&temp.path().join("runs.db")).unwrap();
    db.conn()
        .execute(
            "UPDATE mako_runs SET config_json = json_set(config_json, '$.model', 'provider:mutated')
             WHERE id = ?1",
            [&claim.run.id],
        )
        .unwrap();
    assert!(!store
        .validate_claimed_execution_fenced(&claim, &fence, instant(2))
        .unwrap());
}

#[test]
fn exhausted_retry_budget_dead_letters_the_run() {
    let (store, _temp) = store();
    store.insert_run(&run("run-1", 0, 1)).unwrap();
    let claimed = store
        .claim_next(&claim_request(instant(0), 1))
        .unwrap()
        .unwrap();
    store
        .mark_running("run-1", &claimed.lease_token, 1, instant(1))
        .unwrap();
    let mut retry = completion(MakoRunStatus::RetryWait, instant(2));
    retry.available_at = Some(instant(20));

    assert_eq!(
        store
            .finish_claimed("run-1", &claimed.lease_token, 1, &retry)
            .unwrap(),
        Some(MakoRunStatus::DeadLetter)
    );
    let attempts = store.list_attempts("run-1").unwrap();
    assert_eq!(attempts[0].outcome, MakoRunAttemptOutcome::DeadLetter);
    assert_eq!(attempts[0].retry_at, None);
}

#[test]
fn cancellation_fences_an_active_worker_and_closes_its_attempt() {
    let (store, _temp) = store();
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let claimed = store
        .claim_next(&claim_request(instant(0), 4))
        .unwrap()
        .unwrap();
    store
        .mark_running("run-1", &claimed.lease_token, 4, instant(1))
        .unwrap();

    assert!(store
        .cancel("run-1", instant(2), "cancelled by user")
        .unwrap());
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        MakoRunStatus::Cancelled
    );
    assert_eq!(
        store.list_attempts("run-1").unwrap()[0].outcome,
        MakoRunAttemptOutcome::Cancelled
    );
    assert_eq!(
        store
            .finish_claimed(
                "run-1",
                &claimed.lease_token,
                4,
                &completion(MakoRunStatus::Succeeded, instant(3)),
            )
            .unwrap(),
        None
    );
}

#[test]
fn committed_cancellation_requires_exact_live_claim_and_daemon_fence() {
    let (store, temp) = store();
    let lease = match MakoDaemonLeaseStore::new(
        Database::new(&temp.path().join("runs.db")).expect("daemon lease database"),
    )
    .acquire(
        "mako-scheduler",
        "worker-1",
        instant(0),
        Duration::from_secs(10),
    )
    .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        held => panic!("unexpected lease result: {held:?}"),
    };
    let fence = DaemonFence {
        lease_name: lease.lease_name,
        owner_id: lease.owner_id,
        fencing_token: lease.fencing_token,
    };
    store.insert_run(&run("run-1", 0, 3)).unwrap();
    let claimed = store
        .claim_next_fenced(&claim_request(instant(0), fence.fencing_token), &fence)
        .unwrap()
        .unwrap();
    assert!(store
        .mark_running_fenced(
            "run-1",
            &claimed.lease_token,
            fence.fencing_token,
            instant(1),
            &fence,
        )
        .unwrap());
    let mut cancelled = completion(MakoRunStatus::Cancelled, instant(2));
    cancelled.stop_reason = Some("cancellation grace elapsed".into());
    cancelled.error = Some("side effects may be uncertain".into());
    cancelled.outcome = Some(serde_json::json!({
        "kind": "cancelled",
        "forced": true,
    }));

    assert_eq!(
        store
            .finish_cancelled_claim_fenced(
                "run-1",
                &claimed.lease_token,
                fence.fencing_token,
                &cancelled,
                &fence,
            )
            .unwrap(),
        None,
        "an active controller is not a committed CancelSession"
    );
    Database::new(&temp.path().join("runs.db"))
        .unwrap()
        .conn()
        .execute(
            "UPDATE mako_controllers SET status = 'disabled' WHERE id = 'controller-1'",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .finish_cancelled_claim_fenced(
                "run-1",
                "wrong-token",
                fence.fencing_token,
                &cancelled,
                &fence,
            )
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .finish_cancelled_claim_fenced(
                "run-1",
                &claimed.lease_token,
                fence.fencing_token,
                &cancelled,
                &fence,
            )
            .unwrap(),
        Some(MakoRunStatus::Cancelled)
    );
    assert_eq!(
        store.list_attempts("run-1").unwrap()[0].outcome,
        MakoRunAttemptOutcome::Cancelled
    );
    assert_eq!(
        store
            .finish_claimed_fenced(
                "run-1",
                &claimed.lease_token,
                fence.fencing_token,
                &completion(MakoRunStatus::Succeeded, instant(3)),
                &fence,
            )
            .unwrap(),
        None,
        "late worker completion must not overwrite authoritative cancellation"
    );
}

#[test]
fn due_run_projection_does_not_clobber_an_active_sibling() {
    let (store, temp) = store();
    let mut active = run("active", 50, 3);
    active.session_id = Some("session-1".into());
    store.insert_run(&active).unwrap();

    let claimed = store
        .claim_next(&claim_request(instant(0), 1))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.run.id, "active");
    assert!(store
        .mark_running("active", &claimed.lease_token, 1, instant(1))
        .unwrap());

    let mut sleeping = run("sleeping", 0, 3);
    sleeping.session_id = Some("session-1".into());
    store.insert_run(&sleeping).unwrap();
    let projection_db = Database::new(&temp.path().join("runs.db")).unwrap();
    projection_db
        .conn()
        .execute(
            "UPDATE mako_runs SET status = 'sleeping', wake_at = ?2 WHERE id = ?1",
            rusqlite::params![sleeping.id, timestamp(instant(2))],
        )
        .unwrap();
    drop(projection_db);
    assert_eq!(store.promote_due_runs(instant(2)).unwrap(), 1);

    let db = Database::new(&temp.path().join("runs.db")).unwrap();
    let (runtime_status, current_run_id): (String, String) = db
        .conn()
        .query_row(
            "SELECT status, current_run_id FROM mako_runtime_state
             WHERE session_id = 'session-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(runtime_status, "running");
    assert_eq!(current_run_id, "active");
}

#[test]
fn scheduled_objective_is_materialized_exactly_once_across_retry() {
    let (store, temp) = store();
    let mut scheduled = run("scheduled", 0, 3);
    scheduled.session_id = Some("session-1".into());
    scheduled.kind = MakoRunKind::Scheduled;
    scheduled.objective = "Inspect the deployment health".into();
    store.insert_run(&scheduled).unwrap();

    let first = store
        .claim_next(&claim_request(instant(0), 1))
        .unwrap()
        .unwrap();
    assert!(store
        .mark_running("scheduled", &first.lease_token, 1, instant(1))
        .unwrap());
    let mut retry = completion(MakoRunStatus::RetryWait, instant(2));
    retry.available_at = Some(instant(3));
    assert_eq!(
        store
            .finish_claimed("scheduled", &first.lease_token, 1, &retry)
            .unwrap(),
        Some(MakoRunStatus::RetryWait)
    );
    assert_eq!(store.promote_due_runs(instant(3)).unwrap(), 1);
    let second = store
        .claim_next(&claim_request(instant(3), 1))
        .unwrap()
        .unwrap();
    assert!(store
        .mark_running("scheduled", &second.lease_token, 1, instant(4))
        .unwrap());

    let db = Database::new(&temp.path().join("runs.db")).unwrap();
    let (messages, episodes, objective_message_id): (i64, i64, Option<i64>) = db
        .conn()
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM messages
                  WHERE session_id = 'session-1' AND role = 'user'
                    AND content LIKE '%deployment health%'),
                 (SELECT COUNT(*) FROM conversation_episodes
                  WHERE session_id = 'session-1' AND role = 'user'
                    AND body LIKE '%deployment health%'),
                 objective_message_id
             FROM mako_runs WHERE id = 'scheduled'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(messages, 1);
    assert_eq!(episodes, 1);
    assert!(objective_message_id.is_some());
}
