//! Always-on Worker heartbeat wakes.
//!
//! Each fenced tick looks for active `always_on` Workers with a DM lane and
//! no live run. If the last heartbeat is older than the Worker's interval
//! (default 15 minutes), the pump queues one `worker_heartbeat` run on that
//! DM. Pause, archive, or switching autonomy to manual stops future wakes;
//! an in-flight heartbeat is cancelled through the ordinary run-stop path.

use chrono::{Duration as ChronoDuration, Utc};
use mitsuro_core::hive::{canonical_timestamp, RetryPolicy};
use mitsuro_core::storage::{
    load_worker_with_conn, DaemonFence, Database, HiveWorker, HiveWorkerStatus,
    DEFAULT_WORKER_HEARTBEAT_INTERVAL_SECS,
};
use mitsuro_hive_protocol::Actor;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::json;

use super::handler::{insert_canonical_user_message, RuntimeShared};
use super::persistence::{
    append_event, get_or_create_controller, require_owned_session, PersistedEvent,
    RuntimeStoreError,
};

const HEARTBEAT_OBJECTIVE: &str = "Heartbeat: review HEARTBEAT.md and act only if something is due. If nothing needs attention, say so briefly and stop.";
const HEARTBEAT_MAX_ATTEMPTS: u32 = 3;
const NON_TERMINAL: &str =
    "('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')";

pub(super) async fn wake_always_on_workers(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let worker_ids = tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let mut statement = db.conn().prepare(
            "SELECT id FROM hive_workers
             WHERE autonomy = 'always_on'
               AND status = 'active'
               AND dm_session_id IS NOT NULL
             ORDER BY updated_at ASC, id ASC
             LIMIT 32",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok::<_, RuntimeStoreError>(ids)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;

    for worker_id in worker_ids {
        let path = shared.config.database_path.clone();
        let fence = daemon_fence(shared, fencing_token);
        let events = tokio::task::spawn_blocking(move || wake_one_worker(path, worker_id, fence))
            .await
            .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
        for event in events {
            shared.events.publish(event.envelope());
        }
    }
    Ok(())
}

fn daemon_fence(shared: &RuntimeShared, fencing_token: u64) -> DaemonFence {
    DaemonFence {
        lease_name: super::handler::DAEMON_LEASE_NAME.to_string(),
        owner_id: shared.instance_id.clone(),
        fencing_token,
    }
}

fn wake_one_worker(
    path: std::path::PathBuf,
    worker_id: String,
    fence: DaemonFence,
) -> Result<Vec<PersistedEvent>, RuntimeStoreError> {
    let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
    let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
    let now = Utc::now();
    let now_text = canonical_timestamp(now);
    let current = tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM hive_daemon_leases
             WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3
               AND expires_at > ?4
         )",
        params![
            fence.lease_name,
            fence.owner_id,
            fence.fencing_token,
            now_text
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !current {
        tx.commit()?;
        return Ok(Vec::new());
    }

    let Some(worker) =
        load_worker_with_conn(&tx, &worker_id).map_err(RuntimeStoreError::Internal)?
    else {
        tx.commit()?;
        return Ok(Vec::new());
    };
    if worker.status != HiveWorkerStatus::Active
        || worker.autonomy != mitsuro_core::storage::HiveWorkerAutonomy::AlwaysOn
    {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let Some(session_id) = worker
        .dm_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        tx.commit()?;
        return Ok(Vec::new());
    };

    let live: i64 = tx.query_row(
        &format!(
            "SELECT COUNT(*) FROM hive_runs
             WHERE worker_id = ?1 AND status IN {NON_TERMINAL}"
        ),
        [&worker.id],
        |row| row.get(0),
    )?;
    if live > 0 {
        tx.commit()?;
        return Ok(Vec::new());
    }

    let interval_secs = i64::from(
        worker
            .heartbeat_interval_secs
            .unwrap_or(DEFAULT_WORKER_HEARTBEAT_INTERVAL_SECS)
            .max(1),
    );
    let last_finished: Option<String> = tx
        .query_row(
            "SELECT finished_at FROM hive_runs
             WHERE worker_id = ?1 AND kind = 'worker_heartbeat' AND finished_at IS NOT NULL
             ORDER BY finished_at DESC
             LIMIT 1",
            [&worker.id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(finished_at) = last_finished {
        if let Ok(finished) = mitsuro_core::hive::parse_utc_timestamp(&finished_at) {
            if now < finished + ChronoDuration::seconds(interval_secs) {
                tx.commit()?;
                return Ok(Vec::new());
            }
        }
    }

    let events = enqueue_heartbeat(&tx, &worker, session_id, interval_secs, &now_text)?;
    tx.commit()?;
    Ok(events)
}

fn enqueue_heartbeat(
    tx: &Transaction<'_>,
    worker: &HiveWorker,
    session_id: &str,
    interval_secs: i64,
    now: &str,
) -> Result<Vec<PersistedEvent>, RuntimeStoreError> {
    let actor = Actor {
        user_id: worker.user_id.clone(),
        client_kind: "hive-heartbeat".into(),
    };
    let session = require_owned_session(tx, &actor, session_id)?;
    let workspace = session
        .working_dir
        .as_deref()
        .or(session.project_dir.as_deref())
        .map(str::trim)
        .filter(|path| !path.is_empty() && std::path::Path::new(path).is_absolute());
    let Some(working_dir) = workspace else {
        return Ok(Vec::new());
    };
    let model = worker
        .model
        .clone()
        .or_else(|| session.model.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(model) = model else {
        return Ok(Vec::new());
    };
    let controller = get_or_create_controller(tx, &session, now)?;
    let bucket = Utc::now().timestamp().div_euclid(interval_secs);
    let run_id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("mitsuro:hive:worker-heartbeat:{}:{bucket}", worker.id).as_bytes(),
    )
    .to_string();
    insert_canonical_user_message(tx, &session.id, HEARTBEAT_OBJECTIVE, now)?;
    let config = json!({
        "working_dir": working_dir,
        "project_dir": session.project_dir,
        "model": model,
        "model_key": worker.model_key,
        "model_catalog_revision": worker
            .model_catalog_revision
            .clone()
            .or_else(|| session.model_catalog_revision.clone()),
        "permission_mode": worker.permission_mode.as_str(),
        "retry": RetryPolicy::default(),
        "heartbeat": true,
        "worker_id": worker.id,
    });
    tx.execute(
        "INSERT INTO hive_runs (
            id, controller_id, session_id, schedule_id, occurrence_id, kind,
            objective, config_json, status, priority, concurrency_key,
            scheduled_for, available_at, wake_at, attempt_count, max_attempts,
            lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
            last_stop_reason, last_error, outcome_json, created_at, started_at,
            finished_at, updated_at, worker_id
         ) VALUES (
            ?1, ?2, ?3, NULL, NULL, 'worker_heartbeat',
            ?4, ?5, 'queued', 0, ?6,
            ?7, ?7, NULL, 0, ?8, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, ?7, NULL, NULL, ?7, ?9
         )
         ON CONFLICT(id) DO NOTHING",
        params![
            run_id,
            controller.id,
            session.id,
            HEARTBEAT_OBJECTIVE,
            serde_json::to_string(&config)
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
            format!("worker-heartbeat:{}", worker.id),
            now,
            HEARTBEAT_MAX_ATTEMPTS,
            worker.id,
        ],
    )?;
    let inserted = tx.changes() > 0;
    if !inserted {
        return Ok(Vec::new());
    }
    let event = append_event(
        tx,
        &controller,
        "run_queued",
        Some(&run_id),
        None,
        Some(&format!("run:{run_id}:queued")),
        json!({
            "run_id": run_id,
            "kind": "worker_heartbeat",
            "worker_id": worker.id,
        }),
        now,
    )?;
    Ok(vec![event])
}
