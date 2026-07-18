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

    let reconciled = store.reconcile_expired_leases(instant(11)).unwrap();
    assert_eq!(reconciled.requeued_unstarted, 0);
    assert_eq!(reconciled.recovery_required, 1);
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        MakoRunStatus::RecoveryRequired
    );
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
