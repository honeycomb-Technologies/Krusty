//! Always-on Worker heartbeat wakes.
//!
//! Each fenced tick looks for active `always_on` Workers with a DM lane and
//! no live run. If the last heartbeat is older than the Worker's interval
//! (default 15 minutes), the pump queues one `worker_heartbeat` run on that
//! DM. Pause, archive, or switching autonomy to manual stops future wakes;
//! an in-flight heartbeat is cancelled through the ordinary run-stop path.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use mitsuro_core::ai::models::ModelKey;
use mitsuro_core::hive::{canonical_timestamp, RetryPolicy};
use mitsuro_core::storage::{
    load_worker_with_conn, DaemonFence, Database, HiveWorker, HiveWorkerStatus,
    WorkerConversationLane, WorkerRunOrigin, DEFAULT_WORKER_HEARTBEAT_INTERVAL_SECS,
};
use mitsuro_hive_protocol::Actor;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::json;

use super::handler::RuntimeShared;
use super::persistence::{
    append_event, get_or_create_controller, require_owned_session, PersistedEvent,
    RuntimeStoreError,
};

const HEARTBEAT_OBJECTIVE: &str = "Heartbeat: review HEARTBEAT.md and act only if something is due. If nothing needs attention, say so briefly and stop.";
const HEARTBEAT_MAX_ATTEMPTS: u32 = 3;
const NON_TERMINAL: &str =
    "('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')";
const HEARTBEAT_WAKE_PAGE: usize = 32;

#[derive(Debug)]
struct HeartbeatCandidate {
    worker_id: String,
    created_at: String,
    interval_secs: i64,
    last_finished_at: Option<String>,
    introduction_status: Option<String>,
    model: Option<String>,
    model_key_json: Option<String>,
    model_catalog_revision: Option<String>,
}

pub(super) async fn wake_always_on_workers(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let worker_ids = tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        select_due_worker_ids(db.conn(), Utc::now(), HEARTBEAT_WAKE_PAGE)
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

/// Select eligibility and due time before applying the wake-page cap. This
/// keeps a stable prefix of busy or recently-heartbeated Workers from starving
/// later rows. The query also proves that the DM, owner, workspace and model
/// prerequisites needed by `enqueue_heartbeat` are present.
fn select_due_worker_ids(
    conn: &Connection,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<String>, RuntimeStoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT worker.id, worker.created_at,
                COALESCE(worker.heartbeat_interval_secs, ?1),
                (
                    SELECT run.finished_at
                    FROM hive_runs AS run
                    WHERE run.worker_id = worker.id
                      AND run.kind = 'worker_heartbeat'
                      AND run.finished_at IS NOT NULL
                    ORDER BY run.finished_at DESC, run.id DESC
                    LIMIT 1
                ),
                introduction.status,
                worker.model,
                worker.model_key_json,
                worker.model_catalog_revision
         FROM hive_workers AS worker
         JOIN sessions AS session ON session.id = worker.dm_session_id
         LEFT JOIN hive_worker_introductions AS introduction
           ON introduction.worker_id = worker.id
         WHERE worker.autonomy = 'always_on'
           AND worker.status = 'active'
           AND COALESCE(worker.user_id, '') = COALESCE(session.user_id, '')
           AND NOT EXISTS (
                 SELECT 1 FROM hive_runs AS live
                 WHERE live.worker_id = worker.id
                   AND live.status IN {NON_TERMINAL}
               )"
    );
    let mut statement = conn.prepare(&sql)?;
    let candidates = statement
        .query_map([i64::from(DEFAULT_WORKER_HEARTBEAT_INTERVAL_SECS)], |row| {
            Ok(HeartbeatCandidate {
                worker_id: row.get(0)?,
                created_at: row.get(1)?,
                interval_secs: row.get::<_, i64>(2)?.max(1),
                last_finished_at: row.get(3)?,
                introduction_status: row.get(4)?,
                model: row.get(5)?,
                model_key_json: row.get(6)?,
                model_catalog_revision: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut due = Vec::new();
    for candidate in candidates {
        if !candidate_has_exact_model_identity(&candidate) {
            continue;
        }
        if !introduction_allows_heartbeat(candidate.introduction_status.as_deref()) {
            continue;
        }
        let due_at = candidate
            .last_finished_at
            .as_deref()
            .and_then(|timestamp| mitsuro_core::hive::parse_utc_timestamp(timestamp).ok())
            .and_then(|finished| {
                finished.checked_add_signed(ChronoDuration::seconds(candidate.interval_secs))
            })
            .or_else(|| mitsuro_core::hive::parse_utc_timestamp(&candidate.created_at).ok())
            .unwrap_or(now);
        if due_at <= now {
            due.push((due_at, candidate.worker_id));
        }
    }
    due.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(due
        .into_iter()
        .take(limit)
        .map(|(_, worker_id)| worker_id)
        .collect())
}

fn candidate_has_exact_model_identity(candidate: &HeartbeatCandidate) -> bool {
    let Some(model_key) = candidate
        .model_key_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<ModelKey>(value).ok())
    else {
        return false;
    };
    exact_model_identity(
        candidate.model.as_deref(),
        Some(&model_key),
        candidate.model_catalog_revision.as_deref(),
    )
    .is_some()
}

fn exact_worker_model_identity(worker: &HiveWorker) -> Option<(&str, &ModelKey, Option<&str>)> {
    exact_model_identity(
        worker.model.as_deref(),
        worker.model_key.as_ref(),
        worker.model_catalog_revision.as_deref(),
    )
}

/// Return only a canonical, provider-aware Worker identity. Catalog revision
/// is optional, but when present it must be a real canonical value belonging
/// to this same frozen Worker identity.
fn exact_model_identity<'a>(
    model: Option<&'a str>,
    model_key: Option<&'a ModelKey>,
    model_catalog_revision: Option<&'a str>,
) -> Option<(&'a str, &'a ModelKey, Option<&'a str>)> {
    let model = model.filter(|value| {
        !value.is_empty()
            && *value == value.trim()
            && value.len() <= 512
            && !value.as_bytes().contains(&0)
    })?;
    let model_key = model_key.filter(|key| key.model_id == model)?;
    if model_catalog_revision.is_some_and(|revision| {
        revision.is_empty()
            || revision != revision.trim()
            || revision.len() > 512
            || revision.as_bytes().contains(&0)
    }) {
        return None;
    }
    Some((model, model_key, model_catalog_revision))
}

/// Single policy seam for the introduction lifecycle. Existing Workers with
/// no ledger row remain compatible and operational. A newly-created Worker is
/// held until its introduction is confirmed or explicitly skipped, preventing
/// autonomous work from racing ahead of the user's boundaries.
fn introduction_allows_heartbeat(status: Option<&str>) -> bool {
    status.is_none_or(|status| matches!(status, "confirmed" | "skipped"))
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
    if exact_worker_model_identity(&worker).is_none() {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let introduction_status = tx
        .query_row(
            "SELECT status FROM hive_worker_introductions WHERE worker_id = ?1",
            [&worker.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if !introduction_allows_heartbeat(introduction_status.as_deref()) {
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
    let Some((model, model_key, model_catalog_revision)) = exact_worker_model_identity(worker)
    else {
        return Ok(Vec::new());
    };
    let controller = get_or_create_controller(tx, &session, now)?;
    let controller_bound = tx.execute(
        "UPDATE hive_controllers
         SET worker_id = ?2, scope_key = ?3, updated_at = ?4
         WHERE id = ?1 AND session_id = ?5 AND user_id IS ?6
           AND (worker_id IS NULL OR worker_id = ?2)",
        params![
            controller.id,
            worker.id,
            format!("worker:{}", worker.id),
            now,
            session.id,
            worker.user_id,
        ],
    )?;
    if controller_bound != 1 {
        return Err(RuntimeStoreError::StateConflict(
            "the heartbeat controller belongs to another Worker".into(),
        ));
    }
    let bucket = Utc::now().timestamp().div_euclid(interval_secs);
    let run_id = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("mitsuro:hive:worker-heartbeat:{}:{bucket}", worker.id).as_bytes(),
    )
    .to_string();
    let execution = super::worker_context::resolve_worker_conversation_execution_binding(
        tx,
        &session.id,
        &worker.id,
        worker.revision,
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
        "model_catalog_revision": model_catalog_revision,
        "permission_mode": worker.permission_mode.as_str(),
        "retry": RetryPolicy::default(),
        "heartbeat": true,
        "worker_id": worker.id,
        "working_dir": execution.working_dir,
        "project_dir": execution.project_dir,
    });
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
            ?1, ?2, ?3, NULL, NULL, 'worker_heartbeat',
            ?4, ?5, 'queued', 0, ?6,
            ?7, ?7, NULL, 0, ?8, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, ?7, NULL, NULL, ?7, ?9, ?10, ?11, ?12
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
            WorkerRunOrigin::Heartbeat.as_str(),
            governor_lane_key,
            serde_json::to_string(&execution_context)
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::{Duration as ChronoDuration, Utc};
    use mitsuro_core::ai::models::{ApiFormat, ModelKey};
    use mitsuro_core::ai::providers::ProviderId;
    use mitsuro_core::hive::canonical_timestamp;
    use mitsuro_core::storage::{
        Database, HiveWorker, HiveWorkerAutonomy, HiveWorkerStatus, HiveWorkerStore, NewHiveWorker,
        SessionManager, SessionType, WorkspaceMode,
    };
    use rusqlite::{params, Transaction, TransactionBehavior};
    use tempfile::TempDir;

    use super::{enqueue_heartbeat, select_due_worker_ids, HEARTBEAT_OBJECTIVE};

    const TEST_MODEL: &str = "test:model";
    const TEST_CATALOG_REVISION: &str = "catalog-test";

    fn test_model_key() -> ModelKey {
        ModelKey::new(ProviderId::OpenAI, TEST_MODEL, ApiFormat::OpenAIResponses)
    }

    fn seed_worker(
        path: &std::path::Path,
        slug: &str,
        autonomy: HiveWorkerAutonomy,
        interval_secs: u32,
    ) -> HiveWorker {
        let session_id = SessionManager::new(Database::new(path).unwrap())
            .create_session_for_user_with_config(
                &format!("{slug} DM"),
                Some(TEST_MODEL),
                None,
                None,
                WorkspaceMode::Neutral,
                None,
                None,
                SessionType::Hive,
            )
            .unwrap();
        let model_key = test_model_key();
        Database::new(path)
            .unwrap()
            .conn()
            .execute(
                "UPDATE sessions
                 SET model_key_json = ?2, model_catalog_revision = ?3
                 WHERE id = ?1",
                params![
                    session_id,
                    serde_json::to_string(&model_key).unwrap(),
                    TEST_CATALOG_REVISION,
                ],
            )
            .unwrap();
        HiveWorkerStore::new(Database::new(path).unwrap())
            .create(&NewHiveWorker {
                model: Some(TEST_MODEL.into()),
                model_key: Some(model_key),
                model_catalog_revision: Some(TEST_CATALOG_REVISION.into()),
                dm_session_id: Some(session_id),
                autonomy,
                heartbeat_interval_secs: Some(interval_secs),
                ..NewHiveWorker::new(slug)
            })
            .unwrap()
    }

    fn enqueue_for_worker(path: &std::path::Path, worker: &HiveWorker, now: &str) -> String {
        let db = Database::new(path).unwrap();
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
        enqueue_heartbeat(
            &tx,
            worker,
            worker.dm_session_id.as_deref().unwrap(),
            i64::from(worker.heartbeat_interval_secs.unwrap()),
            now,
        )
        .unwrap();
        let run_id = tx
            .query_row(
                "SELECT id FROM hive_runs
                 WHERE worker_id = ?1 AND kind = 'worker_heartbeat'",
                [&worker.id],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        tx.commit().unwrap();
        run_id
    }

    #[test]
    fn heartbeat_enqueue_is_an_internal_trigger_not_a_user_message() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("heartbeat.db");
        let worker = seed_worker(&path, "honest", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let now = canonical_timestamp(Utc::now());
        let run_id = enqueue_for_worker(&path, &worker, &now);
        let db = Database::new(&path).unwrap();

        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT COUNT(*) FROM messages
                     WHERE session_id = ?1 AND role = 'user'",
                    [worker.dm_session_id.as_deref().unwrap()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "an automated wake must never impersonate the user"
        );
        let (objective, heartbeat): (String, bool) = db
            .conn()
            .query_row(
                "SELECT objective, json_extract(config_json, '$.heartbeat')
                 FROM hive_runs WHERE id = ?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(objective, HEARTBEAT_OBJECTIVE);
        assert!(
            heartbeat,
            "the ephemeral execution trigger remains explicit"
        );
    }

    #[test]
    fn due_selection_requires_an_exact_worker_model_identity() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("heartbeat.db");
        let valid = seed_worker(&path, "valid", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let missing_model =
            seed_worker(&path, "missing-model", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let missing_key = seed_worker(&path, "missing-key", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let mismatched = seed_worker(&path, "mismatched", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let incoherent_catalog = seed_worker(
            &path,
            "incoherent-catalog",
            HiveWorkerAutonomy::AlwaysOn,
            3_600,
        );
        let db = Database::new(&path).unwrap();
        db.conn()
            .execute(
                "UPDATE hive_workers SET model = NULL WHERE id = ?1",
                [&missing_model.id],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE hive_workers SET model_key_json = NULL WHERE id = ?1",
                [&missing_key.id],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE hive_workers SET model = 'other:model' WHERE id = ?1",
                [&mismatched.id],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE hive_workers SET model_catalog_revision = '   ' WHERE id = ?1",
                [&incoherent_catalog.id],
            )
            .unwrap();

        let selected =
            select_due_worker_ids(db.conn(), Utc::now() + ChronoDuration::seconds(1), 32).unwrap();
        assert_eq!(selected, vec![valid.id]);
    }

    #[test]
    fn enqueue_never_falls_back_to_the_session_model() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("heartbeat.db");
        let worker = seed_worker(&path, "invalid", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let now = canonical_timestamp(Utc::now());
        let mut invalid_workers = Vec::new();

        let mut missing_model = worker.clone();
        missing_model.model = None;
        invalid_workers.push(missing_model);

        let mut missing_key = worker.clone();
        missing_key.model_key = None;
        invalid_workers.push(missing_key);

        let mut mismatched = worker.clone();
        mismatched.model = Some("other:model".into());
        invalid_workers.push(mismatched);

        let mut incoherent_catalog = worker.clone();
        incoherent_catalog.model_catalog_revision = Some("   ".into());
        invalid_workers.push(incoherent_catalog);

        for invalid_worker in invalid_workers {
            let db = Database::new(&path).unwrap();
            let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
            let events = enqueue_heartbeat(
                &tx,
                &invalid_worker,
                invalid_worker.dm_session_id.as_deref().unwrap(),
                i64::from(invalid_worker.heartbeat_interval_secs.unwrap()),
                &now,
            )
            .unwrap();
            assert!(events.is_empty());
            tx.commit().unwrap();
        }

        assert_eq!(
            Database::new(&path)
                .unwrap()
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM hive_runs WHERE worker_id = ?1",
                    [&worker.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn due_selection_excludes_paused_live_and_recent_workers() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("heartbeat.db");
        let due = seed_worker(&path, "due", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let queued = seed_worker(&path, "queued", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let introducing = seed_worker(&path, "introducing", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let review_ready = seed_worker(&path, "review-ready", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let confirmed = seed_worker(&path, "confirmed", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let skipped = seed_worker(&path, "skipped", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let paused = seed_worker(&path, "paused", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let live = seed_worker(&path, "live", HiveWorkerAutonomy::AlwaysOn, 3_600);
        let recent = seed_worker(&path, "recent", HiveWorkerAutonomy::AlwaysOn, 3_600);
        HiveWorkerStore::new(Database::new(&path).unwrap())
            .set_status(&paused.id, HiveWorkerStatus::Paused)
            .unwrap();

        let now = Utc::now();
        let now_text = canonical_timestamp(now);
        let introduction_db = Database::new(&path).unwrap();
        introduction_db
            .conn()
            .execute(
                "INSERT INTO hive_worker_introductions (
                    worker_id, status, prompt_version, created_at, updated_at
                 ) VALUES (?1, 'queued', 1, ?6, ?6),
                          (?2, 'awaiting_context', 1, ?6, ?6),
                          (?3, 'review_ready', 1, ?6, ?6),
                          (?4, 'confirmed', 1, ?6, ?6),
                          (?5, 'skipped', 1, ?6, ?6)",
                params![
                    queued.id,
                    introducing.id,
                    review_ready.id,
                    confirmed.id,
                    skipped.id,
                    now_text
                ],
            )
            .unwrap();
        enqueue_for_worker(&path, &live, &now_text);
        let recent_run = enqueue_for_worker(&path, &recent, &now_text);
        Database::new(&path)
            .unwrap()
            .conn()
            .execute(
                "UPDATE hive_runs
                 SET status = 'succeeded', finished_at = ?2, updated_at = ?2
                 WHERE id = ?1",
                params![recent_run, now_text],
            )
            .unwrap();

        let selected = select_due_worker_ids(
            Database::new(&path).unwrap().conn(),
            now + ChronoDuration::seconds(1),
            32,
        )
        .unwrap();
        assert_eq!(
            selected.into_iter().collect::<BTreeSet<_>>(),
            [due.id, confirmed.id, skipped.id].into_iter().collect()
        );
    }

    #[test]
    fn due_selection_pages_past_more_than_32_busy_workers() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("heartbeat.db");
        let workers = (0..40)
            .map(|index| {
                seed_worker(
                    &path,
                    &format!("worker-{index:02}"),
                    HiveWorkerAutonomy::AlwaysOn,
                    3_600,
                )
            })
            .collect::<Vec<_>>();
        let all_ids = workers
            .iter()
            .map(|worker| worker.id.clone())
            .collect::<BTreeSet<_>>();
        let now = Utc::now();
        let now_text = canonical_timestamp(now);
        let first_page = select_due_worker_ids(
            Database::new(&path).unwrap().conn(),
            now + ChronoDuration::seconds(1),
            32,
        )
        .unwrap();
        assert_eq!(first_page.len(), 32);

        for worker_id in &first_page {
            let worker = HiveWorkerStore::new(Database::new(&path).unwrap())
                .get(worker_id)
                .unwrap()
                .unwrap();
            enqueue_for_worker(&path, &worker, &now_text);
        }

        let second_page = select_due_worker_ids(
            Database::new(&path).unwrap().conn(),
            now + ChronoDuration::seconds(2),
            32,
        )
        .unwrap();
        let first_ids = first_page.iter().cloned().collect::<BTreeSet<_>>();
        let expected = all_ids
            .difference(&first_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            second_page.iter().cloned().collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(second_page.len(), 8);
    }
}
