use chrono::{Duration as ChronoDuration, Utc};
use rusqlite::params;
use tempfile::TempDir;

use crate::hive::canonical_timestamp;
use crate::storage::{Database, HiveWorker, HiveWorkerStatus, HiveWorkerStore, NewHiveWorker};

use super::store::{
    ack_for_terminal_runs_with_conn, claim_due_with_conn, fail_attempt_with_conn,
    hive_delivery_retry_backoff, mark_delivered_with_conn, revert_wait_with_conn,
};
use super::{
    HiveDeliveryPriority, HiveDeliveryStatus, HiveDeliveryStore, NewHiveDelivery,
    MAX_HIVE_DELIVERY_BODY_BYTES,
};

struct DeliveryWorld {
    db_path: std::path::PathBuf,
    sender: HiveWorker,
    recipient: HiveWorker,
    _temp: TempDir,
}

fn world() -> DeliveryWorld {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("deliveries.db");
    let db = Database::new(&db_path).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('dm-recipient', 'Recipient DM', '2026-08-01T00:00:00.000000Z',
                     '2026-08-01T00:00:00.000000Z', 'hive');",
        )
        .unwrap();
    let workers = HiveWorkerStore::new(Database::new(&db_path).unwrap());
    let sender = workers.create(&NewHiveWorker::new("sender")).unwrap();
    let recipient = workers
        .create(&NewHiveWorker {
            dm_session_id: Some("dm-recipient".into()),
            ..NewHiveWorker::new("recipient")
        })
        .unwrap();
    DeliveryWorld {
        db_path,
        sender,
        recipient,
        _temp: temp,
    }
}

fn store(world: &DeliveryWorld) -> HiveDeliveryStore {
    HiveDeliveryStore::new(Database::new(&world.db_path).unwrap())
}

fn new_delivery(world: &DeliveryWorld) -> NewHiveDelivery {
    NewHiveDelivery {
        from_worker_id: Some(world.sender.id.clone()),
        ..NewHiveDelivery::worker_message(world.recipient.id.clone(), "status update please")
    }
}

fn force_due(world: &DeliveryWorld, id: &str) {
    let db = Database::new(&world.db_path).unwrap();
    db.conn()
        .execute(
            "UPDATE hive_deliveries SET available_at = ?2 WHERE id = ?1",
            params![
                id,
                canonical_timestamp(Utc::now() - ChronoDuration::seconds(1))
            ],
        )
        .unwrap();
}

fn seed_lane_run(world: &DeliveryWorld, run_id: &str, status: &str) {
    let db = Database::new(&world.db_path).unwrap();
    let now = canonical_timestamp(Utc::now());
    db.conn()
        .execute(
            "INSERT OR IGNORE INTO hive_controllers (
                id, scope_key, user_id, session_id, status, timezone,
                max_concurrent_runs, created_at, updated_at
             ) VALUES ('controller-dm', 'session:dm-recipient', NULL, 'dm-recipient',
                       'active', 'UTC', 1, ?1, ?1)",
            [&now],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO hive_runs (
                id, controller_id, session_id, schedule_id, occurrence_id, kind,
                objective, config_json, status, priority, concurrency_key,
                scheduled_for, available_at, wake_at, attempt_count, max_attempts,
                lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
                last_stop_reason, last_error, outcome_json, created_at, started_at,
                finished_at, updated_at
             ) VALUES (?1, 'controller-dm', 'dm-recipient', NULL, NULL, 'dispatch',
                       'busy work', '{}', ?2, 0, NULL, NULL, ?3, NULL, 0, 5,
                       NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?3, NULL,
                       NULL, ?3)",
            params![run_id, status, now],
        )
        .unwrap();
}

fn set_run_status(world: &DeliveryWorld, run_id: &str, status: &str) {
    let db = Database::new(&world.db_path).unwrap();
    db.conn()
        .execute(
            "UPDATE hive_runs SET status = ?2 WHERE id = ?1",
            params![run_id, status],
        )
        .unwrap();
}

#[test]
fn enqueue_round_trips_and_lists_both_directions() {
    let world = world();
    let store = store(&world);
    let outbound = store.enqueue(&new_delivery(&world)).unwrap();
    assert!(!outbound.deduplicated);
    assert_eq!(outbound.delivery.status, HiveDeliveryStatus::Pending);
    assert_eq!(outbound.delivery.attempt_count, 0);

    let inbound = store
        .enqueue(&NewHiveDelivery {
            from_worker_id: Some(world.recipient.id.clone()),
            priority: HiveDeliveryPriority::High,
            ..NewHiveDelivery::worker_message(world.sender.id.clone(), "reply")
        })
        .unwrap();

    // Both directions are visible from either Worker.
    let sender_view = store.list_for_worker(&world.sender.id, None, 50).unwrap();
    assert_eq!(sender_view.len(), 2);
    let recipient_view = store
        .list_for_worker(&world.recipient.id, None, 50)
        .unwrap();
    assert_eq!(recipient_view.len(), 2);

    let pending_only = store
        .list_for_worker(&world.sender.id, Some(HiveDeliveryStatus::Pending), 50)
        .unwrap();
    assert_eq!(pending_only.len(), 2);
    assert!(pending_only
        .iter()
        .any(|delivery| delivery.id == inbound.delivery.id));

    let loaded = store.get(&outbound.delivery.id).unwrap().unwrap();
    assert_eq!(loaded.body, "status update please");
    assert_eq!(loaded.priority, HiveDeliveryPriority::Normal);
}

#[test]
fn enqueue_is_idempotent_on_dedupe_key_and_validates_input() {
    let world = world();
    let store = store(&world);
    let keyed = NewHiveDelivery {
        dedupe_key: Some("run-1:ping".into()),
        ..new_delivery(&world)
    };
    let first = store.enqueue(&keyed).unwrap();
    assert!(!first.deduplicated);
    let replay = store.enqueue(&keyed).unwrap();
    assert!(replay.deduplicated);
    assert_eq!(replay.delivery.id, first.delivery.id);

    // Keyless enqueues never collapse into each other.
    let a = store.enqueue(&new_delivery(&world)).unwrap();
    let b = store.enqueue(&new_delivery(&world)).unwrap();
    assert_ne!(a.delivery.id, b.delivery.id);

    let empty = NewHiveDelivery {
        body: "   ".into(),
        ..new_delivery(&world)
    };
    assert!(store.enqueue(&empty).is_err());
    let oversized = NewHiveDelivery {
        body: "x".repeat(MAX_HIVE_DELIVERY_BODY_BYTES + 1),
        ..new_delivery(&world)
    };
    assert!(store.enqueue(&oversized).is_err());
    let no_budget = NewHiveDelivery {
        max_attempts: 0,
        ..new_delivery(&world)
    };
    assert!(store.enqueue(&no_budget).is_err());
}

#[test]
fn claim_marks_delivering_with_backoff_and_reclaims_after_crash() {
    let world = world();
    let store = store(&world);
    let enqueued = store.enqueue(&new_delivery(&world)).unwrap();

    let db = Database::new(&world.db_path).unwrap();
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, enqueued.delivery.id);
    assert_eq!(claimed[0].status, HiveDeliveryStatus::Delivering);
    assert_eq!(claimed[0].attempt_count, 1);
    // The claim scheduled its own redelivery in the future.
    assert!(claimed[0].available_at > canonical_timestamp(Utc::now()));

    // Nothing further is due while the claim backoff holds.
    assert!(claim_due_with_conn(db.conn(), Utc::now(), 10)
        .unwrap()
        .is_empty());

    // A crash between claim and effect leaves the row delivering; once the
    // backoff elapses it is reclaimed with one more attempt.
    force_due(&world, &enqueued.delivery.id);
    let reclaimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].status, HiveDeliveryStatus::Delivering);
    assert_eq!(reclaimed[0].attempt_count, 2);
}

#[test]
fn normal_priority_waits_for_busy_lane_while_high_claims() {
    let world = world();
    let store = store(&world);
    seed_lane_run(&world, "run-busy", "running");

    let normal = store.enqueue(&new_delivery(&world)).unwrap();
    let high = store
        .enqueue(&NewHiveDelivery {
            priority: HiveDeliveryPriority::High,
            ..new_delivery(&world)
        })
        .unwrap();

    let db = Database::new(&world.db_path).unwrap();
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed.len(), 1, "only the high delivery may claim");
    assert_eq!(claimed[0].id, high.delivery.id);

    // Waiting burned nothing: the normal row is untouched pending.
    let waiting = store.get(&normal.delivery.id).unwrap().unwrap();
    assert_eq!(waiting.status, HiveDeliveryStatus::Pending);
    assert_eq!(waiting.attempt_count, 0);

    set_run_status(&world, "run-busy", "succeeded");
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, normal.delivery.id);
}

#[test]
fn failed_attempts_back_off_then_dead_letter() {
    let world = world();
    let store = store(&world);
    let enqueued = store
        .enqueue(&NewHiveDelivery {
            max_attempts: 2,
            ..new_delivery(&world)
        })
        .unwrap();

    let db = Database::new(&world.db_path).unwrap();
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed[0].attempt_count, 1);
    let status =
        fail_attempt_with_conn(db.conn(), &enqueued.delivery.id, "no DM lane", Utc::now())
            .unwrap();
    assert_eq!(status, HiveDeliveryStatus::Pending);
    let after_first = store.get(&enqueued.delivery.id).unwrap().unwrap();
    assert_eq!(after_first.last_error.as_deref(), Some("no DM lane"));
    assert!(after_first.available_at > canonical_timestamp(Utc::now()));

    force_due(&world, &enqueued.delivery.id);
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed[0].attempt_count, 2);
    let status =
        fail_attempt_with_conn(db.conn(), &enqueued.delivery.id, "still broken", Utc::now())
            .unwrap();
    assert_eq!(status, HiveDeliveryStatus::DeadLetter);
    let dead = store.get(&enqueued.delivery.id).unwrap().unwrap();
    assert_eq!(dead.status, HiveDeliveryStatus::DeadLetter);
    assert_eq!(dead.last_error.as_deref(), Some("still broken"));

    // A crashed claim that already consumed the final attempt dead-letters
    // in the claim sweep instead of looping forever.
    let exhausted = store
        .enqueue(&NewHiveDelivery {
            max_attempts: 1,
            ..new_delivery(&world)
        })
        .unwrap();
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed[0].id, exhausted.delivery.id);
    force_due(&world, &exhausted.delivery.id);
    assert!(claim_due_with_conn(db.conn(), Utc::now(), 10)
        .unwrap()
        .is_empty());
    let swept = store.get(&exhausted.delivery.id).unwrap().unwrap();
    assert_eq!(swept.status, HiveDeliveryStatus::DeadLetter);
    assert_eq!(
        swept.last_error.as_deref(),
        Some("delivery attempts exhausted")
    );
}

#[test]
fn deliveries_to_archived_recipients_dead_letter_at_claim() {
    let world = world();
    let store = store(&world);
    let enqueued = store.enqueue(&new_delivery(&world)).unwrap();
    HiveWorkerStore::new(Database::new(&world.db_path).unwrap())
        .set_status(&world.recipient.id, HiveWorkerStatus::Archived)
        .unwrap();

    let db = Database::new(&world.db_path).unwrap();
    assert!(claim_due_with_conn(db.conn(), Utc::now(), 10)
        .unwrap()
        .is_empty());
    let dead = store.get(&enqueued.delivery.id).unwrap().unwrap();
    assert_eq!(dead.status, HiveDeliveryStatus::DeadLetter);
    assert_eq!(
        dead.last_error.as_deref(),
        Some("the recipient Worker is archived")
    );
}

#[test]
fn paused_recipients_hold_deliveries_pending() {
    let world = world();
    let store = store(&world);
    let enqueued = store.enqueue(&new_delivery(&world)).unwrap();
    let workers = HiveWorkerStore::new(Database::new(&world.db_path).unwrap());
    workers
        .set_status(&world.recipient.id, HiveWorkerStatus::Paused)
        .unwrap();

    let db = Database::new(&world.db_path).unwrap();
    assert!(claim_due_with_conn(db.conn(), Utc::now(), 10)
        .unwrap()
        .is_empty());
    let waiting = store.get(&enqueued.delivery.id).unwrap().unwrap();
    assert_eq!(waiting.status, HiveDeliveryStatus::Pending);
    assert_eq!(waiting.attempt_count, 0);

    workers
        .set_status(&world.recipient.id, HiveWorkerStatus::Active)
        .unwrap();
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, enqueued.delivery.id);
}

#[test]
fn delivered_rows_ack_when_their_run_terminates() {
    let world = world();
    let store = store(&world);
    let woken = store.enqueue(&new_delivery(&world)).unwrap();
    let steered = store
        .enqueue(&NewHiveDelivery {
            priority: HiveDeliveryPriority::High,
            ..new_delivery(&world)
        })
        .unwrap();
    seed_lane_run(&world, "run-woken", "queued");

    let db = Database::new(&world.db_path).unwrap();
    db.conn()
        .execute(
            "UPDATE hive_deliveries SET status = 'delivering', attempt_count = 1",
            [],
        )
        .unwrap();

    // The wake path defers acknowledgement to the run's terminal state.
    assert!(mark_delivered_with_conn(
        db.conn(),
        &woken.delivery.id,
        Some("run-woken"),
        false,
        Utc::now(),
    )
    .unwrap());
    // The steer path acknowledges at delivery time.
    assert!(mark_delivered_with_conn(
        db.conn(),
        &steered.delivery.id,
        Some("run-woken"),
        true,
        Utc::now(),
    )
    .unwrap());
    // Replayed effects cannot double-commit.
    assert!(!mark_delivered_with_conn(
        db.conn(),
        &woken.delivery.id,
        Some("run-woken"),
        false,
        Utc::now(),
    )
    .unwrap());

    assert_eq!(
        store.get(&steered.delivery.id).unwrap().unwrap().status,
        HiveDeliveryStatus::Acked
    );
    assert!(ack_for_terminal_runs_with_conn(db.conn(), Utc::now())
        .unwrap()
        .is_empty());

    set_run_status(&world, "run-woken", "succeeded");
    let acked = ack_for_terminal_runs_with_conn(db.conn(), Utc::now()).unwrap();
    assert_eq!(acked, vec![woken.delivery.id.clone()]);
    let final_state = store.get(&woken.delivery.id).unwrap().unwrap();
    assert_eq!(final_state.status, HiveDeliveryStatus::Acked);
    assert!(final_state.acked_at.is_some());
    assert_eq!(final_state.run_id.as_deref(), Some("run-woken"));
}

#[test]
fn revert_wait_refunds_the_claimed_attempt() {
    let world = world();
    let store = store(&world);
    let enqueued = store.enqueue(&new_delivery(&world)).unwrap();
    let db = Database::new(&world.db_path).unwrap();
    let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
    assert_eq!(claimed[0].attempt_count, 1);

    assert!(revert_wait_with_conn(
        db.conn(),
        &enqueued.delivery.id,
        ChronoDuration::seconds(5),
        Utc::now(),
    )
    .unwrap());
    let reverted = store.get(&enqueued.delivery.id).unwrap().unwrap();
    assert_eq!(reverted.status, HiveDeliveryStatus::Pending);
    assert_eq!(reverted.attempt_count, 0, "waiting is not a failed attempt");
    assert!(reverted.available_at > canonical_timestamp(Utc::now()));
}

#[test]
fn retry_backoff_grows_exponentially_and_caps() {
    assert_eq!(hive_delivery_retry_backoff(1).num_seconds(), 5);
    assert_eq!(hive_delivery_retry_backoff(2).num_seconds(), 10);
    assert_eq!(hive_delivery_retry_backoff(3).num_seconds(), 20);
    assert_eq!(hive_delivery_retry_backoff(7).num_seconds(), 300);
    assert_eq!(hive_delivery_retry_backoff(40).num_seconds(), 300);
}
