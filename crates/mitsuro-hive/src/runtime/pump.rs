use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use mitsuro_core::agent::materialize_due_worker_introduction_review_runs_fenced;
use mitsuro_core::hive::{
    canonical_timestamp, next_retry_at, occurrences_between, parse_timezone, resolve_misfires,
    HiveRunStatus, MisfireDispatch, RetryPolicy,
};
use mitsuro_core::storage::{
    hive_groups, load_worker_with_conn, ClaimRunRequest, ClaimedHiveRun, DaemonFence,
    DaemonLeaseAcquire, Database, HiveDaemonLeaseStore, HiveGroupStatus, HiveRun,
    HiveRunExecutionContextV1, HiveRunExecutionModeV1, HiveRunKind, HiveRunStore, HiveSchedule,
    HiveScheduleStore, HiveWorkerIntroductionStore, HiveWorkerStatus, OverlapPolicy, ReconciledRun,
    RunCompletion, WorkerConversationLane, WorkerRunOrigin,
    WORKER_CONVERSATION_STOP_REQUESTED_REASON,
};
use mitsuro_core::workflow::WorkflowManager;
use mitsuro_hive_protocol::{
    unix_time_millis, Actor, EventEnvelope, ExtensionEvent, GroupMessageCommand, HiveEvent,
    ProtocolVersion, ResponsePayload, RuntimeEvent,
};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::backend::{ExecutionEvent, ExecutionEventSink, ExecutionOutcome, ExecutionRequest};
use super::config::MAX_ABORT_DELIVERY_TIMEOUT;
use super::deliveries;
use super::groups;
use super::handler::{
    CommittedCancellation, CommittedCancellationKind, RuntimeShared, DAEMON_LEASE_NAME,
    MAX_RETRY_ATTEMPTS, MAX_RETRY_DELAY_SECS,
};
use super::persistence::{
    append_event, get_or_create_controller, require_owned_session, ControllerRecord,
    PersistedEvent, RuntimeStoreError,
};

const MAX_DUE_OCCURRENCES: usize = 1_000;
const MAX_DUE_WORKER_INTRODUCTION_REVIEWS_PER_TICK: usize = 4;
const MAX_DUE_WORKER_WORKFLOW_ROLLOVERS_PER_TICK: usize = 8;
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
                        tracing::warn!(error = %error, "Hive execution task panicked");
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
                            tracing::error!(error = ?error, "Hive lease reconciliation failed");
                            continue;
                        }
                        if let Err(error) = materialize_due_worker_workflow_rollovers(
                            &shared,
                            token,
                        )
                        .await
                        {
                            shared.health.set_scheduler_activated(false);
                            tracing::error!(
                                error = ?error,
                                "Hive Worker Workflow rollover recovery failed"
                            );
                            continue;
                        }
                        shared.health.set_scheduler_activated(true);
                        if let Err(error) = materialize_due_worker_introduction_reviews(
                            &shared,
                            token,
                        ).await {
                            tracing::warn!(
                                error = ?error,
                                "Hive Worker Introduction review materialization failed"
                            );
                        }
                        if let Err(error) = deliver_pending_control(&shared, token).await {
                            tracing::warn!(error = ?error, "Hive durable control delivery failed");
                        }
                        if let Err(error) = deliveries::deliver_worker_messages(&shared, token).await
                        {
                            tracing::warn!(error = ?error, "Hive worker-message delivery failed");
                        }
                        if let Err(error) = super::heartbeat::wake_always_on_workers(&shared, token).await
                        {
                            tracing::warn!(error = ?error, "Hive always-on heartbeat wake failed");
                        }
                        if let Err(error) = materialize_due_schedules(&shared, token).await {
                            tracing::warn!(error = ?error, "Hive schedule materialization failed");
                        }
                        if let Err(error) = promote_due_runs(&shared, token).await {
                            tracing::warn!(error = ?error, "Hive delayed-run promotion failed");
                        }
                        if let Err(error) = advance_group_turns(&shared, token).await {
                            tracing::warn!(error = ?error, "Hive group turn advancement failed");
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
                                    tracing::warn!(error = ?error, "Hive run claim failed");
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
                        tracing::warn!(error = %error, "Hive daemon lease maintenance failed");
                    }
                }
            }
        }
    }

    cancel_active_executions(
        &shared,
        &mut executions,
        &mut active_sessions,
        "Hive runtime shutting down",
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
                 FROM hive_control_outbox o
                 JOIN hive_runs r ON r.id = o.run_id
                 JOIN hive_controllers c ON c.id = o.controller_id
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
                    "UPDATE hive_control_outbox
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
                    "UPDATE hive_control_outbox
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
    // Drop every scheduler-owned execution future first. The concrete Hive
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
                    tracing::warn!(session_id, error = %error, "Hive stale execution cancellation failed");
                }
                Err(error) => {
                    tracing::warn!(error = %error, "Hive stale execution cancellation task failed");
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
             SELECT 1 FROM hive_daemon_leases
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
        let store = HiveDaemonLeaseStore::new(Database::new(&path)?);
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
        let store = HiveDaemonLeaseStore::new(Database::new(&path)?);
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
            HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?)
                .reconcile_expired_leases_fenced(Utc::now(), &fence)
                .map_err(RuntimeStoreError::Internal)?;
        if reconciliation.requeued_runs.is_empty()
            && reconciliation.recovery_required_runs.is_empty()
            && reconciliation.recovered_succeeded_runs.is_empty()
            && reconciliation.recovered_failed_runs.is_empty()
            && reconciliation.recovered_cancelled_runs.is_empty()
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
        for (reconciled, event_type, reason, target_status) in reconciliation
            .recovered_succeeded_runs
            .into_iter()
            .map(|reconciled| {
                (
                    reconciled,
                    "run_completed",
                    "committed Worker Introduction opening recovered after worker lease expiry",
                    "succeeded",
                )
            })
            .chain(reconciliation.requeued_runs.into_iter().map(|reconciled| {
                (
                    reconciled,
                    "run_lease_requeued",
                    "worker lease expired before execution; requeued",
                    "queued",
                )
            }))
            .chain(reconciliation.recovered_failed_runs.into_iter().map(|reconciled| {
                (
                    reconciled,
                    "run_failed",
                    "terminal Worker Introduction review failure recovered after worker lease expiry",
                    "failed",
                )
            }))
            .chain(
                reconciliation
                    .recovered_cancelled_runs
                    .into_iter()
                    .map(|reconciled| {
                        (
                            reconciled,
                            "run_cancelled",
                            "committed Worker conversation Stop recovered after worker lease expiry",
                            "cancelled",
                        )
                    }),
            )
            .chain(
                reconciliation
                    .recovery_required_runs
                    .into_iter()
                    .map(|reconciled| {
                        (
                            reconciled,
                            "recovery_required",
                            "worker lease expired; side effects may be uncertain",
                            "recovery_required",
                        )
                    }),
            )
        {
            let run_id = reconciled.run_id;
            let controller = tx.query_row(
                "SELECT c.id, c.session_id, c.status, c.timezone
                 FROM hive_controllers c JOIN hive_runs r ON r.controller_id = c.id
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
                    "status": target_status,
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

async fn materialize_due_worker_introduction_reviews(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let materialized = tokio::task::spawn_blocking(move || {
        materialize_due_worker_introduction_review_runs_fenced(
            &path,
            MAX_DUE_WORKER_INTRODUCTION_REVIEWS_PER_TICK,
            &fence,
        )
        .map_err(RuntimeStoreError::Internal)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    if !materialized.is_empty() {
        tracing::debug!(
            materialized_count = materialized.len(),
            "Materialized due Worker Introduction review runs"
        );
    }
    Ok(())
}

/// Recover the narrow crash gap after a committed Worker Workflow outcome
/// reaches `succeeded` but before the daemon can create its next bounded
/// attempt. The core facade re-reconciles provider usage under the current
/// daemon fence before it considers a rollover, so token exhaustion or an
/// uncertain outcome can never be bypassed by this periodic wake.
async fn materialize_due_worker_workflow_rollovers(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let events = tokio::task::spawn_blocking(move || {
        let manager = WorkflowManager::new(path.clone())
            .map_err(|error| RuntimeStoreError::Internal(anyhow::Error::new(error)))?;
        manager
            .materialize_due_worker_workflow_rollovers(
                &fence,
                MAX_DUE_WORKER_WORKFLOW_ROLLOVERS_PER_TICK,
                Utc::now(),
            )
            .map_err(|error| RuntimeStoreError::Internal(anyhow::Error::new(error)))?;

        // Event publication is deliberately recoverable independently of the
        // core transaction. If the process died after materialization, this
        // bounded scan projects the already-authoritative successor exactly
        // once on the next fenced tick.
        persist_missing_worker_workflow_rollover_events(
            &path,
            &fence,
            MAX_DUE_WORKER_WORKFLOW_ROLLOVERS_PER_TICK * 2,
        )
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    for event in events {
        shared.events.publish(event.envelope());
    }
    Ok(())
}

fn persist_missing_worker_workflow_rollover_events(
    path: &std::path::Path,
    fence: &DaemonFence,
    limit: usize,
) -> Result<Vec<PersistedEvent>, RuntimeStoreError> {
    let db = Database::new(path).map_err(RuntimeStoreError::Internal)?;
    let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
    let now = canonical_timestamp(Utc::now());
    if !daemon_fence_is_current(&tx, fence, &now)? {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let limit = i64::try_from(limit).map_err(|_| {
        RuntimeStoreError::Invalid("Worker Workflow rollover event limit overflow".into())
    })?;
    let rows = {
        let mut statement = tx.prepare(
            "SELECT run.id, run.status, controller.id, controller.session_id,
                    controller.status, controller.timezone
             FROM hive_runs run
             JOIN hive_controllers controller ON controller.id = run.controller_id
             WHERE run.kind = 'worker_workflow'
               AND run.governor_origin = 'workflow_rollover'
               AND NOT EXISTS (
                   SELECT 1 FROM hive_controller_events event
                   WHERE event.controller_id = run.controller_id
                     AND event.dedupe_key = 'worker-workflow-rollover:' || run.id
               )
             ORDER BY run.created_at, run.id
             LIMIT ?1",
        )?;
        let mapped = statement.query_map([limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                ControllerRecord {
                    id: row.get(2)?,
                    session_id: row.get(3)?,
                    status: row.get(4)?,
                    timezone: row.get(5)?,
                },
            ))
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut events = Vec::with_capacity(rows.len());
    for (run_id, status, controller) in rows {
        events.push(append_event(
            &tx,
            &controller,
            "worker_workflow_rollover_queued",
            Some(&run_id),
            None,
            Some(&format!("worker-workflow-rollover:{run_id}")),
            serde_json::json!({
                "run_id": run_id,
                "status": status,
                "kind": "worker_workflow",
            }),
            &now,
        )?);
    }
    tx.commit()?;
    Ok(events)
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
            HiveScheduleStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
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
    schedule: HiveSchedule,
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
            mitsuro_core::hive::parse_utc_timestamp(value)
                .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))
        })?;
    let after = schedule
        .last_scheduled_for
        .as_deref()
        .map(mitsuro_core::hive::parse_utc_timestamp)
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
    schedule: HiveSchedule,
    resolution: mitsuro_core::hive::MisfireResolution,
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
            "SELECT revision FROM hive_schedules WHERE id = ?1 AND status = 'enabled'",
            [&schedule.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if persisted_revision != Some(schedule.revision as i64) {
        tx.commit()?;
        return Ok(Vec::new());
    }
    let controller = tx.query_row(
        "SELECT id, session_id, status, timezone FROM hive_controllers WHERE id = ?1",
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
    let targets_worker = schedule
        .worker_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|id| !id.is_empty())
        && schedule
            .group_id
            .as_deref()
            .map(str::trim)
            .is_none_or(|id| id.is_empty());
    let targets_group = schedule
        .group_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|id| !id.is_empty());
    let schedule_identity_is_absent = schedule.model.is_none()
        && schedule.model_key.is_none()
        && schedule.model_catalog_revision.is_none();
    let invalid_config = if schedule
        .model
        .as_deref()
        .is_none_or(|model| model.trim().is_empty())
        && !(targets_worker && schedule_identity_is_absent)
    {
        Some("schedule has no frozen model")
    } else if schedule
        .model_key
        .as_ref()
        .is_some_and(|key| schedule.model.as_deref() != Some(key.model_id.as_str()))
        || (schedule.model_key.is_none() && schedule.model_catalog_revision.is_some())
    {
        Some("schedule has inconsistent frozen model identity")
    } else if (!targets_worker
        && !targets_group
        && schedule
            .project_dir
            .as_deref()
            .is_none_or(|path| path.trim().is_empty() || !std::path::Path::new(path).is_absolute()))
        || ((targets_worker || targets_group)
            && schedule.project_dir.as_deref().is_some_and(|path| {
                path.trim().is_empty() || !std::path::Path::new(path).is_absolute()
            }))
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
            "UPDATE hive_schedules SET status = 'paused', revision = revision + 1,
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
        "UPDATE hive_schedules SET last_scheduled_for = ?3, next_fire_at = ?4,
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

struct ScheduleLane {
    controller: ControllerRecord,
    permission_mode: String,
    worker_id: Option<String>,
    model: Option<String>,
    model_key: Option<mitsuro_core::ai::models::ModelKey>,
    model_catalog_revision: Option<String>,
    execution_context: Option<HiveRunExecutionContextV1>,
}

fn resolve_schedule_lane(
    tx: &Transaction<'_>,
    schedule: &HiveSchedule,
    fallback: &ControllerRecord,
    permission_mode: &str,
    now: &str,
) -> Result<Result<ScheduleLane, &'static str>, RuntimeStoreError> {
    let Some(worker_id) = schedule
        .worker_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(Ok(ScheduleLane {
            controller: fallback.clone(),
            permission_mode: permission_mode.to_string(),
            worker_id: None,
            model: schedule.model.clone(),
            model_key: schedule.model_key.clone(),
            model_catalog_revision: schedule.model_catalog_revision.clone(),
            execution_context: None,
        }));
    };
    let Some(worker) = load_worker_with_conn(tx, worker_id).map_err(RuntimeStoreError::Internal)?
    else {
        return Ok(Err("targeted Worker was not found"));
    };
    if worker.status == HiveWorkerStatus::Archived {
        return Ok(Err("targeted Worker is archived"));
    }
    if worker.status == HiveWorkerStatus::Paused {
        return Ok(Err("targeted Worker is paused"));
    }
    if HiveWorkerIntroductionStore::from_connection(tx)
        .get_by_worker(&worker.id)
        .map_err(RuntimeStoreError::Internal)?
        .is_some_and(|introduction| !introduction.status.allows_autonomy())
    {
        return Ok(Err(
            "targeted Worker has not completed or skipped its Introduction",
        ));
    }
    let Some(session_id) = worker
        .dm_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(Err("targeted Worker has no DM lane"));
    };
    let actor = Actor {
        user_id: worker.user_id.clone(),
        client_kind: "hive-scheduler".into(),
    };
    let session = require_owned_session(tx, &actor, session_id)?;
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
        return Ok(Err("targeted Worker controller belongs to another Worker"));
    }
    let Some(worker_model) = worker
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return Ok(Err("targeted Worker has no frozen model identity"));
    };
    let Some(worker_model_key) = worker.model_key.as_ref() else {
        return Ok(Err("targeted Worker has no exact provider model identity"));
    };
    if worker_model_key.model_id != worker_model {
        return Ok(Err("targeted Worker has an inconsistent model identity"));
    }
    let schedule_identity_is_absent = schedule.model.is_none()
        && schedule.model_key.is_none()
        && schedule.model_catalog_revision.is_none();
    if !schedule_identity_is_absent
        && (schedule.model.as_deref() != Some(worker_model)
            || schedule.model_key.as_ref() != worker.model_key.as_ref()
            || schedule.model_catalog_revision.as_deref()
                != worker.model_catalog_revision.as_deref())
    {
        return Ok(Err(
            "schedule model identity does not match targeted Worker",
        ));
    }
    let execution = super::worker_context::resolve_worker_conversation_execution_binding(
        tx,
        &session.id,
        &worker.id,
        worker.revision,
        WorkerConversationLane::DirectMessage,
    )
    .map_err(RuntimeStoreError::Internal)?;
    if schedule.project_dir.is_some() && schedule.project_dir != execution.project_dir {
        return Ok(Err(
            "schedule workspace does not match the targeted Worker's exact DM workspace",
        ));
    }
    let execution_context = execution.context;
    Ok(Ok(ScheduleLane {
        controller,
        permission_mode: worker.permission_mode.as_str().to_string(),
        worker_id: Some(worker.id),
        model: Some(worker_model.to_string()),
        model_key: worker.model_key,
        model_catalog_revision: worker.model_catalog_revision,
        execution_context: Some(execution_context),
    }))
}

fn materialize_dispatch(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    schedule: &HiveSchedule,
    permission_mode: &str,
    dispatch: MisfireDispatch,
    now: &str,
    events: &mut Vec<PersistedEvent>,
) -> Result<(), RuntimeStoreError> {
    if schedule
        .group_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|id| !id.is_empty())
    {
        return materialize_group_dispatch(tx, controller, schedule, dispatch, now, events);
    }

    let unfinished: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs WHERE schedule_id = ?1
         AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        [&schedule.id],
        |row| row.get(0),
    )?;
    let queued_waiting: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs WHERE schedule_id = ?1
         AND status IN ('queued', 'sleeping', 'retry_wait', 'awaiting_input')",
        [&schedule.id],
        |row| row.get(0),
    )?;
    let lane = match resolve_schedule_lane(tx, schedule, controller, permission_mode, now)? {
        Ok(lane) => lane,
        Err(skip_reason) => {
            materialize_occurrence(
                tx,
                controller,
                schedule,
                dispatch.scheduled_for,
                None,
                "skipped",
                Some(skip_reason),
                dispatch.coalesced_count as u32,
                now,
                events,
            )?;
            return Ok(());
        }
    };
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
        &lane.controller,
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
        let (working_dir, project_dir) =
            match lane.execution_context.as_ref().map(|value| &value.mode) {
                Some(HiveRunExecutionModeV1::WorkerConversationNeutral { .. }) => (None, None),
                Some(HiveRunExecutionModeV1::WorkerWorkspaceAttached {
                    working_dir,
                    project_dir,
                    ..
                }) => (Some(working_dir.clone()), project_dir.clone()),
                Some(HiveRunExecutionModeV1::WorkerGoal { .. }) => {
                    return Err(RuntimeStoreError::StateConflict(
                        "schedule materialization cannot reuse a Worker Goal execution context"
                            .into(),
                    ));
                }
                Some(HiveRunExecutionModeV1::WorkerGoalAcceptance { .. }) => {
                    return Err(RuntimeStoreError::StateConflict(
                        "schedule materialization cannot reuse a Worker Goal acceptance context"
                            .into(),
                    ));
                }
                None => (schedule.project_dir.clone(), schedule.project_dir.clone()),
            };
        let config_json = serde_json::to_string(&serde_json::json!({
            "working_dir": working_dir,
            "project_dir": project_dir,
            "model": lane.model,
            "model_key": lane.model_key,
            "model_catalog_revision": lane.model_catalog_revision,
            "permission_mode": lane.permission_mode,
            "crew_slug": schedule.crew_slug,
            "retry": schedule.retry,
            "worker_id": lane.worker_id,
        }))
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        let concurrency_key = (schedule.overlap_policy != OverlapPolicy::Allow)
            .then(|| format!("schedule:{}", schedule.id));
        let scheduled_for = canonical_timestamp(dispatch.scheduled_for);
        let governor_origin = lane
            .worker_id
            .as_ref()
            .map(|_| WorkerRunOrigin::Scheduled.as_str());
        let governor_lane_key = lane
            .execution_context
            .as_ref()
            .map(|context| context.lane().canonical_lane_key())
            .transpose()
            .map_err(RuntimeStoreError::Internal)?;
        let execution_context_json = lane
            .execution_context
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        tx.execute(
            "INSERT INTO hive_runs (
                id, controller_id, session_id, schedule_id, occurrence_id, kind,
                objective, config_json, status, priority, concurrency_key,
                scheduled_for, available_at, wake_at, attempt_count, max_attempts,
                lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
                last_stop_reason, last_error, outcome_json, created_at, started_at,
                finished_at, updated_at, worker_id, governor_origin,
                governor_lane_key, execution_context_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'scheduled', ?6, ?7, 'queued', ?8, ?9,
                       ?10, ?10, NULL, 0, ?11, NULL, NULL, NULL, NULL, NULL,
                       NULL, NULL, NULL, ?12, NULL, NULL, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(id) DO NOTHING",
            params![
                run_id,
                lane.controller.id,
                lane.controller.session_id,
                schedule.id,
                occurrence_id,
                schedule.objective,
                config_json,
                schedule.priority,
                concurrency_key,
                scheduled_for,
                schedule.retry.max_attempts,
                now,
                lane.worker_id,
                governor_origin,
                governor_lane_key,
                execution_context_json,
            ],
        )?;
        events.push(append_event(
            tx,
            &lane.controller,
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

fn materialize_group_dispatch(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    schedule: &HiveSchedule,
    dispatch: MisfireDispatch,
    now: &str,
    events: &mut Vec<PersistedEvent>,
) -> Result<(), RuntimeStoreError> {
    let group_id = schedule
        .group_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .expect("group-targeted schedule requires group_id");
    let Some(group) = hive_groups::load_group(tx, group_id).map_err(RuntimeStoreError::Internal)?
    else {
        materialize_occurrence(
            tx,
            controller,
            schedule,
            dispatch.scheduled_for,
            None,
            "skipped",
            Some("targeted group was not found"),
            dispatch.coalesced_count as u32,
            now,
            events,
        )?;
        return Ok(());
    };
    if group.status == HiveGroupStatus::Archived {
        materialize_occurrence(
            tx,
            controller,
            schedule,
            dispatch.scheduled_for,
            None,
            "skipped",
            Some("targeted group is archived"),
            dispatch.coalesced_count as u32,
            now,
            events,
        )?;
        return Ok(());
    }

    let unfinished_runs: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs WHERE schedule_id = ?1
         AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')",
        [&schedule.id],
        |row| row.get(0),
    )?;
    let unfinished_turns: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_group_turns WHERE group_id = ?1 AND status = 'running'",
        [&group.id],
        |row| row.get(0),
    )?;
    let unfinished = unfinished_runs + unfinished_turns;
    let queued_waiting: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs WHERE schedule_id = ?1
         AND status IN ('queued', 'sleeping', 'retry_wait', 'awaiting_input')",
        [&schedule.id],
        |row| row.get(0),
    )?;
    let (status, reason, should_queue) = match schedule.overlap_policy {
        OverlapPolicy::Allow => ("queued", None, true),
        OverlapPolicy::Skip if unfinished > 0 => {
            ("skipped", Some("overlap policy skipped occurrence"), false)
        }
        OverlapPolicy::QueueOne if queued_waiting > 0 || unfinished_turns > 0 => (
            "coalesced",
            Some("a queued occurrence already exists"),
            false,
        ),
        _ => ("queued", None, true),
    };
    if !should_queue {
        materialize_occurrence(
            tx,
            controller,
            schedule,
            dispatch.scheduled_for,
            None,
            status,
            reason,
            dispatch.coalesced_count as u32,
            now,
            events,
        )?;
        return Ok(());
    }

    let actor = Actor {
        user_id: group.user_id.clone(),
        client_kind: "hive-scheduler".into(),
    };
    let scheduled_for = canonical_timestamp(dispatch.scheduled_for);
    let mutation = groups::group_message(
        tx,
        &actor,
        now,
        GroupMessageCommand {
            group_id: group.id,
            message: schedule.objective.clone(),
            mentions_override: None,
        },
        &format!("schedule:{}:{scheduled_for}", schedule.id),
        WorkerRunOrigin::ScheduledGroup,
    )?;
    let turn_id = match &mutation.response {
        ResponsePayload::GroupTurn(turn) => turn.turn_id.clone(),
        other => {
            return Err(RuntimeStoreError::Internal(anyhow::anyhow!(
                "group schedule materialize returned {other:?}"
            )));
        }
    };
    tx.execute(
        "UPDATE hive_runs SET schedule_id = ?1 WHERE group_turn_id = ?2",
        params![schedule.id, turn_id],
    )?;
    events.extend(mutation.events);
    materialize_occurrence(
        tx,
        controller,
        schedule,
        dispatch.scheduled_for,
        Some(&turn_id),
        status,
        reason,
        dispatch.coalesced_count as u32,
        now,
        events,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize_occurrence(
    tx: &Transaction<'_>,
    controller: &ControllerRecord,
    schedule: &HiveSchedule,
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
        "INSERT INTO hive_schedule_occurrences (
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
    crate::legacy_identity::schedule_object_id(kind, schedule_id, scheduled_for.timestamp_micros())
}

async fn promote_due_runs(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let promoted = tokio::task::spawn_blocking(move || {
        let store = HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
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
             FROM hive_runs r JOIN hive_controllers c ON c.id = r.controller_id
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

/// Advance durable group turns each fenced tick:
/// - roundtable turns whose current speaker finished get their next speaker
///   dispatched (rotation is pre-encoded in the turn's speaker plan);
/// - member outcome summaries are refreshed onto the turn row;
/// - turns whose members all reached terminal states are finalized as
///   completed, partial, or failed — one member's provider failure never
///   cancels its siblings;
/// - cancelled turns get exact CancelRun controls delivered to still-running
///   member executions until none remain.
async fn advance_group_turns(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let (events, cancellations, room_updates) = tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &fence, &now)? {
            tx.commit()?;
            return Ok::<_, RuntimeStoreError>((Vec::new(), Vec::new(), 0usize));
        }
        let mut events: Vec<PersistedEvent> = Vec::new();
        let mut cancellations: Vec<(String, String)> = Vec::new();
        let mut room_updates = 0usize;

        // Defense in depth for archives written by an older client or a
        // mixed-version server: stop any still-running turn before the
        // cancelled-turn pass below, so roundtables cannot dispatch another
        // speaker and live runs retain the normal exact cancellation fence.
        let archived_group_owners = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT g.id, g.user_id
                 FROM hive_groups g
                 JOIN hive_group_turns t ON t.group_id = g.id
                 WHERE g.status = 'archived' AND t.status = 'running'",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (group_id, user_id) in archived_group_owners {
            let actor = Actor {
                user_id,
                client_kind: "hive-archive-reconciler".into(),
            };
            let mutation = groups::group_archive(&tx, &actor, &now, &group_id)?;
            events.extend(mutation.events);
            room_updates += 1;
        }

        // Cancelled turns first: durably cancel anything not yet executing
        // and collect exact cancel controls for live executions.
        let cancelled_turn_ids = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT t.id FROM hive_group_turns t
                 JOIN hive_runs r ON r.group_turn_id = t.id
                 WHERE t.status = 'cancelled'
                   AND r.status IN ('queued', 'leased', 'running', 'sleeping',
                                    'retry_wait', 'awaiting_input', 'recovery_required')",
            )?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for turn_id in cancelled_turn_ids {
            for run in groups::load_member_runs(&tx, &turn_id)? {
                match run.status.as_str() {
                    "queued" | "leased" | "sleeping" | "retry_wait" | "awaiting_input"
                    | "recovery_required" => {
                        tx.execute(
                            "UPDATE hive_runs
                             SET status = 'cancelled', lease_owner = NULL, lease_token = NULL,
                                 lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
                                 wake_at = NULL, last_stop_reason = 'group turn stopped by user',
                                 finished_at = ?2, updated_at = ?2
                             WHERE id = ?1 AND status = ?3",
                            params![run.id, now, run.status],
                        )?;
                        if run.status == "leased" {
                            if let Some(lease_token) = run.lease_token.as_deref() {
                                tx.execute(
                                    "UPDATE hive_run_attempts
                                     SET finished_at = ?4, outcome = 'cancelled',
                                         stop_reason = 'group turn stopped by user'
                                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                                       AND finished_at IS NULL",
                                    params![run.id, run.attempt_count, lease_token, now],
                                )?;
                            }
                        }
                        let controller =
                            super::persistence::require_controller(&tx, &run.session_id)?;
                        events.push(append_event(
                            &tx,
                            &controller,
                            "run_cancelled",
                            Some(&run.id),
                            None,
                            Some(&format!(
                                "transition:{}:{}:cancelled",
                                run.id, run.attempt_count
                            )),
                            serde_json::json!({
                                "run_id": run.id,
                                "reason": "group turn stopped by user"
                            }),
                            &now,
                        )?);
                    }
                    "running" => {
                        cancellations.push((run.session_id.clone(), run.id.clone()));
                    }
                    _ => {}
                }
            }
        }

        // Running turns: refresh outcomes, advance roundtables, finalize.
        let running_turn_ids = {
            let mut statement =
                tx.prepare("SELECT id FROM hive_group_turns WHERE status = 'running'")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for turn_id in running_turn_ids {
            let Some(turn) = mitsuro_core::storage::hive_groups::load_turn(&tx, &turn_id)
                .map_err(RuntimeStoreError::Internal)?
            else {
                continue;
            };
            let runs = groups::load_member_runs(&tx, &turn.id)?;
            let has_active = runs
                .iter()
                .any(|run| groups::NON_TERMINAL_RUN_STATUSES.contains(&run.status.as_str()));

            // Merge existing entries (dispatch failures without runs) with
            // fresh per-run summaries keyed by worker id.
            let mut outcomes = turn
                .member_outcomes
                .as_ref()
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for run in &runs {
                let key = run
                    .worker_id
                    .clone()
                    .unwrap_or_else(|| format!("run:{}", run.id));
                outcomes.insert(key, groups::member_run_outcome(run));
            }

            let mut next_index = turn.next_speaker_index as usize;
            if !has_active && next_index < turn.speaker_plan.len() {
                // The lane is idle and the plan continues: dispatch the next
                // dispatchable speaker in rotation order.
                let group = mitsuro_core::storage::hive_groups::load_group(&tx, &turn.group_id)
                    .map_err(RuntimeStoreError::Internal)?;
                let roster =
                    mitsuro_core::storage::hive_groups::load_member_workers(&tx, &turn.group_id)
                        .map_err(RuntimeStoreError::Internal)?;
                let excerpt = trigger_excerpt(&tx, &turn.trigger_message_id)?;
                let schedule_id = tx
                    .query_row(
                        "SELECT schedule_id FROM hive_runs
                         WHERE group_turn_id = ?1 AND schedule_id IS NOT NULL
                         ORDER BY created_at ASC, id ASC LIMIT 1",
                        [&turn.id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                let origin = if schedule_id.is_some() {
                    WorkerRunOrigin::ScheduledGroup
                } else {
                    WorkerRunOrigin::UserGroup
                };
                let mut dispatched_next = false;
                if let Some(group) = group {
                    while next_index < turn.speaker_plan.len() {
                        let worker_id = turn.speaker_plan[next_index].clone();
                        next_index += 1;
                        let worker = roster.iter().find(|worker| worker.id == worker_id);
                        match groups::dispatch_member_run(
                            &tx, &now, &group, &turn, worker, None, &excerpt, origin,
                        )? {
                            Ok((run_id, run_events)) => {
                                if let Some(schedule_id) = schedule_id.as_deref() {
                                    let changed = tx.execute(
                                        "UPDATE hive_runs SET schedule_id = ?2
                                         WHERE id = ?1 AND group_turn_id = ?3
                                           AND governor_origin = 'scheduled_group'",
                                        params![run_id, schedule_id, turn.id],
                                    )?;
                                    if changed != 1 {
                                        return Err(RuntimeStoreError::StateConflict(
                                            "scheduled group continuation lost its exact schedule binding"
                                                .into(),
                                        ));
                                    }
                                }
                                events.extend(run_events);
                                outcomes.insert(
                                    worker_id,
                                    serde_json::json!({
                                        "status": "dispatched",
                                        "run_id": run_id
                                    }),
                                );
                                dispatched_next = true;
                                break;
                            }
                            Err(reason) => {
                                outcomes.insert(
                                    worker_id,
                                    serde_json::json!({
                                        "status": "failed",
                                        "error": reason
                                    }),
                                );
                            }
                        }
                    }
                }
                mitsuro_core::storage::hive_groups::update_turn_progress_with_conn(
                    &tx,
                    &turn.id,
                    next_index as u32,
                    &now,
                )
                .map_err(RuntimeStoreError::Internal)?;
                let outcomes_value = Value::Object(outcomes);
                mitsuro_core::storage::hive_groups::update_turn_member_outcomes_with_conn(
                    &tx,
                    &turn.id,
                    &outcomes_value,
                    &now,
                )
                .map_err(RuntimeStoreError::Internal)?;
                if dispatched_next || next_index < turn.speaker_plan.len() {
                    continue;
                }
                // The remaining plan could not dispatch at all; fall through
                // to finalize against the merged outcomes.
                let status = groups::classify_turn_outcomes(&outcomes_value);
                finalize_group_turn(&tx, &turn, status, &outcomes_value, &now)?;
                room_updates += 1;
                continue;
            }

            if !has_active && next_index >= turn.speaker_plan.len() {
                let outcomes_value = Value::Object(outcomes);
                let status = groups::classify_turn_outcomes(&outcomes_value);
                finalize_group_turn(&tx, &turn, status, &outcomes_value, &now)?;
                room_updates += 1;
                continue;
            }

            let outcomes_value = Value::Object(outcomes);
            if turn.member_outcomes.as_ref() != Some(&outcomes_value) {
                mitsuro_core::storage::hive_groups::update_turn_member_outcomes_with_conn(
                    &tx,
                    &turn.id,
                    &outcomes_value,
                    &now,
                )
                .map_err(RuntimeStoreError::Internal)?;
            }
        }

        tx.commit()?;
        Ok((events, cancellations, room_updates))
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;

    for event in events {
        shared.events.publish(event.envelope());
    }
    if room_updates > 0 {
        tracing::debug!(room_updates, "Hive group turns finalized");
    }
    for (session_id, run_id) in cancellations {
        if let Err(error) = shared
            .backend
            .control(
                &session_id,
                super::backend::ExecutionControl::CancelRun {
                    run_id: run_id.clone(),
                    reason: "group turn stopped by user".into(),
                },
            )
            .await
        {
            tracing::warn!(
                session_id,
                run_id,
                error = %error,
                "Hive group member cancellation delivery failed; retrying next tick"
            );
        }
    }
    Ok(())
}

fn trigger_excerpt(
    conn: &rusqlite::Connection,
    trigger_message_id: &str,
) -> Result<String, RuntimeStoreError> {
    let content: Option<String> = conn
        .query_row(
            "SELECT content FROM hive_group_messages WHERE id = ?1",
            [trigger_message_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(content.unwrap_or_else(|| "(the triggering message is no longer available)".into()))
}

/// Finalize once and post a room-visible system summary for non-clean ends.
fn finalize_group_turn(
    tx: &Transaction<'_>,
    turn: &mitsuro_core::storage::HiveGroupTurn,
    status: mitsuro_core::storage::HiveGroupTurnStatus,
    outcomes: &Value,
    now: &str,
) -> Result<(), RuntimeStoreError> {
    use mitsuro_core::storage::HiveGroupTurnStatus as TurnStatus;
    let finalized = mitsuro_core::storage::hive_groups::finalize_turn_with_conn(
        tx,
        &turn.id,
        status,
        Some(outcomes),
        now,
    )
    .map_err(RuntimeStoreError::Internal)?;
    if !finalized || status == TurnStatus::Completed {
        return Ok(());
    }
    let (succeeded, total) = outcome_counts(outcomes);
    let summary = match status {
        TurnStatus::Partial => {
            format!("Turn finished with partial results: {succeeded} of {total} members succeeded.")
        }
        TurnStatus::Failed => "Turn failed: no member completed.".to_string(),
        TurnStatus::Cancelled => "Turn cancelled.".to_string(),
        TurnStatus::Completed | TurnStatus::Running => return Ok(()),
    };
    mitsuro_core::storage::hive_groups::append_message_with_conn(
        tx,
        &mitsuro_core::storage::NewHiveGroupMessage {
            turn_id: Some(turn.id.clone()),
            ..mitsuro_core::storage::NewHiveGroupMessage::system(&turn.group_id, summary)
        },
        now,
    )
    .map_err(RuntimeStoreError::Internal)?;
    Ok(())
}

fn outcome_counts(outcomes: &Value) -> (usize, usize) {
    let Some(entries) = outcomes.as_object() else {
        return (0, 0);
    };
    let succeeded = entries
        .values()
        .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("succeeded"))
        .count();
    (succeeded, entries.len())
}

async fn claim_next(
    shared: &RuntimeShared,
    fencing_token: u64,
) -> Result<Option<ClaimedHiveRun>, RuntimeStoreError> {
    let _gate = shared.mutation_gate.lock().await;
    let path = shared.config.database_path.clone();
    let executor_id = shared.instance_id.clone();
    let lease_duration = shared.config.worker_lease_duration;
    let global_concurrency_limit = shared.config.global_concurrency_limit;
    let fence = daemon_fence(shared, fencing_token);
    let claim = tokio::task::spawn_blocking(move || {
        let store = HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        store
            .claim_next_fenced(
                &ClaimRunRequest {
                    executor_id,
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
        reject_worker_goal_acceptance_execution(claim)?;
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

async fn execute_claim(shared: Arc<RuntimeShared>, claim: ClaimedHiveRun, fencing_token: u64) {
    if let Err(error) = execute_claim_inner(&shared, claim, fencing_token).await {
        tracing::error!(error = ?error, "Hive claimed run execution failed");
    }
}

async fn execute_claim_inner(
    shared: &RuntimeShared,
    claim: ClaimedHiveRun,
    fencing_token: u64,
) -> Result<(), RuntimeStoreError> {
    reject_worker_goal_acceptance_execution(&claim)?;
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
        let store = HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
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
    if claim.run.kind == HiveRunKind::GroupTurn
        && cancellation_committed_for_claim(shared, &claim, fencing_token).await?
    {
        return finish_committed_cancellation(shared, &claim, fencing_token, false).await;
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
                        && cancellation_signal_committed_for_claim(
                            shared,
                            &claim,
                            fencing_token,
                            &cancellation,
                        ).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Durable session or exact-run cancellation state is
                        // authoritative when a burst overruns this bounded
                        // optimization channel.
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
                let heartbeat_alive = heartbeat_run(shared, &claim, fencing_token).await?;
                if cancellation_deadline.is_none()
                    && claim.run.kind == HiveRunKind::GroupTurn
                    && cancellation_committed_for_claim(shared, &claim, fencing_token).await?
                {
                    cancellation_deadline = Some(
                        begin_cooperative_cancellation(shared, &claim).await
                    );
                    continue;
                }
                if !heartbeat_alive {
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

fn reject_worker_goal_acceptance_execution(
    claim: &ClaimedHiveRun,
) -> Result<(), RuntimeStoreError> {
    let acceptance_context = claim.run.execution_context.as_ref().is_some_and(|context| {
        matches!(
            &context.mode,
            HiveRunExecutionModeV1::WorkerGoalAcceptance { .. }
        )
    });
    if claim.run.kind == mitsuro_core::storage::HiveRunKind::WorkerWorkflowAcceptance
        || acceptance_context
    {
        return Err(RuntimeStoreError::StateConflict(
            "Worker Workflow acceptance is awaiting owner input and cannot be claimed or executed"
                .into(),
        ));
    }
    Ok(())
}

async fn heartbeat_run(
    shared: &RuntimeShared,
    claim: &ClaimedHiveRun,
    fencing_token: u64,
) -> Result<bool, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let duration = shared.config.worker_lease_duration;
    let fence = daemon_fence(shared, fencing_token);
    tokio::task::spawn_blocking(move || {
        let store = HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
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
    claim: &ClaimedHiveRun,
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
                 FROM hive_runs r JOIN hive_controllers c ON c.id = r.controller_id
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
        HiveEvent::Extension(ExtensionEvent {
            name: "agentic_event".into(),
            payload,
        })
    } else {
        HiveEvent::Runtime(RuntimeEvent {
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

async fn cancel_fenced_execution(shared: &RuntimeShared, claim: &ClaimedHiveRun, reason: &str) {
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
            tracing::warn!(session_id, error = %error, "Hive fenced execution cancellation failed");
        }
    }
}

fn cancellation_matches_claim(
    cancellation: &CommittedCancellation,
    claim: &ClaimedHiveRun,
) -> bool {
    if claim.run.session_id.as_deref() != Some(cancellation.session_id.as_str()) {
        return false;
    }
    match &cancellation.kind {
        CommittedCancellationKind::Session => true,
        CommittedCancellationKind::WorkerIntroduction { run_id } => claim.run.id == *run_id,
        CommittedCancellationKind::WorkerRun {
            worker_id, run_id, ..
        } => {
            claim.run.id == *run_id
                && (claim.run.worker_id.as_deref() == Some(worker_id.as_str())
                    || configured_worker_id(&claim.run.config) == Some(worker_id.as_str()))
        }
        CommittedCancellationKind::WorkerWorkflow {
            worker_id,
            goal_id,
            run_id,
            ..
        } => {
            claim.run.id == *run_id
                && claim.run.worker_id.as_deref() == Some(worker_id.as_str())
                && claim.run.workflow_goal_id.as_deref() == Some(goal_id.as_str())
                && claim.run.kind == HiveRunKind::WorkerWorkflow
        }
    }
}

fn configured_worker_id(config: &serde_json::Value) -> Option<&str> {
    config
        .get("worker_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            config
                .get("group")
                .and_then(|group| group.get("worker_id"))
                .and_then(serde_json::Value::as_str)
        })
}

async fn begin_cooperative_cancellation(shared: &RuntimeShared, claim: &ClaimedHiveRun) -> Instant {
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
            "Hive execution host did not accept cancellation before the grace deadline"
        );
    }
    deadline
}

async fn finish_committed_cancellation(
    shared: &RuntimeShared,
    claim: &ClaimedHiveRun,
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
        target_status: HiveRunStatus::Cancelled,
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
        let store = HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        let status = match store
            .finish_stopped_worker_conversation_claim_fenced(
                &run_id,
                &lease_token,
                fencing_token,
                &completion,
                &fence,
            )
            .map_err(RuntimeStoreError::Internal)?
        {
            Some(status) => Some(status),
            None => match store
                .finish_cancelled_group_turn_claim_fenced(
                    &run_id,
                    &lease_token,
                    fencing_token,
                    &completion,
                    &fence,
                )
                .map_err(RuntimeStoreError::Internal)?
            {
                Some(status) => Some(status),
                None => store
                    .finish_cancelled_claim_fenced(
                        &run_id,
                        &lease_token,
                        fencing_token,
                        &completion,
                        &fence,
                    )
                    .map_err(RuntimeStoreError::Internal)?,
            },
        };
        status
            .map(|_| store.get_run(&run_id).map_err(RuntimeStoreError::Internal))
            .transpose()
            .map(|run| run.flatten())
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    let Some(persisted) = persisted else {
        return Ok(());
    };
    let event_type = if persisted.status == HiveRunStatus::Succeeded {
        "run_succeeded"
    } else {
        "run_cancelled"
    };
    let event = record_run_event(
        shared,
        &claim.run,
        event_type,
        serde_json::json!({
            "run_id": claim.run.id,
            "status": persisted.status.as_str(),
            "stop_reason": persisted.last_stop_reason,
            "error": persisted.last_error,
            "outcome": persisted.outcome,
        }),
    )
    .await?;
    shared.events.publish(event.envelope());
    Ok(())
}

async fn abort_fenced_execution(shared: &RuntimeShared, claim: &ClaimedHiveRun) -> bool {
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
                "Hive exact execution abort delivery failed"
            );
            false
        }
        Err(_) => {
            tracing::error!(
                session_id,
                run_id = %claim.run.id,
                timeout_ms = timeout.as_millis(),
                "Hive exact execution abort delivery timed out"
            );
            false
        }
    }
}

async fn finish_execution(
    shared: &RuntimeShared,
    claim: ClaimedHiveRun,
    fencing_token: u64,
    mut outcome: ExecutionOutcome,
) -> Result<(), RuntimeStoreError> {
    let finish_gate = shared.mutation_gate.lock().await;
    // Session cancellation and an exact Introduction skip commit under the
    // same mutation gate before delivery to the hosted loop. If either commit
    // won the race, cancellation is authoritative even when a slow or
    // imperfect backend races back a successful terminal result.
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
        && claim.run.kind != HiveRunKind::WorkerWorkflow
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
        let store = HiveRunStore::new(Database::new(&path).map_err(RuntimeStoreError::Internal)?);
        let status = if target_status == HiveRunStatus::Cancelled {
            match store
                .finish_stopped_worker_conversation_claim_fenced(
                    &run_id,
                    &lease_token,
                    fencing_token,
                    &completion,
                    &fence,
                )
                .map_err(RuntimeStoreError::Internal)?
            {
                Some(status) => Some(status),
                None => store
                    .finish_claimed_fenced(
                        &run_id,
                        &lease_token,
                        fencing_token,
                        &completion,
                        &fence,
                    )
                    .map_err(RuntimeStoreError::Internal)?,
            }
        } else {
            store
                .finish_claimed_fenced(&run_id, &lease_token, fencing_token, &completion, &fence)
                .map_err(RuntimeStoreError::Internal)?
        };
        status
            .map(|_| store.get_run(&run_id).map_err(RuntimeStoreError::Internal))
            .transpose()
            .map(|run| run.flatten())
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))??;
    let Some(persisted) = persisted else {
        return Ok(());
    };
    let status = persisted.status;
    let event_type = match status {
        HiveRunStatus::Succeeded => "run_completed",
        HiveRunStatus::RetryWait => "run_retry_scheduled",
        HiveRunStatus::Sleeping => "run_sleeping",
        HiveRunStatus::AwaitingInput => "run_awaiting_input",
        HiveRunStatus::RecoveryRequired => "recovery_required",
        HiveRunStatus::Cancelled => "run_cancelled",
        HiveRunStatus::DeadLetter => "run_dead_lettered",
        _ => "run_failed",
    };
    let event = record_run_event(
        shared,
        &claim.run,
        event_type,
        serde_json::json!({
            "run_id": claim.run.id,
            "status": status.as_str(),
            "stop_reason": persisted.last_stop_reason,
            "error": persisted.last_error,
            "outcome": persisted.outcome,
        }),
    )
    .await?;
    shared.events.publish(event.envelope());
    if claim.run.kind == HiveRunKind::WorkerWorkflow
        && matches!(
            status,
            HiveRunStatus::Succeeded
                | HiveRunStatus::Failed
                | HiveRunStatus::Cancelled
                | HiveRunStatus::DeadLetter
                | HiveRunStatus::RecoveryRequired
        )
    {
        match reconcile_finished_worker_workflow(shared, &claim, status, fencing_token).await {
            Ok(events) => {
                for event in events {
                    shared.events.publish(event.envelope());
                }
            }
            Err(error) => {
                // The source run is already durably terminal. Leave its
                // deterministic rollover unmaterialized and let the bounded
                // fenced sweep adopt it on the next tick rather than guessing
                // across an uncertain accounting boundary.
                tracing::warn!(
                    run_id = %claim.run.id,
                    error = ?error,
                    "Hive Worker Workflow terminal reconciliation deferred"
                );
            }
        }
    }
    drop(finish_gate);
    Ok(())
}

async fn reconcile_finished_worker_workflow(
    shared: &RuntimeShared,
    claim: &ClaimedHiveRun,
    status: HiveRunStatus,
    fencing_token: u64,
) -> Result<Vec<PersistedEvent>, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let fence = daemon_fence(shared, fencing_token);
    let run_id = claim.run.id.clone();
    let worker_id = claim.run.worker_id.clone().ok_or_else(|| {
        RuntimeStoreError::StateConflict(
            "Worker Workflow terminal run lost its authoritative Worker binding".into(),
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let owner_user_id = {
            let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
            db.conn()
                .query_row(
                    "SELECT worker.user_id
                     FROM hive_runs run
                     JOIN hive_workers worker ON worker.id = run.worker_id
                     WHERE run.id = ?1 AND run.kind = 'worker_workflow'
                       AND run.worker_id = ?2",
                    params![run_id, worker_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .ok_or_else(|| {
                    RuntimeStoreError::StateConflict(
                        "Worker Workflow terminal ownership binding changed".into(),
                    )
                })?
        };
        let manager = WorkflowManager::new(path.clone())
            .map_err(|error| RuntimeStoreError::Internal(anyhow::Error::new(error)))?;
        let reconciliation = manager
            .reconcile_worker_workflow_run(&fence, &run_id, Utc::now())
            .map_err(|error| RuntimeStoreError::Internal(anyhow::Error::new(error)))?
            .ok_or_else(|| {
                RuntimeStoreError::StateConflict(
                    "Worker Workflow terminal run has no canonical Workflow linkage".into(),
                )
            })?;
        if reconciliation.run_id != run_id || reconciliation.run_status != status.as_str() {
            return Err(RuntimeStoreError::StateConflict(
                "Worker Workflow terminal status changed during reconciliation".into(),
            ));
        }

        let rollover_materialized = if status == HiveRunStatus::Succeeded
            && reconciliation.run_status == HiveRunStatus::Succeeded.as_str()
            && reconciliation.goal_status == "active"
            && !reconciliation.recovery_required
        {
            manager
                .finalize_worker_workflow_attempt(
                    &fence,
                    &worker_id,
                    owner_user_id.as_deref(),
                    &run_id,
                    &format!("worker-workflow-rollover:{run_id}"),
                    Utc::now(),
                )
                .map_err(|error| RuntimeStoreError::Internal(anyhow::Error::new(error)))?
                .is_some()
        } else {
            false
        };

        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(Utc::now());
        if !daemon_fence_is_current(&tx, &fence, &now)? {
            tx.commit()?;
            return Err(RuntimeStoreError::StateConflict(
                "Hive daemon generation changed during Worker Workflow reconciliation".into(),
            ));
        }
        let controller = tx.query_row(
            "SELECT controller.id, controller.session_id, controller.status,
                    controller.timezone
             FROM hive_controllers controller
             JOIN hive_runs run ON run.controller_id = controller.id
             WHERE run.id = ?1 AND run.worker_id = ?2
               AND run.kind = 'worker_workflow'",
            params![run_id, worker_id],
            |row| {
                Ok(ControllerRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    status: row.get(2)?,
                    timezone: row.get(3)?,
                })
            },
        )?;
        let event = append_event(
            &tx,
            &controller,
            "worker_workflow_reconciled",
            Some(&run_id),
            None,
            Some(&format!(
                "worker-workflow-reconciled:{run_id}:{}",
                status.as_str()
            )),
            serde_json::json!({
                "run_id": run_id,
                "status": status.as_str(),
                "total_tokens": reconciliation.tokens_used,
                "rollover_count": if rollover_materialized { 1 } else { 0 },
            }),
            &now,
        )?;
        tx.commit()?;
        let mut events = vec![event];
        if rollover_materialized {
            events.extend(persist_missing_worker_workflow_rollover_events(
                &path, &fence, 2,
            )?);
        }
        Ok(events)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
}

async fn cancellation_committed_for_claim(
    shared: &RuntimeShared,
    claim: &ClaimedHiveRun,
    fencing_token: u64,
) -> Result<bool, RuntimeStoreError> {
    let path = shared.config.database_path.clone();
    let run_id = claim.run.id.clone();
    let lease_token = claim.lease_token.clone();
    let session_id = claim.run.session_id.clone();
    tokio::task::spawn_blocking(move || {
        let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
        db.conn()
            .query_row(
                "SELECT EXISTS(
                     SELECT 1
                     FROM hive_runs r
                     JOIN hive_controllers c ON c.id = r.controller_id
                     WHERE r.id = ?1 AND r.lease_token = ?2 AND r.lease_epoch = ?3
                       AND r.status = 'running' AND c.status = 'disabled'
                 ) OR EXISTS(
                     SELECT 1
                     FROM hive_runs r
                     JOIN hive_worker_introductions introduction
                       ON introduction.run_id = r.id
                      AND introduction.worker_id = r.worker_id
                     WHERE r.id = ?1 AND r.status = 'cancelled'
                       AND r.kind = 'worker_introduction'
                       AND introduction.status = 'skipped'
                 ) OR EXISTS(
                     SELECT 1
                     FROM hive_runs r
                     JOIN hive_controllers c ON c.id = r.controller_id
                     JOIN hive_workers worker ON worker.id = r.worker_id
                     WHERE r.id = ?1 AND r.session_id = ?4
                       AND r.lease_token = ?2 AND r.lease_epoch = ?3
                       AND r.status = 'running'
                       AND r.kind = 'worker_conversation'
                       AND r.governor_origin = 'user_dm'
                       AND r.governor_lane_key = 'dm'
                       AND json_extract(r.execution_context_json, '$.mode.kind')
                           IN ('worker_conversation_neutral', 'worker_workspace_attached')
                       AND json_extract(r.execution_context_json, '$.mode.lane.kind')
                           = 'direct_message'
                       AND json_extract(r.execution_context_json, '$.mode.worker_id')
                           = r.worker_id
                       AND json_extract(r.execution_context_json, '$.mode.worker_revision')
                           = worker.revision
                       AND r.last_stop_reason = ?5
                       AND c.worker_id = worker.id
                       AND worker.dm_session_id = r.session_id
                 ) OR EXISTS(
                     SELECT 1
                     FROM hive_runs r
                     JOIN hive_group_turns turn ON turn.id = r.group_turn_id
                     JOIN hive_groups group_row ON group_row.id = turn.group_id
                     WHERE r.id = ?1 AND r.lease_token = ?2 AND r.lease_epoch = ?3
                       AND r.status = 'running'
                       AND turn.group_id = r.group_id
                       AND (turn.status = 'cancelled' OR group_row.status = 'archived')
                 )",
                params![
                    run_id,
                    lease_token,
                    fencing_token,
                    session_id,
                    WORKER_CONVERSATION_STOP_REQUESTED_REASON
                ],
                |row| row.get::<_, bool>(0),
            )
            .map_err(RuntimeStoreError::from)
    })
    .await
    .map_err(|error| RuntimeStoreError::Internal(error.into()))?
}

async fn cancellation_signal_committed_for_claim(
    shared: &RuntimeShared,
    claim: &ClaimedHiveRun,
    fencing_token: u64,
    cancellation: &CommittedCancellation,
) -> Result<bool, RuntimeStoreError> {
    match &cancellation.kind {
        CommittedCancellationKind::Session => {
            cancellation_committed_for_claim(shared, claim, fencing_token).await
        }
        CommittedCancellationKind::WorkerIntroduction { run_id } => {
            let path = shared.config.database_path.clone();
            let run_id = run_id.clone();
            let session_id = cancellation.session_id.clone();
            tokio::task::spawn_blocking(move || {
                let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
                db.conn()
                    .query_row(
                        "SELECT EXISTS(
                             SELECT 1
                             FROM hive_runs r
                             JOIN hive_worker_introductions introduction
                               ON introduction.run_id = r.id
                              AND introduction.worker_id = r.worker_id
                             WHERE r.id = ?1 AND r.session_id = ?2
                               AND r.status = 'cancelled'
                               AND r.kind = 'worker_introduction'
                               AND introduction.status = 'skipped'
                         )",
                        params![run_id, session_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(RuntimeStoreError::from)
            })
            .await
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?
        }
        CommittedCancellationKind::WorkerRun { worker_id, run_id } => {
            let path = shared.config.database_path.clone();
            let worker_id = worker_id.clone();
            let run_id = run_id.clone();
            let session_id = cancellation.session_id.clone();
            tokio::task::spawn_blocking(move || {
                let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
                db.conn()
                    .query_row(
                        "SELECT EXISTS(
                             SELECT 1
                             FROM hive_runs run
                             JOIN hive_controllers controller ON controller.id = run.controller_id
                             JOIN hive_workers worker ON worker.id = run.worker_id
                             WHERE run.id = ?1 AND run.session_id = ?2
                               AND run.worker_id = ?3
                               AND controller.worker_id = worker.id
                               AND (
                                 (
                                   run.status = 'recovery_required'
                                   AND controller.status IN ('paused', 'disabled')
                                   AND worker.status IN ('paused', 'archived')
                                 ) OR (
                                   run.status = 'running'
                                   AND run.kind = 'worker_conversation'
                                   AND run.governor_origin = 'user_dm'
                                   AND run.governor_lane_key = 'dm'
                                   AND json_extract(run.execution_context_json, '$.mode.kind')
                                       IN (
                                           'worker_conversation_neutral',
                                           'worker_workspace_attached'
                                       )
                                   AND json_extract(run.execution_context_json, '$.mode.lane.kind')
                                       = 'direct_message'
                                   AND json_extract(run.execution_context_json, '$.mode.worker_id')
                                       = run.worker_id
                                   AND json_extract(run.execution_context_json, '$.mode.worker_revision')
                                       = worker.revision
                                   AND run.last_stop_reason = ?4
                                   AND worker.dm_session_id = run.session_id
                                 )
                               )
                         )",
                        params![
                            run_id,
                            session_id,
                            worker_id,
                            WORKER_CONVERSATION_STOP_REQUESTED_REASON
                        ],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(RuntimeStoreError::from)
            })
            .await
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?
        }
        CommittedCancellationKind::WorkerWorkflow {
            worker_id,
            goal_id,
            run_id,
        } => {
            let path = shared.config.database_path.clone();
            let worker_id = worker_id.clone();
            let goal_id = goal_id.clone();
            let run_id = run_id.clone();
            let session_id = cancellation.session_id.clone();
            tokio::task::spawn_blocking(move || {
                let db = Database::new(&path).map_err(RuntimeStoreError::Internal)?;
                db.conn()
                    .query_row(
                        "SELECT EXISTS(
                             SELECT 1
                             FROM hive_runs run
                             JOIN workflow_goals goal ON goal.id = run.workflow_goal_id
                             JOIN hive_workers worker ON worker.id = run.worker_id
                             WHERE run.id = ?1 AND run.session_id = ?2
                               AND run.worker_id = ?3 AND run.workflow_goal_id = ?4
                               AND run.kind = 'worker_workflow'
                               AND run.status = 'cancelled'
                               AND goal.session_id = run.session_id
                               AND goal.status IN ('paused', 'cancelled')
                               AND worker.dm_session_id = run.session_id
                         )",
                        params![run_id, session_id, worker_id, goal_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(RuntimeStoreError::from)
            })
            .await
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?
        }
    }
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
    claim: &ClaimedHiveRun,
    outcome: ExecutionOutcome,
    now: DateTime<Utc>,
) -> (
    HiveRunStatus,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<String>,
    Option<String>,
    Option<Value>,
) {
    match outcome {
        ExecutionOutcome::Succeeded { output } => (
            HiveRunStatus::Succeeded,
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
                    HiveRunStatus::DeadLetter,
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
                    HiveRunStatus::RetryWait,
                    Some(retry_at),
                    None,
                    Some("transient_failure".into()),
                    Some("transient execution failure".into()),
                    Some(safe_outcome_summary("retry_scheduled", None)),
                ),
                None => (
                    HiveRunStatus::DeadLetter,
                    None,
                    None,
                    Some("retry_schedule_unavailable".into()),
                    Some("execution failed and no safe retry instant was available".into()),
                    Some(safe_outcome_summary("failed", None)),
                ),
            }
        }
        ExecutionOutcome::Failed { .. } => (
            HiveRunStatus::Failed,
            None,
            None,
            Some("failed".into()),
            Some("execution failed".into()),
            Some(safe_outcome_summary("failed", None)),
        ),
        ExecutionOutcome::Sleeping { wake_at, reason } => (
            HiveRunStatus::Sleeping,
            None,
            Some(wake_at),
            reason.map(|_| "execution requested sleep".into()),
            None,
            Some(safe_outcome_summary("sleeping", None)),
        ),
        ExecutionOutcome::AwaitingInput { details } => (
            HiveRunStatus::AwaitingInput,
            None,
            None,
            Some("awaiting_input".into()),
            None,
            Some(safe_outcome_summary("awaiting_input", Some(&details))),
        ),
        ExecutionOutcome::RecoveryRequired { reason: _ } => (
            HiveRunStatus::RecoveryRequired,
            None,
            None,
            Some("recovery_required".into()),
            Some("execution requires operator recovery".into()),
            Some(safe_outcome_summary("recovery_required", None)),
        ),
        ExecutionOutcome::Cancelled { reason: _ } => (
            HiveRunStatus::Cancelled,
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
    run: &HiveRun,
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
            "SELECT id, session_id, status, timezone FROM hive_controllers WHERE id = ?1",
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
    use mitsuro_core::hive::{canonical_timestamp, HiveRunStatus, RetryPolicy};
    use mitsuro_core::storage::{ClaimedHiveRun, HiveRun, HiveRunKind};

    use super::{
        completion_for, reject_worker_goal_acceptance_execution, ExecutionOutcome,
        RuntimeStoreError,
    };

    fn claim(retry: RetryPolicy) -> ClaimedHiveRun {
        let now = canonical_timestamp(Utc::now());
        ClaimedHiveRun {
            run: HiveRun {
                id: "run-overflow".into(),
                controller_id: "controller-1".into(),
                session_id: Some("session-1".into()),
                schedule_id: None,
                occurrence_id: None,
                worker_id: None,
                objective_message_id: None,
                execution_context: None,
                conversation_through_message_id: None,
                response_message_id: None,
                response_provider_call_id: None,
                response_group_message_id: None,
                workflow_goal_id: None,
                workflow_attempt_id: None,
                governor: None,
                kind: HiveRunKind::Dispatch,
                objective: "test retry overflow".into(),
                config: serde_json::json!({"retry": retry}),
                status: HiveRunStatus::Running,
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
        assert_eq!(status, HiveRunStatus::DeadLetter);
        assert_eq!(available_at, None);
        assert_eq!(reason.as_deref(), Some("retry_schedule_unavailable"));
        assert!(!error.unwrap().contains("raw provider error"));
    }

    #[test]
    fn worker_goal_acceptance_is_rejected_before_running_or_backend_execution() {
        let mut claim = claim(RetryPolicy::default());
        claim.run.kind = HiveRunKind::WorkerWorkflowAcceptance;

        let error = reject_worker_goal_acceptance_execution(&claim)
            .expect_err("awaiting-input acceptance must be unclaimable");
        match error {
            RuntimeStoreError::StateConflict(message) => assert_eq!(
                message,
                "Worker Workflow acceptance is awaiting owner input and cannot be claimed or executed"
            ),
            other => panic!("unexpected acceptance rejection: {other:?}"),
        }
    }
}
