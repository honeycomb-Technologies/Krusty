use std::collections::{hash_map::DefaultHasher, HashMap};
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
    ReconciledRun, RunCompletion,
};
use krusty_mako_protocol::{
    unix_time_millis, EventEnvelope, ExtensionEvent, MakoEvent, ProtocolVersion, RuntimeEvent,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::backend::{ExecutionEvent, ExecutionEventSink, ExecutionOutcome, ExecutionRequest};
use super::config::MAX_ABORT_DELIVERY_TIMEOUT;
use super::handler::{
    CommittedCancellation, RuntimeShared, DAEMON_LEASE_NAME, MAX_RETRY_ATTEMPTS,
    MAX_RETRY_DELAY_SECS,
};
use super::persistence::{append_event, ControllerRecord, PersistedEvent, RuntimeStoreError};

const MAX_DUE_OCCURRENCES: usize = 1_000;
const EVENT_JOURNAL_EXHAUSTED_REASON: &str =
    "durable event journal exhausted; execution side effects may be uncertain";
const FORCED_CANCELLATION_STOP_REASON: &str = "cancellation grace elapsed";
const FORCED_CANCELLATION_ERROR: &str =
    "execution host did not acknowledge cancellation before the deadline; side effects may be uncertain";
struct PumpLivenessGuard(Arc<RuntimeShared>);

impl Drop for PumpLivenessGuard {
    fn drop(&mut self) {
        self.0.health.mark_pump_stopped();
    }
}

pub(crate) async fn run(shared: Arc<RuntimeShared>, mut shutdown: watch::Receiver<bool>) {
    shared.health.mark_pump_running();
    let _liveness = PumpLivenessGuard(Arc::clone(&shared));
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
                        if newly_acquired {
                            shared.health.set_scheduler_activated(false);
                        }
                        if fencing_token.is_some() && newly_acquired {
                            cancel_active_executions(
                                &shared,
                                &mut executions,
                                &mut active_sessions,
                                "scheduler lease generation changed",
                            ).await;
                        }
                        fencing_token = Some(token);
                        // A replacement daemon can acquire its lease before
                        // the previous owner's longer worker leases expire.
                        // Reconcile on every fenced tick so those runs cannot
                        // remain permanently active after their later expiry.
                        if let Err(error) = reconcile_expired(&shared, token).await {
                            shared.health.set_scheduler_activated(false);
                            tracing::error!(error = ?error, "Mako lease reconciliation failed");
                            continue;
                        }
                        shared.health.set_scheduler_activated(true);
                        if let Err(error) = deliver_pending_control(&shared, token).await {
                            tracing::warn!(error = ?error, "Mako durable control delivery failed");
                        }
                        if let Err(error) = materialize_due_schedules(&shared, token).await {
                            tracing::warn!(error = ?error, "Mako schedule materialization failed");
                        }
                        if let Err(error) = promote_due_runs(&shared, token).await {
                            tracing::warn!(error = ?error, "Mako delayed-run promotion failed");
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
                        shared.health.set_scheduler_activated(false);
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
                        shared.health.set_scheduler_activated(false);
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
    shared.health.set_scheduler_activated(false);
    if let Some(token) = fencing_token {
        let _ = release_daemon_lease(&shared, token).await;
    }
}

#[derive(Debug)]
struct PendingToolApproval {
    id: String,
    controller: ControllerRecord,
    session_id: String,
    run_id: String,
    tool_call_id: String,
    approved: bool,
}

async fn deliver_pending_control(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    // Serialize selection, host delivery, and acknowledgement with user
    // mutations and run completion. This prevents a local Pause/Cancel or
    // terminal transition from overtaking a queued authorization decision.
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let pending = tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &fence, &now)? {
            tx.commit()?;
            return Ok::<_, RuntimeStoreError>(None);
        }
        let pending = tx
            .query_row(
                "SELECT o.id, c.id, c.session_id, c.status, c.timezone,
                        o.session_id, o.run_id, o.payload_json
                 FROM mako_control_outbox o
                 JOIN mako_runs r ON r.id = o.run_id
                 JOIN mako_controllers c ON c.id = o.controller_id
                 WHERE o.status = 'pending' AND o.available_at <= ?1
                   AND r.status IN ('leased', 'running')
                   AND r.lease_epoch = ?2 AND r.lease_expires_at > ?1
                   AND c.status = 'active'
                 ORDER BY o.available_at, o.created_at, o.id LIMIT 1",
                params![now, fence.fencing_token],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        ControllerRecord {
                            id: row.get(1)?,
                            session_id: row.get(2)?,
                            status: row.get(3)?,
                            timezone: row.get(4)?,
                        },
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        tx.commit()?;
        let Some((id, controller, session_id, run_id, payload)) = pending else {
            return Ok(None);
        };
        let payload = serde_json::from_str::<Value>(&payload)
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        let tool_call_id = payload
            .get("tool_call_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RuntimeStoreError::Invalid("invalid tool approval outbox payload".into())
            })?
            .to_string();
        let approved = payload
            .get("approved")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                RuntimeStoreError::Invalid("invalid tool approval outbox payload".into())
            })?;
        Ok(Some(PendingToolApproval {
            id,
            controller,
            session_id,
            run_id,
            tool_call_id,
            approved,
        }))
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;

    let Some(pending) = pending else {
        return Ok(());
    };
    let delivery = shared
        .backend
        .control(
            &pending.session_id,
            super::backend::ExecutionControl::ToolApproval {
                run_id: pending.run_id.clone(),
                tool_call_id: pending.tool_call_id.clone(),
                approved: pending.approved,
            },
        )
        .await;
    let path = shared.config.database_path.clone();
    let event_fence = daemon_fence(shared, fencing_token);
    let retry_delay = shared.config.scheduler_poll_interval;
    let pending_id = pending.id.clone();
    let persisted = tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = Utc::now();
        let now_text = canonical_timestamp(now);
        if !daemon_fence_is_current(&tx, &event_fence, &now_text)? {
            tx.commit()?;
            return Ok::<_, RuntimeStoreError>(None);
        }
        let event = match delivery {
            Ok(()) => {
                let changed = tx.execute(
                    "UPDATE mako_control_outbox
                     SET status = 'delivered', attempt_count = attempt_count + 1,
                         delivered_at = ?2, last_error = NULL, updated_at = ?2
                     WHERE id = ?1 AND status = 'pending'",
                    params![pending_id, now_text],
                )?;
                if changed == 0 {
                    tx.commit()?;
                    return Ok(None);
                }
                Some(append_event(
                    &tx,
                    &pending.controller,
                    "tool_approval_delivered",
                    Some(&pending.run_id),
                    None,
                    Some(&format!("tool_approval_delivered:{}", pending.id)),
                    serde_json::json!({
                        "run_id": pending.run_id,
                        "tool_call_id": pending.tool_call_id,
                        "approved": pending.approved,
                    }),
                    &now_text,
                )?)
            }
            Err(_error) => {
                let retry_at = now
                    + ChronoDuration::from_std(retry_delay)
                        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
                tx.execute(
                    "UPDATE mako_control_outbox
                     SET attempt_count = attempt_count + 1, available_at = ?2,
                         last_error = ?3, updated_at = ?4
                     WHERE id = ?1 AND status = 'pending'",
                    params![
                        pending_id,
                        canonical_timestamp(retry_at),
                        "execution host control delivery failed",
                        now_text
                    ],
                )?;
                None
            }
        };
        tx.commit()?;
        Ok(event)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if let Some(event) = persisted {
        shared.events.publish(event.envelope());
    }
    Ok(())
}

async fn cancel_active_executions(
    shared: &RuntimeShared,
    executions: &mut JoinSet<String>,
    active_sessions: &mut HashMap<String, String>,
    reason: &str,
) {
    let active = active_sessions
        .iter()
        .map(|(run_id, session_id)| (run_id.clone(), session_id.clone()))
        .collect::<Vec<_>>();
    // Drop every scheduler-owned execution future first. The concrete Mako
    // backend's execution guard aborts its hosted runner on drop, so fencing
    // loss stops side effects immediately instead of waiting behind a serial
    // queue of cooperative cancellation grace periods.
    executions.abort_all();
    while executions.join_next().await.is_some() {}
    active_sessions.clear();

    let mut cancellations = JoinSet::new();
    for (run_id, session_id) in active {
        let backend = Arc::clone(&shared.backend);
        let reason = reason.to_string();
        cancellations.spawn(async move {
            let result = backend
                .control(
                    &session_id,
                    super::backend::ExecutionControl::CancelRun { run_id, reason },
                )
                .await;
            (session_id, result)
        });
    }
    let drain = async {
        while let Some(result) = cancellations.join_next().await {
            match result {
                Ok((session_id, Err(error))) => {
                    tracing::warn!(session_id, error = %error, "Mako stale execution cancellation failed");
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Mako stale execution cancellation task failed");
                }
                Ok((_, Ok(()))) => {}
            }
        }
    };
    if tokio::time::timeout(shared.config.worker_heartbeat_interval, drain)
        .await
        .is_err()
    {
        tracing::warn!(
            timeout_ms = shared.config.worker_heartbeat_interval.as_millis(),
            "Timed out draining stale execution cancellations"
        );
        cancellations.abort_all();
        while cancellations.join_next().await.is_some() {}
    }
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
    let event_fence = fence.clone();
    let events = tokio::task::spawn_blocking(move || {
        let reconciliation =
            MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?)
                .reconcile_expired_leases_fenced(Utc::now(), &fence)
                .map_err(RuntimeStoreError::Internal)?;
        if reconciliation.requeued_runs.is_empty()
            && reconciliation.recovery_required_runs.is_empty()
        {
            return Ok::<_, RuntimeStoreError>(Vec::new());
        }

        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &event_fence, &now)? {
            tx.commit()?;
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for (reconciled, event_type, reason) in reconciliation
            .requeued_runs
            .into_iter()
            .map(|reconciled| {
                (
                    reconciled,
                    "run_lease_requeued",
                    "worker lease expired before execution; requeued",
                )
            })
            .chain(
                reconciliation
                    .recovery_required_runs
                    .into_iter()
                    .map(|reconciled| {
                        (
                            reconciled,
                            "recovery_required",
                            "worker lease expired; side effects may be uncertain",
                        )
                    }),
            )
        {
            let run_id = reconciled.run_id;
            let controller = tx.query_row(
                "SELECT c.id, c.session_id, c.status, c.timezone
                 FROM mako_controllers c JOIN mako_runs r ON r.controller_id = c.id
                 WHERE r.id = ?1",
                [&run_id],
                |row| {
                    Ok(ControllerRecord {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        status: row.get(2)?,
                        timezone: row.get(3)?,
                    })
                },
            )?;
            let target_status = if event_type == "run_lease_requeued" {
                "queued"
            } else {
                "recovery_required"
            };
            let dedupe_key = format!(
                "transition:{run_id}:{}:{target_status}",
                reconciled.attempt_no
            );
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
    let permission_mode = tx
        .query_row(
            "SELECT permission_mode FROM sessions WHERE id = ?1",
            [&controller.session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let now = canonical_timestamp(Utc::now());
    let invalid_config = if schedule
        .model
        .as_deref()
        .is_none_or(|model| model.trim().is_empty())
    {
        Some("schedule has no frozen model")
    } else if schedule
        .model_key
        .as_ref()
        .is_some_and(|key| schedule.model.as_deref() != Some(key.model_id.as_str()))
        || (schedule.model_key.is_none() && schedule.model_catalog_revision.is_some())
    {
        Some("schedule has inconsistent frozen model identity")
    } else if schedule
        .project_dir
        .as_deref()
        .is_none_or(|path| path.trim().is_empty() || !std::path::Path::new(path).is_absolute())
    {
        Some("schedule has no frozen workspace")
    } else if permission_mode
        .as_deref()
        .is_none_or(|mode| !matches!(mode, "supervised" | "autonomous"))
    {
        Some("schedule session has no valid frozen permission mode")
    } else {
        None
    };
    if let Some(reason) = invalid_config {
        tx.execute(
            "UPDATE mako_schedules SET status = 'paused', revision = revision + 1,
                 updated_at = ?3 WHERE id = ?1 AND revision = ?2 AND status = 'enabled'",
            params![schedule.id, schedule.revision, now],
        )?;
        let event = append_event(
            &tx,
            &controller,
            "schedule_paused_invalid_config",
            None,
            Some(&schedule.id),
            Some(&format!(
                "schedule:{}:invalid-config:{}",
                schedule.id, schedule.revision
            )),
            serde_json::json!({"schedule_id": schedule.id, "reason": reason}),
            &now,
        )?;
        tx.commit()?;
        return Ok(vec![event]);
    }
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
        materialize_dispatch(
            &tx,
            &controller,
            &schedule,
            permission_mode
                .as_deref()
                .expect("validated schedule permission mode"),
            dispatch,
            &now,
            &mut events,
        )?;
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
    permission_mode: &str,
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
            "working_dir": schedule.project_dir.clone(),
            "project_dir": schedule.project_dir,
            "model": schedule.model,
            "model_key": schedule.model_key,
            "model_catalog_revision": schedule.model_catalog_revision,
            "permission_mode": permission_mode,
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

async fn promote_due_runs(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let promoted = tokio::task::spawn_blocking(move || {
        let store = MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .promote_due_runs_fenced(Utc::now(), &fence)
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    for run in promoted {
        if let Some(event) = record_promoted_run_event(shared, fencing_token, run).await? {
            shared.events.publish(event.envelope());
        }
    }
    Ok(())
}

async fn record_promoted_run_event(
    shared: &RuntimeShared,
    fencing_token: u64,
    promoted: ReconciledRun,
) -> Result<Option<PersistedEvent>, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &fence, &now)? {
            tx.commit()?;
            return Ok(None);
        }
        let (controller, schedule_id) = tx.query_row(
            "SELECT c.id, c.session_id, c.status, c.timezone, r.schedule_id
             FROM mako_runs r JOIN mako_controllers c ON c.id = r.controller_id
             WHERE r.id = ?1",
            [&promoted.run_id],
            |row| {
                Ok((
                    ControllerRecord {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        status: row.get(2)?,
                        timezone: row.get(3)?,
                    },
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )?;
        let dedupe_key = format!(
            "transition:{}:{}:queued",
            promoted.run_id, promoted.attempt_no
        );
        let event = append_event(
            &tx,
            &controller,
            "run_requeued",
            Some(&promoted.run_id),
            schedule_id.as_deref(),
            Some(&dedupe_key),
            serde_json::json!({
                "run_id": promoted.run_id,
                "reason": "wake or retry became due",
            }),
            &now,
        )?;
        tx.commit()?;
        Ok(Some(event))
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
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
    // Subscribe before entering the durable running boundary. If an
    // ownership-checked CancelSession commits at any point after this, this
    // exact worker receives the signal; if it committed earlier,
    // mark_running_fenced rejects the disabled controller.
    let mut cancellation_rx = shared.cancellation_tx.subscribe();
    let mut cancellation_signals_open = true;
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
    drop(start_gate);

    // `mark_running_fenced` and its event/state projection can be delayed by
    // I/O. Revalidate immediately before constructing the backend future so a
    // superseded daemon cannot begin external side effects during that gap.
    if !heartbeat_run(shared, &claim, fencing_token).await? {
        if cancellation_committed_for_claim(shared, &claim, fencing_token).await? {
            return finish_committed_cancellation(shared, &claim, fencing_token, false).await;
        }
        cancel_fenced_execution(
            shared,
            &claim,
            "execution start rejected because the scheduler fence was lost",
        )
        .await;
        return Ok(());
    }

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
    let mut cancellation_deadline = None;
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
                        match persist_execution_event(shared, &claim, fencing_token, event).await {
                            Ok(true) => {}
                            Ok(false) => {
                                cancel_fenced_execution(
                                    shared,
                                    &claim,
                                    "execution event rejected because the scheduler fence was lost",
                                ).await;
                                return Ok(());
                            }
                            Err(RuntimeStoreError::ResourceExhausted(_)) => {
                                // The event may represent an external side effect
                                // or an approval/question boundary. Never continue
                                // unmanaged and never wait for lease expiry: cancel
                                // this exact hosted run, then durably require an
                                // operator recovery decision using a fixed reason.
                                cancel_fenced_execution(
                                    shared,
                                    &claim,
                                    EVENT_JOURNAL_EXHAUSTED_REASON,
                                ).await;
                                return finish_execution(
                                    shared,
                                    claim,
                                    fencing_token,
                                    ExecutionOutcome::RecoveryRequired {
                                        reason: EVENT_JOURNAL_EXHAUSTED_REASON.into(),
                                    },
                                ).await;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    None => event_stream_open = false,
                }
            }
            cancellation = cancellation_rx.recv(), if cancellation_signals_open && cancellation_deadline.is_none() => {
                let committed = match cancellation {
                    Ok(cancellation) => cancellation_matches_claim(&cancellation, &claim)
                        && cancellation_committed_for_claim(shared, &claim, fencing_token).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // The durable controller state is authoritative when a
                        // burst overran this bounded optimization channel.
                        cancellation_committed_for_claim(shared, &claim, fencing_token).await?
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        cancellation_signals_open = false;
                        false
                    }
                };
                if committed {
                    cancellation_deadline = Some(
                        begin_cooperative_cancellation(shared, &claim).await
                    );
                }
            }
            _ = tokio::time::sleep_until(
                cancellation_deadline.unwrap_or_else(Instant::now)
            ), if cancellation_deadline.is_some() => {
                return finish_committed_cancellation(shared, &claim, fencing_token, true).await;
            }
            _ = heartbeat.tick() => {
                if !heartbeat_run(shared, &claim, fencing_token).await? {
                    if cancellation_committed_for_claim(shared, &claim, fencing_token).await? {
                        if cancellation_deadline.is_none() {
                            cancellation_deadline = Some(
                                begin_cooperative_cancellation(shared, &claim).await
                            );
                        }
                        continue;
                    }
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
    let envelope = tokio::task::spawn_blocking(move || {
        let ExecutionEvent {
            event_type,
            payload,
            durable_payload,
        } = execution_event;
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &fence, &now)? {
            tx.commit()?;
            return Ok::<_, RuntimeStoreError>(None);
        }
        // A committed CancelSession disables the controller before the hosted
        // loop acknowledges cancellation. The exact running lease remains the
        // authority for that short terminal window: accept its bounded event
        // so `finish_execution` can durably close the run as cancelled. New
        // starts and heartbeats still require an active controller.
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
        let sequence = if let Some(durable_payload) = durable_payload {
            Some(
                append_event(
                    &tx,
                    &controller,
                    &event_type,
                    Some(&run_id),
                    schedule_id.as_deref(),
                    None,
                    durable_payload,
                    &now,
                )?
                .sequence,
            )
        } else {
            None
        };
        tx.commit()?;
        Ok(Some(execution_event_envelope(
            controller.session_id,
            run_id,
            sequence,
            event_type,
            payload,
        )))
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if let Some(envelope) = envelope {
        shared.events.publish(envelope);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn execution_event_envelope(
    session_id: String,
    run_id: String,
    sequence: Option<i64>,
    event_type: String,
    payload: Value,
) -> EventEnvelope {
    let event = if event_type == "agentic_event" {
        MakoEvent::Extension(ExtensionEvent {
            name: "agentic_event".into(),
            payload,
        })
    } else {
        MakoEvent::Runtime(RuntimeEvent {
            event_type,
            payload,
        })
    };
    EventEnvelope {
        version: ProtocolVersion::CURRENT,
        session_id: Some(session_id),
        run_id: Some(run_id),
        sequence,
        emitted_at_unix_ms: unix_time_millis(),
        event,
    }
}

async fn cancel_fenced_execution(shared: &RuntimeShared, claim: &ClaimedMakoRun, reason: &str) {
    if let Some(session_id) = claim.run.session_id.as_deref() {
        if let Err(error) = shared
            .backend
            .control(
                session_id,
                super::backend::ExecutionControl::CancelRun {
                    run_id: claim.run.id.clone(),
                    reason: reason.to_string(),
                },
            )
            .await
        {
            tracing::warn!(session_id, error = %error, "Mako fenced execution cancellation failed");
        }
    }
}

fn cancellation_matches_claim(
    cancellation: &CommittedCancellation,
    claim: &ClaimedMakoRun,
) -> bool {
    claim.run.session_id.as_deref() == Some(cancellation.session_id.as_str())
}

async fn begin_cooperative_cancellation(shared: &RuntimeShared, claim: &ClaimedMakoRun) -> Instant {
    // This explicit budget is validated as non-zero and shorter than the
    // worker lease, so forced terminalization still owns a live exact fence.
    let grace = shared.config.cancellation_grace_period;
    let deadline = Instant::now() + grace;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if tokio::time::timeout(
        remaining,
        cancel_fenced_execution(shared, claim, "cancelled by user"),
    )
    .await
    .is_err()
    {
        tracing::warn!(
            run_id = %claim.run.id,
            timeout_ms = grace.as_millis(),
            "Mako execution host did not accept cancellation before the grace deadline"
        );
    }
    deadline
}

async fn finish_committed_cancellation(
    shared: &RuntimeShared,
    claim: &ClaimedMakoRun,
    fencing_token: u64,
    side_effects_may_be_uncertain: bool,
) -> Result<(), RuntimeStoreError> {
    let abort_delivery_confirmed = if side_effects_may_be_uncertain {
        abort_fenced_execution(shared, claim).await
    } else {
        true
    };
    let _finish_gate = shared.mutation_gate.lock().await;
    let now = Utc::now();
    let stop_reason = if side_effects_may_be_uncertain {
        FORCED_CANCELLATION_STOP_REASON
    } else {
        "cancelled before execution host start"
    };
    let error = side_effects_may_be_uncertain.then(|| FORCED_CANCELLATION_ERROR.to_string());
    let output = serde_json::json!({
        "kind": "cancelled",
        "forced": side_effects_may_be_uncertain,
        "side_effects_may_be_uncertain": side_effects_may_be_uncertain,
        "abort_delivery_confirmed": abort_delivery_confirmed,
    });
    let completion = RunCompletion {
        target_status: MakoRunStatus::Cancelled,
        now,
        available_at: None,
        wake_at: None,
        stop_reason: Some(stop_reason.to_string()),
        error: error.clone(),
        outcome: Some(output.clone()),
        trace_sequence_end: None,
    };
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let fence = daemon_fence(shared, fencing_token);
    let persisted = tokio::task::spawn_blocking(move || {
        MakoRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?)
            .finish_cancelled_claim_fenced(
                &run_id,
                &lease_token,
                fencing_token,
                &completion,
                &fence,
            )
            .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if persisted.is_none() {
        return Ok(());
    }
    let event = record_run_event(
        shared,
        &claim.run,
        "run_cancelled",
        serde_json::json!({
            "run_id": claim.run.id,
            "status": MakoRunStatus::Cancelled.as_str(),
            "stop_reason": stop_reason,
            "error": error,
            "outcome": output,
        }),
    )
    .await?;
    shared.events.publish(event.envelope());
    Ok(())
}

async fn abort_fenced_execution(shared: &RuntimeShared, claim: &ClaimedMakoRun) -> bool {
    let Some(session_id) = claim.run.session_id.as_deref() else {
        return false;
    };
    let timeout = shared
        .config
        .cancellation_grace_period
        .min(MAX_ABORT_DELIVERY_TIMEOUT);
    match tokio::time::timeout(
        timeout,
        shared.backend.control(
            session_id,
            super::backend::ExecutionControl::AbortRun {
                run_id: claim.run.id.clone(),
                reason: FORCED_CANCELLATION_STOP_REASON.to_string(),
            },
        ),
    )
    .await
    {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::error!(
                session_id,
                run_id = %claim.run.id,
                error = %error,
                "Mako exact execution abort delivery failed"
            );
            false
        }
        Err(_) => {
            tracing::error!(
                session_id,
                run_id = %claim.run.id,
                timeout_ms = timeout.as_millis(),
                "Mako exact execution abort delivery timed out"
            );
            false
        }
    }
}

async fn finish_execution(
    shared: &RuntimeShared,
    claim: ClaimedMakoRun,
    fencing_token: u64,
    mut outcome: ExecutionOutcome,
) -> Result<(), RuntimeStoreError> {
    let finish_gate = shared.mutation_gate.lock().await;
    // CancelSession commits under the same mutation gate before delivery to
    // the hosted loop. If that commit won the race, cancellation is
    // authoritative even when a slow or imperfect backend races back a
    // successful terminal result.
    if !matches!(outcome, ExecutionOutcome::Cancelled { .. })
        && cancellation_committed_for_claim(shared, &claim, fencing_token).await?
    {
        outcome = ExecutionOutcome::Cancelled {
            reason: "cancelled by user".into(),
        };
    }
    // A steer can be durably staged while the backend is crossing its final
    // model boundary. Even a successful channel send is not proof that the
    // loop consumed it before exiting. Do not commit a terminal state while
    // such input remains hidden: yield the run immediately, then the next
    // execution promotes the orphaned staging rows before loading history.
    if !matches!(outcome, ExecutionOutcome::Cancelled { .. })
        && has_pending_user_messages(shared, claim.run.session_id.as_deref()).await?
    {
        outcome = ExecutionOutcome::Sleeping {
            wake_at: Utc::now(),
            reason: Some("durable steering arrived after the terminal boundary".into()),
        };
    }
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
    drop(finish_gate);
    Ok(())
}

async fn cancellation_committed_for_claim(
    shared: &RuntimeShared,
    claim: &ClaimedMakoRun,
    fencing_token: u64,
) -> Result<bool, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        db.conn()
            .query_row(
                "SELECT EXISTS(
                     SELECT 1
                     FROM mako_runs r
                     JOIN mako_controllers c ON c.id = r.controller_id
                     WHERE r.id = ?1 AND r.lease_token = ?2 AND r.lease_epoch = ?3
                       AND r.status = 'running' AND c.status = 'disabled'
                 )",
                params![run_id, lease_token, fencing_token],
                |row| row.get::<_, bool>(0),
            )
            .map_err(RuntimeStoreError::from)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
}

async fn has_pending_user_messages(
    shared: &RuntimeShared,
    session_id: Option<&str>,
) -> Result<bool, RuntimeStoreError> {
    let Some(session_id) = session_id else {
        return Ok(false);
    };
    let path = shared.config.database_path.clone();
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        db.conn()
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM messages
                     WHERE session_id = ?1 AND role LIKE 'pending_user:%'
                 )",
                [&session_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(RuntimeStoreError::from)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
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
            Some(safe_outcome_summary("succeeded", Some(&output))),
        ),
        ExecutionOutcome::Failed {
            error: _,
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
            if !valid_retry_policy(policy) {
                return (
                    MakoRunStatus::DeadLetter,
                    None,
                    None,
                    Some("invalid_retry_policy".into()),
                    Some("execution failed and its retry policy was unsafe".into()),
                    Some(safe_outcome_summary("failed", None)),
                );
            }
            let retry_after = retry_after
                .map(|delay| delay.min(std::time::Duration::from_secs(MAX_RETRY_DELAY_SECS)));
            let retry_at = next_retry_at(
                now,
                policy,
                claim.attempt_no,
                deterministic_jitter(&claim.run.id, claim.attempt_no),
                retry_after,
            );
            match retry_at {
                Some(retry_at) => (
                    MakoRunStatus::RetryWait,
                    Some(retry_at),
                    None,
                    Some("transient_failure".into()),
                    Some("transient execution failure".into()),
                    Some(safe_outcome_summary("retry_scheduled", None)),
                ),
                None => (
                    MakoRunStatus::DeadLetter,
                    None,
                    None,
                    Some("retry_schedule_unavailable".into()),
                    Some("execution failed and no safe retry instant was available".into()),
                    Some(safe_outcome_summary("failed", None)),
                ),
            }
        }
        ExecutionOutcome::Failed { .. } => (
            MakoRunStatus::Failed,
            None,
            None,
            Some("failed".into()),
            Some("execution failed".into()),
            Some(safe_outcome_summary("failed", None)),
        ),
        ExecutionOutcome::Sleeping { wake_at, reason } => (
            MakoRunStatus::Sleeping,
            None,
            Some(wake_at),
            reason.map(|_| "execution requested sleep".into()),
            None,
            Some(safe_outcome_summary("sleeping", None)),
        ),
        ExecutionOutcome::AwaitingInput { details } => (
            MakoRunStatus::AwaitingInput,
            None,
            None,
            Some("awaiting_input".into()),
            None,
            Some(safe_outcome_summary("awaiting_input", Some(&details))),
        ),
        ExecutionOutcome::RecoveryRequired { reason: _ } => (
            MakoRunStatus::RecoveryRequired,
            None,
            None,
            Some("recovery_required".into()),
            Some("execution requires operator recovery".into()),
            Some(safe_outcome_summary("recovery_required", None)),
        ),
        ExecutionOutcome::Cancelled { reason: _ } => (
            MakoRunStatus::Cancelled,
            None,
            None,
            Some("execution cancelled".into()),
            None,
            Some(safe_outcome_summary("cancelled", None)),
        ),
    }
}

fn valid_retry_policy(policy: RetryPolicy) -> bool {
    policy.max_attempts > 0
        && policy.max_attempts <= MAX_RETRY_ATTEMPTS
        && policy.base_delay_secs > 0
        && policy.max_delay_secs >= policy.base_delay_secs
        && policy.max_delay_secs <= MAX_RETRY_DELAY_SECS
}

fn safe_outcome_summary(kind: &str, value: Option<&Value>) -> Value {
    serde_json::json!({
        "kind": kind,
        "payload_kind": value.map(json_value_kind),
    })
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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

#[cfg(test)]
mod completion_tests {
    use chrono::{DateTime, Utc};
    use krusty_core::mako::{canonical_timestamp, MakoRunStatus, RetryPolicy};
    use krusty_core::storage::{ClaimedMakoRun, MakoRun, MakoRunKind};

    use super::{completion_for, ExecutionOutcome};

    fn claim(retry: RetryPolicy) -> ClaimedMakoRun {
        let now = canonical_timestamp(Utc::now());
        ClaimedMakoRun {
            run: MakoRun {
                id: "run-overflow".into(),
                controller_id: "controller-1".into(),
                session_id: Some("session-1".into()),
                schedule_id: None,
                occurrence_id: None,
                kind: MakoRunKind::Dispatch,
                objective: "test retry overflow".into(),
                config: serde_json::json!({"retry": retry}),
                status: MakoRunStatus::Running,
                priority: 0,
                concurrency_key: None,
                scheduled_for: None,
                available_at: now.clone(),
                wake_at: None,
                attempt_count: 1,
                max_attempts: retry.max_attempts,
                lease_owner: Some("worker".into()),
                lease_token: Some("lease".into()),
                lease_epoch: Some(1),
                lease_expires_at: Some(now.clone()),
                heartbeat_at: Some(now.clone()),
                last_stop_reason: None,
                last_error: None,
                outcome: None,
                created_at: now.clone(),
                started_at: Some(now.clone()),
                finished_at: None,
                updated_at: now,
            },
            attempt_id: "attempt-1".into(),
            attempt_no: 1,
            lease_token: "lease".into(),
        }
    }

    #[test]
    fn retry_timestamp_overflow_dead_letters_instead_of_immediate_retry() {
        let claim = claim(RetryPolicy::default());
        let (status, available_at, _, reason, error, _) = completion_for(
            &claim,
            ExecutionOutcome::Failed {
                error: "raw provider error".into(),
                retryable: true,
                retry_after: None,
            },
            DateTime::<Utc>::MAX_UTC,
        );
        assert_eq!(status, MakoRunStatus::DeadLetter);
        assert_eq!(available_at, None);
        assert_eq!(reason.as_deref(), Some("retry_schedule_unavailable"));
        assert!(!error.unwrap().contains("raw provider error"));
    }
}
