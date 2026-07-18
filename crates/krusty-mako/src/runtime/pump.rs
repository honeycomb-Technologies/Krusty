use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use krusty_core::mako::{
    canonical_timestamp, next_retry_at, occurrences_between, parse_timezone, resolve_misfires,
    MakoRunStatus, MisfireDispatch, RetryPolicy,
};
use krusty_core::storage::{
    ClaimRunRequest, ClaimedMakoRun, DaemonFence, DaemonLeaseAcquire, Database,
    MakoDaemonLeaseStore, MakoRun, MakoRunStore, MakoSchedule, MakoScheduleStore, OverlapPolicy,
    RunCompletion,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::backend::{ExecutionEvent, ExecutionEventSink, ExecutionOutcome, ExecutionRequest};
use super::handler::RuntimeShared;
use super::persistence::{append_event, ControllerRecord, PersistedEvent, RuntimeStoreError};

const DAEMON_LEASE_NAME: &str = "mako-scheduler";
const MAX_DUE_OCCURRENCES: usize = 1_000;

pub(crate) async fn run(shared: Arc<RuntimeShared>, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(shared.config.scheduler_poll_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut fencing_token = None;
    let mut executions = JoinSet::new();
    let mut active_sessions = HashMap::new();

    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            completed = executions.join_next(), if !executions.is_empty() => {
                match completed {
                    Some(Ok(run_id)) => {
                        active_sessions.remove(&run_id);
                    }
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, "Mako execution task panicked");
                    }
                    None => {}
                }
            }
            _ = ticker.tick() => {
                match maintain_daemon_lease(&shared, fencing_token).await {
                    Ok(Some(token)) => {
                        let newly_acquired = fencing_token != Some(token);
                        if fencing_token.is_some() && newly_acquired {
                            cancel_active_executions(
                                &shared,
                                &mut executions,
                                &mut active_sessions,
                                "scheduler lease generation changed",
                            ).await;
                        }
                        fencing_token = Some(token);
                        if newly_acquired {
                            if let Err(error) = reconcile_expired(&shared, token).await {
                                tracing::error!(error = ?error, "Mako restart reconciliation failed");
                                continue;
                            }
                        }
                        if let Err(error) = materialize_due_schedules(&shared, token).await {
                            tracing::warn!(error = ?error, "Mako schedule materialization failed");
                        }
                        if let Err(error) = promote_due_runs(&shared, token).await {
                            tracing::warn!(error = %error, "Mako delayed-run promotion failed");
                        }
                        loop {
                            match claim_next(&shared, token).await {
                                Ok(Some(claim)) => {
                                    let run_id = claim.run.id.clone();
                                    if let Some(session_id) = claim.run.session_id.clone() {
                                        active_sessions.insert(run_id.clone(), session_id);
                                    }
                                    let runtime = Arc::clone(&shared);
                                    executions.spawn(async move {
                                        execute_claim(runtime, claim, token).await;
                                        run_id
                                    });
                                }
                                Ok(None) => break,
                                Err(error) => {
                                    tracing::warn!(error = ?error, "Mako run claim failed");
                                    break;
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        if fencing_token.take().is_some() {
                            cancel_active_executions(
                                &shared,
                                &mut executions,
                                &mut active_sessions,
                                "scheduler lease lost",
                            ).await;
                        }
                    }
                    Err(error) => {
                        if fencing_token.take().is_some() {
                            cancel_active_executions(
                                &shared,
                                &mut executions,
                                &mut active_sessions,
                                "scheduler lease maintenance failed",
                            ).await;
                        }
                        tracing::warn!(error = %error, "Mako daemon lease maintenance failed");
                    }
                }
            }
        }
    }

    cancel_active_executions(
        &shared,
        &mut executions,
        &mut active_sessions,
        "Mako runtime shutting down",
    )
    .await;
    if let Some(token) = fencing_token {
        let _ = release_daemon_lease(&shared, token).await;
    }
}

async fn cancel_active_executions(
    shared: &RuntimeShared,
    executions: &mut JoinSet<String>,
    active_sessions: &mut HashMap<String, String>,
    reason: &str,
) {
    let sessions = active_sessions.values().cloned().collect::<HashSet<_>>();
    for session_id in sessions {
        if let Err(error) = shared
            .backend
            .control(
                &session_id,
                super::backend::ExecutionControl::Cancel {
                    reason: reason.to_string(),
                },
            )
            .await
        {
            tracing::warn!(session_id, error = %error, "Mako stale execution cancellation failed");
        }
    }
    executions.abort_all();
    while executions.join_next().await.is_some() {}
    active_sessions.clear();
}

fn daemon_fence(shared: &RuntimeShared, fencing_token: u64) -> DaemonFence {
    DaemonFence {
        lease_name: DAEMON_LEASE_NAME.to_string(),
        owner_id: shared.instance_id.clone(),
        fencing_token,
    }
}

fn daemon_fence_is_current(
    tx: &Transaction<'_>,
    fence: &DaemonFence,
    now: &str,
) -> Result<bool, RuntimeStoreError> {
    let current = tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM mako_daemon_leases
             WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3
               AND expires_at > ?4
         )",
        params![fence.lease_name, fence.owner_id, fence.fencing_token, now],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(current)
}

async fn maintain_daemon_lease(
    shared: &RuntimeShared,
    fencing_token: Option<u64>,
) -> anyhow::Result<Option<u64>> {
    let path = shared.config.database_path.clone();
    let owner = shared.instance_id.clone();
    let duration = shared.config.daemon_lease_duration;
    tokio::task::spawn_blocking(move || {
        let store = MakoDaemonLeaseStore::new(Database::new(&path)?);
        let now = Utc::now();
        if let Some(token) = fencing_token {
            if store.heartbeat(DAEMON_LEASE_NAME, &owner, token, now, duration)? {
                return Ok(Some(token));
            }
        }
        match store.acquire(DAEMON_LEASE_NAME, &owner, now, duration)? {
            DaemonLeaseAcquire::Acquired(lease) => Ok(Some(lease.fencing_token)),
            DaemonLeaseAcquire::HeldByOther { .. } => Ok(None),
        }
    })
    .await?
}

async fn release_daemon_lease(shared: &RuntimeShared, token: u64) -> anyhow::Result<()> {
    let path = shared.config.database_path.clone();
    let owner = shared.instance_id.clone();
    tokio::task::spawn_blocking(move || {
        let store = MakoDaemonLeaseStore::new(Database::new(&path)?);
        store.release(DAEMON_LEASE_NAME, &owner, token)?;
        Ok(())
    })
    .await?
}

async fn reconcile_expired(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let events = tokio::task::spawn_blocking(move || {
        let reconciliation =
            MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?)
                .reconcile_expired_leases_fenced(Utc::now(), &fence)
                .map_err(RuntimeStoreError::Internal)?;
        if reconciliation.requeued_run_ids.is_empty()
            && reconciliation.recovery_required_run_ids.is_empty()
        {
            return Ok::<_, RuntimeStoreError>(Vec::new());
        }

        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        let mut events = Vec::new();
        for (run_id, event_type, reason) in reconciliation
            .requeued_run_ids
            .into_iter()
            .map(|run_id| {
                (
                    run_id,
                    "run_lease_requeued",
                    "worker lease expired before execution; requeued",
                )
            })
            .chain(
                reconciliation
                    .recovery_required_run_ids
                    .into_iter()
                    .map(|run_id| {
                        (
                            run_id,
                            "recovery_required",
                            "worker lease expired; side effects may be uncertain",
                        )
                    }),
            )
        {
            let (controller, attempt_count) = tx.query_row(
                "SELECT c.id, c.session_id, c.status, c.timezone, r.attempt_count
                 FROM mako_controllers c JOIN mako_runs r ON r.controller_id = c.id
                 WHERE r.id = ?1",
                [&run_id],
                |row| {
                    Ok((
                        ControllerRecord {
                            id: row.get(0)?,
                            session_id: row.get(1)?,
                            status: row.get(2)?,
                            timezone: row.get(3)?,
                        },
                        row.get::<_, i64>(4)?,
                    ))
                },
            )?;
            let target_status = if event_type == "run_lease_requeued" {
                "queued"
            } else {
                "recovery_required"
            };
            let dedupe_key = format!("transition:{run_id}:{attempt_count}:{target_status}");
            events.push(append_event(
                &tx,
                &controller,
                event_type,
                Some(&run_id),
                None,
                Some(&dedupe_key),
                serde_json::json!({
                    "run_id": run_id,
                    "reason": reason
                }),
                &now,
            )?);
        }
        tx.commit()?;
        Ok::<_, RuntimeStoreError>(events)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    for event in events {
        shared.events.publish(event.envelope());
    }
    Ok(())
}

async fn materialize_due_schedules(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let now = Utc::now();
    let now_text = canonical_timestamp(now);
    let schedules = tokio::task::spawn_blocking(move || {
        let store =
            MakoScheduleStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .list_due(&now_text, 100)
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;

    for schedule in schedules {
        materialize_schedule(shared, schedule, now, fencing_token).await?;
    }
    Ok(())
}

async fn materialize_schedule(
    shared: &RuntimeShared,
    schedule: MakoSchedule,
    now: DateTime<Utc>,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let timezone = parse_timezone(&schedule.timezone)
        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?;
    let next_fire = schedule
        .next_fire_at
        .as_deref()
        .ok_or_else(|| RuntimeStoreError::Invalid("due schedule has no next fire".into()))
        .and_then(|value| {
            krusty_core::mako::parse_utc_timestamp(value)
                .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))
        })?;
    let after = schedule
        .last_scheduled_for
        .as_deref()
        .map(krusty_core::mako::parse_utc_timestamp)
        .transpose()
        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?
        .unwrap_or_else(|| next_fire - ChronoDuration::microseconds(1));
    let due = occurrences_between(
        &schedule.recurrence,
        timezone,
        after,
        now,
        schedule.dst_policy,
        MAX_DUE_OCCURRENCES,
    )
    .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?;
    if due.is_empty() {
        return Ok(());
    }
    let resolution = resolve_misfires(&due, now, schedule.misfire);
    let last_due = *due.last().expect("due occurrences are non-empty");
    let next_fire = schedule
        .recurrence
        .next_after(timezone, last_due, schedule.dst_policy)
        .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?;
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let events = tokio::task::spawn_blocking(move || {
        materialize_schedule_transaction(path, schedule, resolution, last_due, next_fire, fence)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    for event in events {
        shared.events.publish(event.envelope());
    }
    Ok(())
}

pub(crate) fn materialize_schedule_transaction(
    path: std::path::PathBuf,
    schedule: MakoSchedule,
    resolution: krusty_core::mako::MisfireResolution,
    last_due: DateTime<Utc>,
    next_fire: Option<DateTime<Utc>>,
    fence: DaemonFence,
) -> Result<Vec<PersistedEvent>, RuntimeStoreError> {
    let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
    let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
    let fence_now = canonical_timestamp(Utc::now());
    if !daemon_fence_is_current(&tx, &fence, &fence_now)? {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let persisted_revision = tx
        .query_row(
            "SELECT revision FROM mako_schedules WHERE id = ?1 AND status = 'enabled'",
            [&schedule.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if persisted_revision != Some(schedule.revision as i64) {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let controller = tx.query_row(
        "SELECT id, session_id, status, timezone FROM mako_controllers WHERE id = ?1",
        [&schedule.controller_id],
        |row| {
            Ok(ControllerRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                status: row.get(2)?,
                timezone: row.get(3)?,
            })
        },
    )?;
    let now = canonical_timestamp(Utc::now());
    let mut events = Vec::new();
    for skipped in resolution.skipped {
        materialize_occurrence(
            &tx,
            &controller,
            &schedule,
            skipped,
            None,
            "skipped",
            Some("misfire policy skipped occurrence"),
            0,
            &now,
            &mut events,
        )?;
    }
    for dispatch in resolution.enqueue {
        materialize_dispatch(&tx, &controller, &schedule, dispatch, &now, &mut events)?;
    }
    let next_fire_text = next_fire.map(canonical_timestamp);
    let status = if next_fire_text.is_some() {
        "enabled"
    } else {
        "completed"
    };
    tx.execute(
        "UPDATE mako_schedules SET last_scheduled_for = ?3, next_fire_at = ?4,
             status = ?5, revision = revision + 1, updated_at = ?6
         WHERE id = ?1 AND revision = ?2 AND status = 'enabled'",
        params![
            schedule.id,
            schedule.revision,
            canonical_timestamp(last_due),
            next_fire_text,
            status,
            now
        ],
    )?;
    tx.commit()?;
    Ok(events)
}

fn materialize_dispatch(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    schedule: &MakoSchedule,
    dispatch: MisfireDispatch,
    now: &str,
    events: &mut Vec<PersistedEvent>,
) -> Result<(), RuntimeStoreError> {
    let unfinished: i64 = tx.query_row(
        "SELECT COUNT(*) FROM mako_runs WHERE schedule_id = ?1
         AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        [&schedule.id],
        |row| row.get(0),
    )?;
    let queued_waiting: i64 = tx.query_row(
        "SELECT COUNT(*) FROM mako_runs WHERE schedule_id = ?1
         AND status IN ('queued', 'sleeping', 'retry_wait', 'awaiting_input')",
        [&schedule.id],
        |row| row.get(0),
    )?;
    let (status, reason, should_queue) = match schedule.overlap_policy {
        OverlapPolicy::Allow => ("queued", None, true),
        OverlapPolicy::Skip if unfinished > 0 => {
            ("skipped", Some("overlap policy skipped occurrence"), false)
        }
        OverlapPolicy::QueueOne if queued_waiting > 0 => (
            "coalesced",
            Some("a queued occurrence already exists"),
            false,
        ),
        _ => ("queued", None, true),
    };
    let run_id =
        should_queue.then(|| deterministic_id("run", &schedule.id, dispatch.scheduled_for));
    materialize_occurrence(
        tx,
        controller,
        schedule,
        dispatch.scheduled_for,
        run_id.as_deref(),
        status,
        reason,
        dispatch.coalesced_count as u32,
        now,
        events,
    )?;
    if let Some(run_id) = run_id {
        let occurrence_id = deterministic_id("occurrence", &schedule.id, dispatch.scheduled_for);
        let config_json = serde_json::to_string(&serde_json::json!({
            "project_dir": schedule.project_dir,
            "model": schedule.model,
            "crew_slug": schedule.crew_slug,
            "retry": schedule.retry,
        }))
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        let concurrency_key = (schedule.overlap_policy != OverlapPolicy::Allow)
            .then(|| format!("schedule:{}", schedule.id));
        let scheduled_for = canonical_timestamp(dispatch.scheduled_for);
        tx.execute(
            "INSERT INTO mako_runs (
                id, controller_id, session_id, schedule_id, occurrence_id, kind,
                objective, config_json, status, priority, concurrency_key,
                scheduled_for, available_at, wake_at, attempt_count, max_attempts,
                lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
                last_stop_reason, last_error, outcome_json, created_at, started_at,
                finished_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'scheduled', ?6, ?7, 'queued', ?8, ?9,
                       ?10, ?10, NULL, 0, ?11, NULL, NULL, NULL, NULL, NULL,
                       NULL, NULL, NULL, ?12, NULL, NULL, ?12)
             ON CONFLICT(id) DO NOTHING",
            params![
                run_id,
                controller.id,
                controller.session_id,
                schedule.id,
                occurrence_id,
                schedule.objective,
                config_json,
                schedule.priority,
                concurrency_key,
                scheduled_for,
                schedule.retry.max_attempts,
                now
            ],
        )?;
        events.push(append_event(
            tx,
            controller,
            "run_queued",
            Some(&run_id),
            Some(&schedule.id),
            Some(&format!("run:{run_id}:queued")),
            serde_json::json!({
                "run_id": run_id,
                "schedule_id": schedule.id,
                "scheduled_for": scheduled_for,
            }),
            now,
        )?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize_occurrence(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    schedule: &MakoSchedule,
    scheduled_for: DateTime<Utc>,
    run_id: Option<&str>,
    status: &str,
    reason: Option<&str>,
    coalesced_count: u32,
    now: &str,
    events: &mut Vec<PersistedEvent>,
) -> Result<(), RuntimeStoreError> {
    let occurrence_id = deterministic_id("occurrence", &schedule.id, scheduled_for);
    let scheduled_for = canonical_timestamp(scheduled_for);
    let inserted = tx.execute(
        "INSERT INTO mako_schedule_occurrences (
            id, schedule_id, scheduled_for, run_id, status, decision_reason,
            coalesced_count, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(schedule_id, scheduled_for) DO NOTHING",
        params![
            occurrence_id,
            schedule.id,
            scheduled_for,
            run_id,
            status,
            reason,
            coalesced_count,
            now
        ],
    )?;
    if inserted == 1 && status != "queued" {
        events.push(append_event(
            tx,
            controller,
            if status == "coalesced" {
                "occurrence_coalesced"
            } else {
                "occurrence_skipped"
            },
            None,
            Some(&schedule.id),
            Some(&format!("occurrence:{occurrence_id}:{status}")),
            serde_json::json!({
                "occurrence_id": occurrence_id,
                "scheduled_for": scheduled_for,
                "status": status,
                "reason": reason,
            }),
            now,
        )?);
    }
    Ok(())
}

fn deterministic_id(kind: &str, schedule_id: &str, scheduled_for: DateTime<Utc>) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!(
            "mako:{kind}:{schedule_id}:{}",
            scheduled_for.timestamp_micros()
        )
        .as_bytes(),
    )
    .to_string()
}

async fn promote_due_runs(shared: &RuntimeShared, fencing_token: u64) -> anyhow::Result<()> {
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&path)?);
        store.promote_due_runs_fenced(Utc::now(), &fence)?;
        Ok(())
    })
    .await?
}

async fn claim_next(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<Option<ClaimedMakoRun>, RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let worker_id = shared.instance_id.clone();
    let lease_duration = shared.config.worker_lease_duration;
    let global_concurrency_limit = shared.config.global_concurrency_limit;
    let fence = daemon_fence(shared, fencing_token);
    let claim = tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .claim_next_fenced(
                &ClaimRunRequest {
                    worker_id,
                    lease_epoch: fencing_token,
                    now: Utc::now(),
                    lease_duration,
                    global_concurrency_limit,
                },
                &fence,
            )
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if let Some(claim) = &claim {
        let event = record_run_event(
            shared,
            &claim.run,
            "run_leased",
            serde_json::json!({
                "run_id": claim.run.id,
                "attempt": claim.attempt_no,
                "lease_epoch": fencing_token,
            }),
        )
        .await?;
        shared.events.publish(event.envelope());
    }
    Ok(claim)
}

async fn execute_claim(shared: Arc<RuntimeShared>, claim: ClaimedMakoRun, fencing_token: u64) {
    if let Err(error) = execute_claim_inner(&shared, claim, fencing_token).await {
        tracing::error!(error = ?error, "Mako claimed run execution failed");
    }
}

async fn execute_claim_inner(
    shared: &RuntimeShared,
    claim: ClaimedMakoRun,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let start_gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let fence = daemon_fence(shared, fencing_token);
    let marked = tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .mark_running_fenced(&run_id, &lease_token, fencing_token, Utc::now(), &fence)
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if !marked {
        return Ok(());
    }
    let event = record_run_event(
        shared,
        &claim.run,
        "run_started",
        serde_json::json!({"run_id": claim.run.id, "attempt": claim.attempt_no}),
    )
    .await?;
    shared.events.publish(event.envelope());
    update_occurrence_and_runtime(shared, &claim.run, MakoRunStatus::Running).await?;
    drop(start_gate);

    let (execution_event_tx, mut execution_events) =
        mpsc::channel(shared.config.execution_event_capacity);
    let request = ExecutionRequest {
        claim: claim.clone(),
        daemon_instance_id: shared.instance_id.clone(),
        events: ExecutionEventSink::new(
            execution_event_tx,
            shared.config.max_execution_event_bytes,
        ),
    };
    let execution = shared.backend.execute(request);
    tokio::pin!(execution);
    let mut heartbeat = tokio::time::interval(shared.config.worker_heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;
    let mut outcome = None;
    let mut event_stream_open = true;
    loop {
        if outcome.is_some() && !event_stream_open {
            break;
        }
        tokio::select! {
            completed = &mut execution, if outcome.is_none() => {
                outcome = Some(completed);
                // Reject new emissions but drain every item that was already
                // accepted before committing the terminal run transition.
                execution_events.close();
            }
            event = execution_events.recv(), if event_stream_open => {
                match event {
                    Some(event) => {
                        if !persist_execution_event(shared, &claim, fencing_token, event).await? {
                            cancel_fenced_execution(
                                shared,
                                &claim,
                                "execution event rejected because the scheduler fence was lost",
                            ).await;
                            return Ok(());
                        }
                    }
                    None => event_stream_open = false,
                }
            }
            _ = heartbeat.tick() => {
                if !heartbeat_run(shared, &claim, fencing_token).await? {
                    cancel_fenced_execution(
                        shared,
                        &claim,
                        "worker or scheduler lease fence was lost",
                    ).await;
                    return Ok(());
                }
            }
        }
    }
    let outcome = outcome.ok_or_else(|| {
        RuntimeStoreError::Internal(anyhow::anyhow!("execution ended without an outcome"))
    })?;
    finish_execution(shared, claim, fencing_token, outcome).await
}

async fn heartbeat_run(
    shared: &RuntimeShared,
    claim: &ClaimedMakoRun,
    fencing_token: u64,
) -> Result<bool, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let duration = shared.config.worker_lease_duration;
    let fence = daemon_fence(shared, fencing_token);
    tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .heartbeat_fenced(
                &run_id,
                &lease_token,
                fencing_token,
                Utc::now(),
                duration,
                &fence,
            )
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
}

async fn persist_execution_event(
    shared: &RuntimeShared,
    claim: &ClaimedMakoRun,
    fencing_token: u64,
    execution_event: ExecutionEvent,
) -> Result<bool, RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let schedule_id = claim.run.schedule_id.clone();
    let fence = daemon_fence(shared, fencing_token);
    let persisted = tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &fence, &now)? {
            tx.commit()?;
            return Ok::<_, RuntimeStoreError>(None);
        }
        let controller = tx
            .query_row(
                "SELECT c.id, c.session_id, c.status, c.timezone
                 FROM mako_runs r JOIN mako_controllers c ON c.id = r.controller_id
                 WHERE r.id = ?1 AND r.lease_token = ?2 AND r.lease_epoch = ?3
                   AND r.status = 'running' AND r.lease_expires_at > ?4",
                params![run_id, lease_token, fencing_token, now],
                |row| {
                    Ok(ControllerRecord {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        status: row.get(2)?,
                        timezone: row.get(3)?,
                    })
                },
            )
            .optional()?;
        let Some(controller) = controller else {
            tx.commit()?;
            return Ok(None);
        };
        let event = append_event(
            &tx,
            &controller,
            &execution_event.event_type,
            Some(&run_id),
            schedule_id.as_deref(),
            None,
            execution_event.payload,
            &now,
        )?;
        tx.commit()?;
        Ok(Some(event))
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if let Some(event) = persisted {
        shared.events.publish(event.envelope());
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn cancel_fenced_execution(shared: &RuntimeShared, claim: &ClaimedMakoRun, reason: &str) {
    if let Some(session_id) = claim.run.session_id.as_deref() {
        if let Err(error) = shared
            .backend
            .control(
                session_id,
                super::backend::ExecutionControl::Cancel {
                    reason: reason.to_string(),
                },
            )
            .await
        {
            tracing::warn!(session_id, error = %error, "Mako fenced execution cancellation failed");
        }
    }
}

async fn finish_execution(
    shared: &RuntimeShared,
    claim: ClaimedMakoRun,
    fencing_token: u64,
    outcome: ExecutionOutcome,
) -> Result<(), RuntimeStoreError> {
    let finish_gate = shared.mutation_gate.lock().await;
    let now = Utc::now();
    let (target_status, available_at, wake_at, stop_reason, error, output) =
        completion_for(&claim, outcome, now);
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let completion = RunCompletion {
        target_status,
        now,
        available_at,
        wake_at,
        stop_reason: stop_reason.clone(),
        error: error.clone(),
        outcome: output.clone(),
        trace_sequence_end: None,
    };
    let fence = daemon_fence(shared, fencing_token);
    let persisted = tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .finish_claimed_fenced(&run_id, &lease_token, fencing_token, &completion, &fence)
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    let Some(status) = persisted else {
        return Ok(());
    };
    let event_type = match status {
        MakoRunStatus::Succeeded => "run_completed",
        MakoRunStatus::RetryWait => "run_retry_scheduled",
        MakoRunStatus::Sleeping => "run_sleeping",
        MakoRunStatus::AwaitingInput => "run_awaiting_input",
        MakoRunStatus::RecoveryRequired => "recovery_required",
        MakoRunStatus::Cancelled => "run_cancelled",
        MakoRunStatus::DeadLetter => "run_dead_lettered",
        _ => "run_failed",
    };
    let event = record_run_event(
        shared,
        &claim.run,
        event_type,
        serde_json::json!({
            "run_id": claim.run.id,
            "status": status.as_str(),
            "stop_reason": stop_reason,
            "error": error,
            "outcome": output,
        }),
    )
    .await?;
    shared.events.publish(event.envelope());
    update_occurrence_and_runtime(shared, &claim.run, status).await?;
    drop(finish_gate);
    Ok(())
}

fn completion_for(
    claim: &ClaimedMakoRun,
    outcome: ExecutionOutcome,
    now: DateTime<Utc>,
) -> (
    MakoRunStatus,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
    Option<Value>,
) {
    match outcome {
        ExecutionOutcome::Succeeded { output } => (
            MakoRunStatus::Succeeded,
            None,
            None,
            Some("completed".into()),
            None,
            Some(output),
        ),
        ExecutionOutcome::Failed {
            error,
            retryable,
            retry_after,
        } if retryable => {
            let policy = claim
                .run
                .config
                .get("retry")
                .cloned()
                .and_then(|value| serde_json::from_value::<RetryPolicy>(value).ok())
                .unwrap_or_default();
            let retry_at = next_retry_at(
                now,
                policy,
                claim.attempt_no,
                deterministic_jitter(&claim.run.id, claim.attempt_no),
                retry_after,
            )
            .or(Some(now));
            (
                MakoRunStatus::RetryWait,
                retry_at,
                None,
                Some("transient_failure".into()),
                Some(error),
                None,
            )
        }
        ExecutionOutcome::Failed { error, .. } => (
            MakoRunStatus::Failed,
            None,
            None,
            Some("failed".into()),
            Some(error),
            None,
        ),
        ExecutionOutcome::Sleeping { wake_at, reason } => (
            MakoRunStatus::Sleeping,
            None,
            Some(wake_at),
            reason,
            None,
            None,
        ),
        ExecutionOutcome::AwaitingInput { details } => (
            MakoRunStatus::AwaitingInput,
            None,
            None,
            Some("awaiting_input".into()),
            None,
            Some(details),
        ),
        ExecutionOutcome::RecoveryRequired { reason } => (
            MakoRunStatus::RecoveryRequired,
            None,
            None,
            Some("recovery_required".into()),
            Some(reason),
            None,
        ),
        ExecutionOutcome::Cancelled { reason } => (
            MakoRunStatus::Cancelled,
            None,
            None,
            Some(reason),
            None,
            None,
        ),
    }
}

fn deterministic_jitter(run_id: &str, attempt: u32) -> f64 {
    let mut hasher = DefaultHasher::new();
    run_id.hash(&mut hasher);
    attempt.hash(&mut hasher);
    hasher.finish() as f64 / u64::MAX as f64
}

async fn record_run_event(
    shared: &RuntimeShared,
    run: &MakoRun,
    event_type: &'static str,
    payload: Value,
) -> Result<PersistedEvent, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let controller_id = run.controller_id.clone();
    let run_id = run.id.clone();
    let schedule_id = run.schedule_id.clone();
    let attempt_count = run.attempt_count;
    tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let controller = tx.query_row(
            "SELECT id, session_id, status, timezone FROM mako_controllers WHERE id = ?1",
            [&controller_id],
            |row| {
                Ok(ControllerRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    status: row.get(2)?,
                    timezone: row.get(3)?,
                })
            },
        )?;
        let now = canonical_timestamp(Utc::now());
        let dedupe_key = transition_status_for_event(event_type)
            .map(|status| format!("transition:{run_id}:{attempt_count}:{status}"));
        let event = append_event(
            &tx,
            &controller,
            event_type,
            Some(&run_id),
            schedule_id.as_deref(),
            dedupe_key.as_deref(),
            payload,
            &now,
        )?;
        tx.commit()?;
        Ok::<_, RuntimeStoreError>(event)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
}

fn transition_status_for_event(event_type: &str) -> Option<&'static str> {
    match event_type {
        "run_leased" => Some("leased"),
        "run_started" => Some("running"),
        "run_completed" => Some("succeeded"),
        "run_retry_scheduled" => Some("retry_wait"),
        "run_sleeping" => Some("sleeping"),
        "run_awaiting_input" => Some("awaiting_input"),
        "recovery_required" => Some("recovery_required"),
        "run_cancelled" => Some("cancelled"),
        "run_dead_lettered" => Some("dead_letter"),
        "run_failed" => Some("failed"),
        _ => None,
    }
}

async fn update_occurrence_and_runtime(
    shared: &RuntimeShared,
    run: &MakoRun,
    status: MakoRunStatus,
) -> Result<(), RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let occurrence_id = run.occurrence_id.clone();
    let session_id = run.session_id.clone();
    tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let now = canonical_timestamp(Utc::now());
        if let Some(occurrence_id) = occurrence_id {
            let occurrence_status = match status {
                MakoRunStatus::Succeeded => "succeeded",
                MakoRunStatus::Cancelled => "cancelled",
                MakoRunStatus::Failed | MakoRunStatus::DeadLetter => "failed",
                _ => "running",
            };
            db.conn().execute(
                "UPDATE mako_schedule_occurrences SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![occurrence_id, occurrence_status, now],
            )?;
        }
        if let Some(session_id) = session_id {
            let runtime_status = match status {
                MakoRunStatus::Running => "running",
                MakoRunStatus::Sleeping => "sleeping",
                MakoRunStatus::AwaitingInput => "awaiting_input",
                MakoRunStatus::Cancelled => "cancelled",
                MakoRunStatus::Failed
                | MakoRunStatus::DeadLetter
                | MakoRunStatus::RecoveryRequired => "error",
                _ => "idle",
            };
            db.conn().execute(
                "INSERT INTO mako_runtime_state (session_id, status, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(session_id) DO UPDATE SET status = excluded.status,
                     updated_at = excluded.updated_at",
                params![session_id, runtime_status, now],
            )?;
        }
        Ok::<_, RuntimeStoreError>(())
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
}
