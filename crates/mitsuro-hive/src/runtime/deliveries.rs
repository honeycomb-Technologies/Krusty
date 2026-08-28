//! Durable Worker-to-Worker delivery pump.
//!
//! Each fenced tick claims due `hive_deliveries` rows and applies one
//! atomic effect per row: enqueue a typed recipient DM run or wait/backoff.
//! High priority changes queue priority but never impersonates a user by
//! steering an active run. Crash between claim and
//! effect leaves the row `delivering`; the next due claim replays the
//! effect idempotently.

use chrono::{Duration as ChronoDuration, Utc};
use mitsuro_core::hive::{canonical_timestamp, RetryPolicy};
use mitsuro_core::storage::{
    ack_for_terminal_runs_with_conn, claim_due_with_conn, fail_attempt_with_conn, load_delivery,
    load_worker_with_conn, mark_delivered_with_conn, revert_wait_with_conn, DaemonFence, Database,
    HiveDelivery, HiveDeliveryPriority, HiveDeliveryStatus, HiveWorker,
    HiveWorkerIntroductionStore, HiveWorkerStatus, WorkerConversationLane, WorkerRunOrigin,
};
use mitsuro_hive_protocol::Actor;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::json;

use super::handler::RuntimeShared;
use super::persistence::{
    append_event, get_or_create_controller, require_owned_session, ControllerRecord,
    PersistedEvent, RuntimeStoreError,
};

const WAIT_BACKOFF_SECS: i64 = 5;
const MAX_OBJECTIVE_BYTES: usize = 2 * 1024;
const WORKER_MESSAGE_MAX_ATTEMPTS: u32 = 5;

pub(super) struct DeliveryTick {
    pub(super) events: Vec<PersistedEvent>,
}

pub(super) async fn deliver_worker_messages(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = DaemonFence {
        lease_name: super::handler::DAEMON_LEASE_NAME.to_string(),
        owner_id: shared.instance_id.clone(),
        fencing_token,
    };
    let claimed = tokio::task::spawn_blocking({
        let path = path.clone();
        let fence = fence.clone();
        move || claim_and_ack(path, fence)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;

    let mut tick = DeliveryTick { events: Vec::new() };
    for delivery in claimed {
        let path = path.clone();
        let fence = fence.clone();
        let delivery_id = delivery.id.clone();
        let applied = tokio::task::spawn_blocking(move || {
            apply_claimed_delivery_on_path(path, fence, &delivery_id)
        })
        .await
        .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
        tick.events.extend(applied.events);
    }

    for event in tick.events {
        shared.events.publish(event.envelope());
    }
    Ok(())
}

fn claim_and_ack(
    path: std::path::PathBuf,
    fence: DaemonFence,
) -> Result<Vec<HiveDelivery>, RuntimeStoreError> {
    let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
    let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
    let now = Utc::now();
    let now_text = canonical_timestamp(now);
    if !daemon_fence_is_current(&tx, &fence, &now_text)? {
        tx.commit()?;
        return Ok(Vec::new());
    }
    ack_for_terminal_runs_with_conn(&tx, now).map_err(RuntimeStoreError::Internal)?;
    let claimed = claim_due_with_conn(&tx, now, 32).map_err(RuntimeStoreError::Internal)?;
    tx.commit()?;
    Ok(claimed)
}

fn apply_claimed_delivery_on_path(
    path: std::path::PathBuf,
    fence: DaemonFence,
    delivery_id: &str,
) -> Result<DeliveryTick, RuntimeStoreError> {
    let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
    let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
    let now = Utc::now();
    let now_text = canonical_timestamp(now);
    if !daemon_fence_is_current(&tx, &fence, &now_text)? {
        tx.commit()?;
        return Ok(DeliveryTick { events: Vec::new() });
    }
    let tick = apply_claimed_delivery(&tx, delivery_id, now)?;
    tx.commit()?;
    Ok(tick)
}

fn daemon_fence_is_current(
    tx: &Transaction<'_>,
    fence: &DaemonFence,
    now: &str,
) -> Result<bool, RuntimeStoreError> {
    let current = tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM hive_daemon_leases
             WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3
               AND expires_at > ?4
         )",
        params![fence.lease_name, fence.owner_id, fence.fencing_token, now],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(current)
}

pub(super) fn apply_claimed_delivery(
    tx: &Transaction<'_>,
    delivery_id: &str,
    now: chrono::DateTime<Utc>,
) -> Result<DeliveryTick, RuntimeStoreError> {
    let now_text = canonical_timestamp(now);
    let Some(delivery) = load_delivery(tx, delivery_id).map_err(RuntimeStoreError::Internal)?
    else {
        return Ok(empty_tick());
    };
    if delivery.status != HiveDeliveryStatus::Delivering {
        return Ok(empty_tick());
    }

    let Some(recipient) =
        load_worker_with_conn(tx, &delivery.to_worker_id).map_err(RuntimeStoreError::Internal)?
    else {
        fail_attempt_with_conn(tx, delivery_id, "recipient Worker is missing", now)
            .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    };
    if recipient.status == HiveWorkerStatus::Archived {
        fail_attempt_with_conn(tx, delivery_id, "the recipient Worker is archived", now)
            .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }
    if HiveWorkerIntroductionStore::from_connection(tx)
        .get_by_worker(&recipient.id)
        .map_err(RuntimeStoreError::Internal)?
        .is_some_and(|introduction| !introduction.status.allows_autonomy())
    {
        revert_wait_with_conn(
            tx,
            delivery_id,
            ChronoDuration::seconds(WAIT_BACKOFF_SECS),
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }
    if recipient.status == HiveWorkerStatus::Paused {
        revert_wait_with_conn(
            tx,
            delivery_id,
            ChronoDuration::seconds(WAIT_BACKOFF_SECS),
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }
    let Some(dm_session_id) = recipient
        .dm_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        fail_attempt_with_conn(tx, delivery_id, "the recipient Worker has no DM lane", now)
            .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    };

    let actor = Actor {
        user_id: recipient.user_id.clone(),
        client_kind: "hive-delivery".into(),
    };
    let session = match require_owned_session(tx, &actor, dm_session_id) {
        Ok(session) => session,
        Err(RuntimeStoreError::Ownership | RuntimeStoreError::NotFound(_)) => {
            fail_attempt_with_conn(
                tx,
                delivery_id,
                "the recipient Worker's DM lane is not reachable",
                now,
            )
            .map_err(RuntimeStoreError::Internal)?;
            return Ok(empty_tick());
        }
        Err(error) => return Err(error),
    };
    let controller = get_or_create_controller(tx, &session, &now_text)?;
    let controller_bound = tx.execute(
        "UPDATE hive_controllers
         SET worker_id = ?2, scope_key = ?3, updated_at = ?4
         WHERE id = ?1 AND session_id = ?5 AND user_id IS ?6
           AND (worker_id IS NULL OR worker_id = ?2)",
        params![
            controller.id,
            recipient.id,
            format!("worker:{}", recipient.id),
            now_text,
            session.id,
            recipient.user_id,
        ],
    )?;
    if controller_bound != 1 {
        fail_attempt_with_conn(
            tx,
            delivery_id,
            "the recipient controller belongs to another Worker",
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }
    if controller.status == "paused" {
        revert_wait_with_conn(
            tx,
            delivery_id,
            ChronoDuration::seconds(WAIT_BACKOFF_SECS),
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }
    if controller.status != "active" {
        fail_attempt_with_conn(
            tx,
            delivery_id,
            "the recipient lane is not accepting work",
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }

    let unfinished: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs
         WHERE controller_id = ?1
           AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait',
                          'awaiting_input', 'recovery_required')",
        [&controller.id],
        |row| row.get(0),
    )?;
    if unfinished > 0 && delivery.priority == HiveDeliveryPriority::Normal {
        revert_wait_with_conn(
            tx,
            delivery_id,
            ChronoDuration::seconds(WAIT_BACKOFF_SECS),
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }

    let inbound = inbound_text(tx, &delivery, &recipient)?;
    wake_recipient_lane(
        tx,
        &delivery,
        &recipient,
        &session,
        &controller,
        &inbound,
        now,
    )
}

fn wake_recipient_lane(
    tx: &Transaction<'_>,
    delivery: &HiveDelivery,
    recipient: &HiveWorker,
    session: &super::persistence::OwnedSession,
    controller: &ControllerRecord,
    inbound: &str,
    now: chrono::DateTime<Utc>,
) -> Result<DeliveryTick, RuntimeStoreError> {
    let now_text = canonical_timestamp(now);
    let run_id = deterministic_delivery_run_id(&delivery.id);
    if let Some(existing) = existing_run_id(tx, &run_id)? {
        mark_delivered_with_conn(tx, &delivery.id, Some(&existing), false, now)
            .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    }

    let model = recipient
        .model
        .clone()
        .or_else(|| session.model.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(model) = model else {
        fail_attempt_with_conn(
            tx,
            &delivery.id,
            "the recipient Worker has no frozen model",
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        return Ok(empty_tick());
    };

    // A Worker delivery is a platform event, never a user-authored message.
    // Give it its own deterministic run even when an older run is sleeping or
    // awaiting real user input. The runner injects this run's objective as a
    // typed, ephemeral Worker-message trigger, so delivery replay cannot
    // contaminate canonical chat history or learning evidence.
    let objective = bound_excerpt(inbound);
    let model_key = recipient
        .model_key
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?
        .or_else(|| {
            session
                .model_key
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .ok()
                .flatten()
        });
    let execution = super::worker_context::resolve_worker_conversation_execution_binding(
        tx,
        &session.id,
        &recipient.id,
        recipient.revision,
        WorkerConversationLane::DirectMessage,
    )
    .map_err(RuntimeStoreError::Internal)?;
    let execution_context = execution.context;
    let governor_lane_key = execution_context
        .lane()
        .canonical_lane_key()
        .map_err(RuntimeStoreError::Internal)?;
    let config = json!({
        "model": model,
        "model_key": model_key,
        "model_catalog_revision": recipient
            .model_catalog_revision
            .clone()
            .or_else(|| session.model_catalog_revision.clone()),
        "permission_mode": recipient.permission_mode.as_str(),
        "retry": RetryPolicy::default(),
        "delivery_id": delivery.id,
        "worker_id": recipient.id,
        "working_dir": execution.working_dir,
        "project_dir": execution.project_dir,
    });
    let priority = if delivery.priority == HiveDeliveryPriority::High {
        50
    } else {
        0
    };
    tx.execute(
        "INSERT INTO hive_runs (
            id, controller_id, session_id, schedule_id, occurrence_id, kind,
            objective, config_json, status, priority, concurrency_key,
            scheduled_for, available_at, wake_at, attempt_count, max_attempts,
            lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
            last_stop_reason, last_error, outcome_json, created_at, started_at,
            finished_at, updated_at, worker_id, governor_origin,
            governor_lane_key, execution_context_json
         ) VALUES (
            ?1, ?2, ?3, NULL, NULL, 'worker_message',
            ?4, ?5, 'queued', ?6, NULL,
            ?7, ?7, NULL, 0, ?8, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, ?7, NULL, NULL, ?7, ?9, ?10, ?11, ?12
         )
         ON CONFLICT(id) DO NOTHING",
        params![
            run_id,
            controller.id,
            session.id,
            objective,
            serde_json::to_string(&config)
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
            priority,
            now_text,
            WORKER_MESSAGE_MAX_ATTEMPTS,
            recipient.id,
            WorkerRunOrigin::WorkerPeer.as_str(),
            governor_lane_key,
            serde_json::to_string(&execution_context)
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
        ],
    )?;
    let runtime_status = if controller.status == "paused" {
        "paused"
    } else {
        "idle"
    };
    tx.execute(
        "INSERT INTO hive_runtime_state (session_id, status, current_run_id, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session_id) DO UPDATE SET current_run_id = excluded.current_run_id,
             status = CASE WHEN hive_runtime_state.status = 'paused' THEN 'paused' ELSE excluded.status END,
             updated_at = excluded.updated_at",
        params![session.id, runtime_status, run_id, now_text],
    )?;

    mark_delivered_with_conn(tx, &delivery.id, Some(&run_id), false, now)
        .map_err(RuntimeStoreError::Internal)?;
    let event = append_event(
        tx,
        controller,
        "run_queued",
        Some(&run_id),
        None,
        Some(&format!("delivery:{}:delivered", delivery.id)),
        json!({
            "delivery_id": delivery.id,
            "run_id": run_id,
            "from_worker_id": delivery.from_worker_id,
            "kind": "worker_message",
        }),
        &now_text,
    )?;
    Ok(DeliveryTick {
        events: vec![event],
    })
}

fn existing_run_id(
    tx: &Transaction<'_>,
    run_id: &str,
) -> Result<Option<String>, RuntimeStoreError> {
    Ok(tx
        .query_row("SELECT id FROM hive_runs WHERE id = ?1", [run_id], |row| {
            row.get(0)
        })
        .optional()?)
}

fn inbound_text(
    tx: &Transaction<'_>,
    delivery: &HiveDelivery,
    recipient: &HiveWorker,
) -> Result<String, RuntimeStoreError> {
    let sender = delivery
        .from_worker_id
        .as_deref()
        .map(|id| load_worker_with_conn(tx, id))
        .transpose()
        .map_err(RuntimeStoreError::Internal)?
        .flatten();
    let (name, slug) = match sender {
        Some(worker) => (worker.display_name, worker.slug),
        None => ("Hive".into(), "system".into()),
    };
    let _ = recipient;
    let prefix = if delivery.priority == HiveDeliveryPriority::High {
        format!("High-priority message from Worker {name} (@{slug}):")
    } else {
        format!("Message from Worker {name} (@{slug}):")
    };
    Ok(format!("{prefix}\n{}", delivery.body))
}

fn bound_excerpt(text: &str) -> String {
    if text.len() <= MAX_OBJECTIVE_BYTES {
        return text.to_string();
    }
    let mut end = MAX_OBJECTIVE_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn deterministic_delivery_run_id(delivery_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("mitsuro:hive-delivery:{delivery_id}").as_bytes(),
    )
    .to_string()
}

fn empty_tick() -> DeliveryTick {
    DeliveryTick { events: Vec::new() }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use mitsuro_core::hive::canonical_timestamp;
    use mitsuro_core::storage::{
        claim_due_with_conn, load_delivery, Database, HiveDeliveryPriority, HiveDeliveryStatus,
        HiveDeliveryStore, HiveWorkerStore, NewHiveDelivery, NewHiveWorker,
    };
    use rusqlite::{params, Transaction, TransactionBehavior};
    use tempfile::TempDir;

    use super::{apply_claimed_delivery, deterministic_delivery_run_id};

    struct World {
        db_path: std::path::PathBuf,
        sender_id: String,
        recipient_id: String,
        _temp: TempDir,
    }

    fn world() -> World {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("delivery-pump.db");
        let db = Database::new(&db_path).unwrap();
        let now = canonical_timestamp(Utc::now());
        db.conn()
            .execute_batch(&format!(
                "INSERT INTO sessions (
                    id, title, created_at, updated_at, session_type,
                    model, permission_mode
                 ) VALUES (
                    'dm-recipient', 'Recipient DM', '{now}', '{now}', 'hive',
                    'test-model', 'autonomous'
                 );"
            ))
            .unwrap();
        let workers = HiveWorkerStore::new(Database::new(&db_path).unwrap());
        let sender = workers.create(&NewHiveWorker::new("sender")).unwrap();
        let recipient = workers
            .create(&NewHiveWorker {
                dm_session_id: Some("dm-recipient".into()),
                model: Some("test-model".into()),
                ..NewHiveWorker::new("recipient")
            })
            .unwrap();
        World {
            db_path,
            sender_id: sender.id,
            recipient_id: recipient.id,
            _temp: temp,
        }
    }

    fn enqueue(world: &World, priority: HiveDeliveryPriority) -> String {
        HiveDeliveryStore::new(Database::new(&world.db_path).unwrap())
            .enqueue(&NewHiveDelivery {
                from_worker_id: Some(world.sender_id.clone()),
                priority,
                ..NewHiveDelivery::worker_message(world.recipient_id.clone(), "need a hand")
            })
            .unwrap()
            .delivery
            .id
    }

    fn claim_one(world: &World) -> String {
        let db = Database::new(&world.db_path).unwrap();
        let claimed = claim_due_with_conn(db.conn(), Utc::now(), 10).unwrap();
        assert_eq!(claimed.len(), 1);
        claimed[0].id.clone()
    }

    fn apply(world: &World, id: &str) {
        let db = Database::new(&world.db_path).unwrap();
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
        apply_claimed_delivery(&tx, id, Utc::now()).unwrap();
        tx.commit().unwrap();
    }

    fn user_message_count(world: &World) -> i64 {
        Database::new(&world.db_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = 'dm-recipient' AND role = 'user'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn idle_recipient_wakes_exactly_once_across_replay() {
        let world = world();
        let id = enqueue(&world, HiveDeliveryPriority::Normal);
        let claimed = claim_one(&world);
        assert_eq!(claimed, id);

        apply(&world, &id);
        apply(&world, &id);
        assert_eq!(user_message_count(&world), 0);

        let delivery = HiveDeliveryStore::new(Database::new(&world.db_path).unwrap())
            .get(&id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.status, HiveDeliveryStatus::Delivered);
        let run_id = deterministic_delivery_run_id(&id);
        assert_eq!(delivery.run_id.as_deref(), Some(run_id.as_str()));
        let kind: String = Database::new(&world.db_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT kind FROM hive_runs WHERE id = ?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "worker_message");
    }

    #[test]
    fn crash_between_claim_and_effect_replays_once() {
        let world = world();
        let id = enqueue(&world, HiveDeliveryPriority::Normal);
        let _ = claim_one(&world);
        // Crash: effect never ran. Force the claim backoff due and reclaim.
        Database::new(&world.db_path)
            .unwrap()
            .conn()
            .execute(
                "UPDATE hive_deliveries SET available_at = ?2 WHERE id = ?1",
                params![
                    id,
                    canonical_timestamp(Utc::now() - chrono::Duration::seconds(1))
                ],
            )
            .unwrap();
        let reclaimed = claim_one(&world);
        assert_eq!(reclaimed, id);
        apply(&world, &id);
        assert_eq!(user_message_count(&world), 0);
        let delivery = load_delivery(Database::new(&world.db_path).unwrap().conn(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.status, HiveDeliveryStatus::Delivered);
        assert_eq!(delivery.attempt_count, 2);
    }

    #[test]
    fn waiting_run_is_not_rewritten_as_a_worker_message_or_fake_user_turn() {
        let world = world();
        let now = canonical_timestamp(Utc::now());
        let db = Database::new(&world.db_path).unwrap();
        db.conn()
            .execute(
                "INSERT INTO hive_controllers (
                    id, scope_key, user_id, session_id, status, timezone,
                    max_concurrent_runs, created_at, updated_at
                 ) VALUES ('controller-waiting', 'session:dm-recipient', NULL, 'dm-recipient',
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
                 ) VALUES (
                    'run-waiting', 'controller-waiting', 'dm-recipient', NULL, NULL, 'dispatch',
                    'wait for the actual user', '{}', 'awaiting_input', 0, NULL,
                    ?1, ?1, NULL, 1, 5, NULL, NULL, NULL, NULL, NULL,
                    'awaiting_input', NULL, NULL, ?1, ?1, NULL, ?1
                 )",
                [&now],
            )
            .unwrap();

        // High priority may bypass a waiting lane, but it must do so as its
        // own typed Worker-message run rather than impersonating the user in
        // the older run's canonical transcript.
        let id = enqueue(&world, HiveDeliveryPriority::High);
        let claimed = claim_one(&world);
        apply(&world, &claimed);

        let waiting_status: String = db
            .conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE id = 'run-waiting'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(waiting_status, "awaiting_input");

        let run_id = deterministic_delivery_run_id(&id);
        let (kind, status, objective): (String, String, String) = db
            .conn()
            .query_row(
                "SELECT kind, status, objective FROM hive_runs WHERE id = ?1",
                [&run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "worker_message");
        assert_eq!(status, "queued");
        assert!(objective.contains("message from Worker"));
        assert_eq!(user_message_count(&world), 0);

        let delivery = HiveDeliveryStore::new(Database::new(&world.db_path).unwrap())
            .get(&id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.run_id.as_deref(), Some(run_id.as_str()));
    }

    #[test]
    fn high_priority_enqueues_a_typed_run_without_steering_the_live_run() {
        let world = world();
        let now = canonical_timestamp(Utc::now());
        let db = Database::new(&world.db_path).unwrap();
        db.conn()
            .execute(
                "INSERT INTO hive_controllers (
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
                 ) VALUES ('run-live', 'controller-dm', 'dm-recipient', NULL, NULL, 'dispatch',
                           'busy', '{}', 'running', 0, NULL, NULL, ?1, NULL, 0, 5,
                           'owner', 'lease', 1, ?1, ?1, NULL, NULL, NULL, ?1, ?1, NULL, ?1)",
                [&now],
            )
            .unwrap();

        let id = enqueue(&world, HiveDeliveryPriority::High);
        let claimed = claim_one(&world);
        apply(&world, &claimed);
        let delivery = HiveDeliveryStore::new(Database::new(&world.db_path).unwrap())
            .get(&id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.status, HiveDeliveryStatus::Delivered);
        let run_id = deterministic_delivery_run_id(&id);
        assert_eq!(delivery.run_id.as_deref(), Some(run_id.as_str()));
        let (kind, priority): (String, i64) = db
            .conn()
            .query_row(
                "SELECT kind, priority FROM hive_runs WHERE id = ?1",
                [&run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "worker_message");
        assert_eq!(priority, 50);
        let pending: i64 = Database::new(&world.db_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE session_id = 'dm-recipient' AND role LIKE 'pending_user:%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
        assert_eq!(user_message_count(&world), 0);
    }

    #[test]
    fn recipient_introduction_defers_delivery_without_consuming_an_attempt() {
        let world = world();
        let db = Database::new(&world.db_path).unwrap();
        let now = canonical_timestamp(Utc::now());
        db.conn()
            .execute(
                "INSERT INTO hive_worker_introductions (
                     worker_id, run_id, status, prompt_version, created_at, updated_at
                 ) VALUES (?1, NULL, 'awaiting_context', 1, ?2, ?2)",
                params![world.recipient_id, now],
            )
            .unwrap();
        let id = enqueue(&world, HiveDeliveryPriority::High);
        let claimed = claim_one(&world);
        apply(&world, &claimed);

        let delivery = HiveDeliveryStore::new(Database::new(&world.db_path).unwrap())
            .get(&id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.status, HiveDeliveryStatus::Pending);
        assert_eq!(delivery.attempt_count, 0);
        assert!(delivery.run_id.is_none());
        assert_eq!(user_message_count(&world), 0);
        let runs: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE worker_id = ?1",
                [&world.recipient_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(runs, 0);
    }
}
