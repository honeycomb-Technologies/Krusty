use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::mako::{canonical_timestamp, normalize_timestamp, MakoRunStatus};
use crate::storage::Database;

use super::{
    ClaimRunRequest, ClaimedMakoRun, DaemonFence, LeaseReconciliation, MakoRun, MakoRunAttempt,
    MakoRunAttemptOutcome, MakoRunKind, RunCompletion,
};

const RUN_COLUMNS: &str = "id, controller_id, session_id, schedule_id, occurrence_id, kind, objective, config_json, status, priority, concurrency_key, scheduled_for, available_at, wake_at, attempt_count, max_attempts, lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at, last_stop_reason, last_error, outcome_json, created_at, started_at, finished_at, updated_at";
const ATTEMPT_COLUMNS: &str = "id, run_id, attempt_no, worker_id, lease_token, lease_epoch, started_at, finished_at, outcome, stop_reason, error, retry_at, trace_sequence_start, trace_sequence_end";

pub struct MakoRunStore {
    db: Database,
}

impl MakoRunStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn insert_run(&self, run: &MakoRun) -> Result<()> {
        anyhow::ensure!(
            run.status == MakoRunStatus::Queued,
            "new Mako runs must enter the queue"
        );
        anyhow::ensure!(!run.id.trim().is_empty(), "run id is empty");
        anyhow::ensure!(
            !run.controller_id.trim().is_empty(),
            "run controller id is empty"
        );
        anyhow::ensure!(!run.objective.trim().is_empty(), "run objective is empty");
        anyhow::ensure!(run.max_attempts > 0, "max_attempts is zero");
        anyhow::ensure!(run.attempt_count == 0, "new run has prior attempts");
        anyhow::ensure!(run.wake_at.is_none(), "new queued run cannot be sleeping");
        anyhow::ensure!(
            run.lease_owner.is_none()
                && run.lease_token.is_none()
                && run.lease_epoch.is_none()
                && run.lease_expires_at.is_none()
                && run.heartbeat_at.is_none(),
            "new queued run cannot hold a worker lease"
        );
        anyhow::ensure!(
            run.started_at.is_none() && run.finished_at.is_none(),
            "new queued run cannot already be started or finished"
        );
        anyhow::ensure!(
            run.last_stop_reason.is_none() && run.last_error.is_none() && run.outcome.is_none(),
            "new queued run cannot have a prior outcome"
        );
        anyhow::ensure!(
            run.concurrency_key
                .as_deref()
                .is_none_or(|key| !key.trim().is_empty()),
            "run concurrency key is empty"
        );
        let config_json = serde_json::to_string(&run.config)?;
        let outcome_json = run
            .outcome
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let scheduled_for = normalize_optional_timestamp(run.scheduled_for.as_deref())?;
        let available_at = normalize_timestamp(&run.available_at)?;
        let wake_at = normalize_optional_timestamp(run.wake_at.as_deref())?;
        let created_at = normalize_timestamp(&run.created_at)?;
        let updated_at = normalize_timestamp(&run.updated_at)?;
        self.db.conn().execute(
            "INSERT INTO mako_runs (
                id, controller_id, session_id, schedule_id, occurrence_id, kind,
                objective, config_json, status, priority, concurrency_key,
                scheduled_for, available_at, wake_at, attempt_count, max_attempts,
                lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
                last_stop_reason, last_error, outcome_json, created_at, started_at,
                finished_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                ?25, ?26, ?27, ?28
             )",
            params![
                run.id,
                run.controller_id,
                run.session_id,
                run.schedule_id,
                run.occurrence_id,
                run.kind.to_string(),
                run.objective,
                config_json,
                run.status.to_string(),
                run.priority,
                run.concurrency_key,
                scheduled_for,
                available_at,
                wake_at,
                run.attempt_count,
                run.max_attempts,
                run.lease_owner,
                run.lease_token,
                run.lease_epoch,
                run.lease_expires_at,
                run.heartbeat_at,
                run.last_stop_reason,
                run.last_error,
                outcome_json,
                created_at,
                run.started_at,
                run.finished_at,
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<MakoRun>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM mako_runs WHERE id = ?1");
        self.db
            .conn()
            .query_row(&sql, [id], map_run)
            .optional()
            .context("reading Mako run")
    }

    /// Atomically claims the next runnable item and opens its durable attempt row.
    pub fn claim_next(&self, request: &ClaimRunRequest) -> Result<Option<ClaimedMakoRun>> {
        self.claim_next_inner(request, None)
    }

    /// Claim only while the caller still owns the current scheduler generation.
    pub fn claim_next_fenced(
        &self,
        request: &ClaimRunRequest,
        daemon_fence: &DaemonFence,
    ) -> Result<Option<ClaimedMakoRun>> {
        anyhow::ensure!(
            request.lease_epoch == daemon_fence.fencing_token,
            "run lease epoch does not match daemon fence"
        );
        self.claim_next_inner(request, Some(daemon_fence))
    }

    fn claim_next_inner(
        &self,
        request: &ClaimRunRequest,
        daemon_fence: Option<&DaemonFence>,
    ) -> Result<Option<ClaimedMakoRun>> {
        anyhow::ensure!(!request.worker_id.trim().is_empty(), "worker id is empty");
        anyhow::ensure!(
            request.lease_epoch <= i64::MAX as u64,
            "lease epoch exceeds SQLite integer range"
        );
        if request.global_concurrency_limit == 0 || request.lease_duration.is_zero() {
            return Ok(None);
        }
        let now = canonical_timestamp(request.now);
        let lease_delta = Duration::from_std(request.lease_duration)
            .context("Mako worker lease duration exceeds chrono range")?;
        let lease_expires_at = canonical_timestamp(
            request
                .now
                .checked_add_signed(lease_delta)
                .context("Mako worker lease expiry overflow")?,
        );
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        if let Some(daemon_fence) = daemon_fence {
            if !daemon_fence_is_current(&tx, daemon_fence, &now)? {
                tx.commit()?;
                return Ok(None);
            }
        }
        let candidate_id = tx
            .query_row(
                "SELECT r.id
                 FROM mako_runs r
                 JOIN mako_controllers c ON c.id = r.controller_id
                 WHERE r.status = 'queued'
                   AND c.status = 'active'
                   AND r.available_at <= ?1
                   AND (SELECT COUNT(*) FROM mako_runs active
                        WHERE active.status IN ('leased', 'running')) < ?2
                   AND (SELECT COUNT(*) FROM mako_runs active
                        WHERE active.controller_id = r.controller_id
                          AND active.status IN ('leased', 'running')) < c.max_concurrent_runs
                   AND (
                       r.concurrency_key IS NULL OR NOT EXISTS (
                           SELECT 1 FROM mako_runs active
                           WHERE active.concurrency_key = r.concurrency_key
                             AND active.status IN ('leased', 'running')
                       )
                   )
                 ORDER BY r.priority DESC, r.available_at ASC, r.created_at ASC, r.id ASC
                 LIMIT 1",
                params![now, request.global_concurrency_limit],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(candidate_id) = candidate_id else {
            tx.commit()?;
            return Ok(None);
        };

        let lease_token = uuid::Uuid::new_v4().to_string();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let changed = tx.execute(
            "UPDATE mako_runs
             SET status = 'leased', lease_owner = ?2, lease_token = ?3,
                 lease_epoch = ?4, lease_expires_at = ?5, heartbeat_at = ?6,
                 attempt_count = attempt_count + 1, updated_at = ?6
             WHERE id = ?1 AND status = 'queued' AND available_at <= ?6",
            params![
                candidate_id,
                request.worker_id,
                lease_token,
                request.lease_epoch,
                lease_expires_at,
                now,
            ],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }

        let attempt_no = tx.query_row(
            "SELECT attempt_count FROM mako_runs WHERE id = ?1",
            [&candidate_id],
            |row| Ok(nonnegative_i64(row, 0)? as u32),
        )?;
        tx.execute(
            "INSERT INTO mako_run_attempts (
                id, run_id, attempt_no, worker_id, lease_token, lease_epoch,
                started_at, finished_at, outcome, stop_reason, error, retry_at,
                trace_sequence_start, trace_sequence_end
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 'leased', NULL, NULL, NULL, NULL, NULL)",
            params![
                attempt_id,
                candidate_id,
                attempt_no,
                request.worker_id,
                lease_token,
                request.lease_epoch,
                now,
            ],
        )?;
        let select = format!("SELECT {RUN_COLUMNS} FROM mako_runs WHERE id = ?1");
        let run = tx.query_row(&select, [&candidate_id], map_run)?;
        tx.commit()?;

        Ok(Some(ClaimedMakoRun {
            run,
            attempt_id,
            attempt_no,
            lease_token,
        }))
    }

    pub fn mark_running(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.mark_running_inner(run_id, lease_token, lease_epoch, now, None, None)
    }

    pub fn mark_running_fenced(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        now: DateTime<Utc>,
        daemon_fence: &DaemonFence,
    ) -> Result<bool> {
        self.mark_running_inner(
            run_id,
            lease_token,
            lease_epoch,
            now,
            None,
            Some(daemon_fence),
        )
    }

    /// Marks a lease as executing and anchors the attempt to the canonical trace stream.
    pub fn mark_running_with_trace(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        now: DateTime<Utc>,
        trace_sequence_start: Option<i64>,
    ) -> Result<bool> {
        self.mark_running_inner(
            run_id,
            lease_token,
            lease_epoch,
            now,
            trace_sequence_start,
            None,
        )
    }

    fn mark_running_inner(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        now: DateTime<Utc>,
        trace_sequence_start: Option<i64>,
        daemon_fence: Option<&DaemonFence>,
    ) -> Result<bool> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(now);
        if let Some(daemon_fence) = daemon_fence {
            if lease_epoch != daemon_fence.fencing_token
                || !daemon_fence_is_current(&tx, daemon_fence, &now)?
            {
                tx.commit()?;
                return Ok(false);
            }
        }
        let changed = tx.execute(
            "UPDATE mako_runs
             SET status = 'running', started_at = COALESCE(started_at, ?4),
                 heartbeat_at = ?4, updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status = 'leased' AND lease_expires_at > ?4",
            params![run_id, lease_token, lease_epoch, now],
        )?;
        if changed == 1 {
            let attempt_changed = tx.execute(
                "UPDATE mako_run_attempts
                 SET trace_sequence_start = ?4
                 WHERE run_id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
                   AND finished_at IS NULL",
                params![run_id, lease_token, lease_epoch, trace_sequence_start],
            )?;
            anyhow::ensure!(
                attempt_changed == 1,
                "leased Mako run has no matching open attempt"
            );
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn heartbeat(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        now: DateTime<Utc>,
        lease_duration: std::time::Duration,
    ) -> Result<bool> {
        anyhow::ensure!(!lease_duration.is_zero(), "lease duration is zero");
        let delta = Duration::from_std(lease_duration).context("lease duration is too large")?;
        let expires_at = now
            .checked_add_signed(delta)
            .context("lease expiry overflow")?;
        let now = canonical_timestamp(now);
        let expires_at = canonical_timestamp(expires_at);
        let changed = self.db.conn().execute(
            "UPDATE mako_runs
             SET heartbeat_at = ?4, lease_expires_at = ?5, updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status IN ('leased', 'running') AND lease_expires_at > ?4",
            params![run_id, lease_token, lease_epoch, now, expires_at],
        )?;
        Ok(changed == 1)
    }

    pub fn heartbeat_fenced(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        now: DateTime<Utc>,
        lease_duration: std::time::Duration,
        daemon_fence: &DaemonFence,
    ) -> Result<bool> {
        anyhow::ensure!(!lease_duration.is_zero(), "lease duration is zero");
        let delta = Duration::from_std(lease_duration).context("lease duration is too large")?;
        let expires_at = now
            .checked_add_signed(delta)
            .context("lease expiry overflow")?;
        let now = canonical_timestamp(now);
        let expires_at = canonical_timestamp(expires_at);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        if lease_epoch != daemon_fence.fencing_token
            || !daemon_fence_is_current(&tx, daemon_fence, &now)?
        {
            tx.commit()?;
            return Ok(false);
        }
        let changed = tx.execute(
            "UPDATE mako_runs
             SET heartbeat_at = ?4, lease_expires_at = ?5, updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status IN ('leased', 'running') AND lease_expires_at > ?4",
            params![run_id, lease_token, lease_epoch, now, expires_at],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Finish an attempt using its lease fence. A stale worker receives `None`.
    pub fn finish_claimed(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        completion: &RunCompletion,
    ) -> Result<Option<MakoRunStatus>> {
        self.finish_claimed_inner(run_id, lease_token, lease_epoch, completion, None)
    }

    pub fn finish_claimed_fenced(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        completion: &RunCompletion,
        daemon_fence: &DaemonFence,
    ) -> Result<Option<MakoRunStatus>> {
        self.finish_claimed_inner(
            run_id,
            lease_token,
            lease_epoch,
            completion,
            Some(daemon_fence),
        )
    }

    fn finish_claimed_inner(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        completion: &RunCompletion,
        daemon_fence: Option<&DaemonFence>,
    ) -> Result<Option<MakoRunStatus>> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(completion.now);
        if let Some(daemon_fence) = daemon_fence {
            if lease_epoch != daemon_fence.fencing_token
                || !daemon_fence_is_current(&tx, daemon_fence, &now)?
            {
                tx.commit()?;
                return Ok(None);
            }
        }
        let state = tx
            .query_row(
                "SELECT status, attempt_count, max_attempts
                 FROM mako_runs
                 WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
                   AND status IN ('leased', 'running') AND lease_expires_at > ?4",
                params![run_id, lease_token, lease_epoch, now],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        nonnegative_i64(row, 1)? as u32,
                        nonnegative_i64(row, 2)? as u32,
                    ))
                },
            )
            .optional()?;
        let Some((current_raw, attempt_no, max_attempts)) = state else {
            tx.commit()?;
            return Ok(None);
        };
        let current = MakoRunStatus::parse(&current_raw)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted run status: {current_raw}"))?;
        let target =
            if completion.target_status == MakoRunStatus::RetryWait && attempt_no >= max_attempts {
                MakoRunStatus::DeadLetter
            } else {
                completion.target_status
            };
        current.ensure_transition_to(target)?;
        anyhow::ensure!(
            target != MakoRunStatus::RetryWait || completion.available_at.is_some(),
            "retry_wait requires available_at"
        );
        anyhow::ensure!(
            target != MakoRunStatus::Sleeping || completion.wake_at.is_some(),
            "sleeping requires wake_at"
        );

        let available_at = completion.available_at.map(canonical_timestamp);
        let wake_at = completion.wake_at.map(canonical_timestamp);
        let retry_at = if target == MakoRunStatus::RetryWait {
            available_at.clone()
        } else {
            None
        };
        let outcome_json = completion
            .outcome
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let finished_at = target.is_terminal().then_some(now.as_str());
        let changed = tx.execute(
            "UPDATE mako_runs
             SET status = ?5, available_at = COALESCE(?6, available_at), wake_at = ?7,
                 lease_owner = NULL, lease_token = NULL, lease_epoch = NULL,
                 lease_expires_at = NULL, heartbeat_at = NULL,
                 last_stop_reason = ?8, last_error = ?9, outcome_json = ?10,
                 finished_at = COALESCE(?11, finished_at), updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status = ?12",
            params![
                run_id,
                lease_token,
                lease_epoch,
                now,
                target.to_string(),
                available_at,
                wake_at,
                completion.stop_reason,
                completion.error,
                outcome_json,
                finished_at,
                current.to_string(),
            ],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        tx.execute(
            "UPDATE mako_run_attempts
             SET finished_at = ?4, outcome = ?5, stop_reason = ?6, error = ?7,
                 retry_at = ?8, trace_sequence_end = ?9
             WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3",
            params![
                run_id,
                attempt_no,
                lease_token,
                now,
                attempt_outcome(target).as_str(),
                completion.stop_reason,
                completion.error,
                retry_at,
                completion.trace_sequence_end,
            ],
        )?;
        tx.commit()?;
        Ok(Some(target))
    }

    /// Reconcile expired leases without replaying uncertain mutating work.
    /// A run that never crossed the durable `running` boundary is safe to put
    /// back on the queue; a running attempt may have produced external side
    /// effects and therefore requires an explicit recovery decision.
    pub fn reconcile_expired_leases(&self, now: DateTime<Utc>) -> Result<LeaseReconciliation> {
        self.reconcile_expired_leases_inner(now, None)
    }

    pub fn reconcile_expired_leases_fenced(
        &self,
        now: DateTime<Utc>,
        daemon_fence: &DaemonFence,
    ) -> Result<LeaseReconciliation> {
        self.reconcile_expired_leases_inner(now, Some(daemon_fence))
    }

    fn reconcile_expired_leases_inner(
        &self,
        now: DateTime<Utc>,
        daemon_fence: Option<&DaemonFence>,
    ) -> Result<LeaseReconciliation> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let now = canonical_timestamp(now);
        if let Some(daemon_fence) = daemon_fence {
            if !daemon_fence_is_current(&tx, daemon_fence, &now)? {
                tx.commit()?;
                return Ok(LeaseReconciliation::default());
            }
        }
        let mut statement = tx.prepare(
            "SELECT id, status, attempt_count, lease_token
             FROM mako_runs
             WHERE status IN ('leased', 'running') AND lease_expires_at <= ?1
             ORDER BY lease_expires_at ASC",
        )?;
        let expired = statement
            .query_map([&now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    nonnegative_i64(row, 2)? as u32,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);

        let mut result = LeaseReconciliation::default();
        for (run_id, status, attempt_no, lease_token) in expired {
            let (target, message) = if status == MakoRunStatus::Leased.as_str() {
                result.requeued_unstarted += 1;
                result.requeued_run_ids.push(run_id.clone());
                (
                    MakoRunStatus::Queued,
                    "worker lease expired before execution; requeued",
                )
            } else {
                result.recovery_required += 1;
                result.recovery_required_run_ids.push(run_id.clone());
                (
                    MakoRunStatus::RecoveryRequired,
                    "worker lease expired; side effects may be uncertain",
                )
            };
            tx.execute(
                "UPDATE mako_runs
                 SET status = ?2,
                     lease_owner = NULL, lease_token = NULL, lease_epoch = NULL,
                     lease_expires_at = NULL, heartbeat_at = NULL,
                     last_error = ?4, updated_at = ?3
                 WHERE id = ?1 AND status = ?5",
                params![run_id, target.to_string(), now, message, status],
            )?;
            if let Some(lease_token) = lease_token {
                tx.execute(
                    "UPDATE mako_run_attempts
                     SET finished_at = ?4, outcome = 'abandoned', error = ?5
                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3",
                    params![run_id, attempt_no, lease_token, now, message],
                )?;
            }
        }
        tx.commit()?;
        Ok(result)
    }

    pub fn promote_due_runs(&self, now: DateTime<Utc>) -> Result<usize> {
        let now = canonical_timestamp(now);
        let changed = self.db.conn().execute(
            "UPDATE mako_runs
             SET status = 'queued', wake_at = NULL, updated_at = ?1
             WHERE (status = 'sleeping' AND wake_at IS NOT NULL AND wake_at <= ?1)
                OR (status = 'retry_wait' AND available_at <= ?1)",
            [&now],
        )?;
        Ok(changed)
    }

    pub fn promote_due_runs_fenced(
        &self,
        now: DateTime<Utc>,
        daemon_fence: &DaemonFence,
    ) -> Result<usize> {
        let now = canonical_timestamp(now);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        if !daemon_fence_is_current(&tx, daemon_fence, &now)? {
            tx.commit()?;
            return Ok(0);
        }
        let changed = tx.execute(
            "UPDATE mako_runs
             SET status = 'queued', wake_at = NULL, updated_at = ?1
             WHERE (status = 'sleeping' AND wake_at IS NOT NULL AND wake_at <= ?1)
                OR (status = 'retry_wait' AND available_at <= ?1)",
            [&now],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    /// Cancels queued, delayed, or active work and fences any worker that still holds its lease.
    pub fn cancel(&self, run_id: &str, now: DateTime<Utc>, reason: &str) -> Result<bool> {
        anyhow::ensure!(!reason.trim().is_empty(), "cancellation reason is empty");
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT status, attempt_count, lease_token
                 FROM mako_runs WHERE id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        nonnegative_i64(row, 1)? as u32,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((current, attempt_no, lease_token)) = current else {
            tx.commit()?;
            return Ok(false);
        };
        let current = MakoRunStatus::parse(&current)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted Mako run status"))?;
        current.ensure_transition_to(MakoRunStatus::Cancelled)?;
        if current == MakoRunStatus::Cancelled {
            tx.commit()?;
            return Ok(true);
        }

        let now = canonical_timestamp(now);
        let changed = tx.execute(
            "UPDATE mako_runs
             SET status = 'cancelled', lease_owner = NULL, lease_token = NULL,
                 lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
                 wake_at = NULL, last_stop_reason = ?3, last_error = NULL,
                 finished_at = ?2, updated_at = ?2
             WHERE id = ?1 AND status = ?4",
            params![run_id, now, reason, current.to_string()],
        )?;
        if changed == 1 {
            if let Some(lease_token) = lease_token {
                tx.execute(
                    "UPDATE mako_run_attempts
                     SET finished_at = ?4, outcome = 'cancelled', stop_reason = ?5
                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                       AND finished_at IS NULL",
                    params![run_id, attempt_no, lease_token, now, reason],
                )?;
            }
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn requeue(&self, run_id: &str, now: DateTime<Utc>) -> Result<bool> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT status FROM mako_runs WHERE id = ?1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(current) = current else {
            tx.commit()?;
            return Ok(false);
        };
        let current = MakoRunStatus::parse(&current)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted Mako run status"))?;
        current.ensure_transition_to(MakoRunStatus::Queued)?;
        let now = canonical_timestamp(now);
        let changed = tx.execute(
            "UPDATE mako_runs
             SET status = 'queued', available_at = ?2, wake_at = NULL,
                 finished_at = NULL, last_stop_reason = NULL, last_error = NULL,
                 outcome_json = NULL, updated_at = ?2
             WHERE id = ?1 AND status = ?3",
            params![run_id, now, current.to_string()],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn list_attempts(&self, run_id: &str) -> Result<Vec<MakoRunAttempt>> {
        let sql = format!(
            "SELECT {ATTEMPT_COLUMNS} FROM mako_run_attempts
             WHERE run_id = ?1 ORDER BY attempt_no ASC"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let attempts = statement
            .query_map([run_id], map_attempt)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Mako run attempts")?;
        Ok(attempts)
    }
}

fn attempt_outcome(status: MakoRunStatus) -> MakoRunAttemptOutcome {
    match status {
        MakoRunStatus::Succeeded => MakoRunAttemptOutcome::Succeeded,
        MakoRunStatus::Failed => MakoRunAttemptOutcome::Failed,
        MakoRunStatus::RetryWait => MakoRunAttemptOutcome::RetryScheduled,
        MakoRunStatus::Sleeping => MakoRunAttemptOutcome::Sleeping,
        MakoRunStatus::AwaitingInput => MakoRunAttemptOutcome::AwaitingInput,
        MakoRunStatus::RecoveryRequired => MakoRunAttemptOutcome::RecoveryRequired,
        MakoRunStatus::Cancelled => MakoRunAttemptOutcome::Cancelled,
        MakoRunStatus::DeadLetter => MakoRunAttemptOutcome::DeadLetter,
        MakoRunStatus::Queued | MakoRunStatus::Leased | MakoRunStatus::Running => {
            MakoRunAttemptOutcome::Abandoned
        }
    }
}

fn map_run(row: &Row<'_>) -> rusqlite::Result<MakoRun> {
    let kind = parse_required(5, row.get::<_, String>(5)?, MakoRunKind::parse)?;
    let config_json = row.get::<_, String>(7)?;
    let config = serde_json::from_str(&config_json)
        .map_err(|error| conversion_error(7, format!("invalid run config JSON: {error}")))?;
    let status = parse_required(8, row.get::<_, String>(8)?, MakoRunStatus::parse)?;
    let outcome_json = row.get::<_, Option<String>>(23)?;
    let outcome = outcome_json
        .map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| conversion_error(23, format!("invalid outcome JSON: {error}")))
        })
        .transpose()?;
    Ok(MakoRun {
        id: row.get(0)?,
        controller_id: row.get(1)?,
        session_id: row.get(2)?,
        schedule_id: row.get(3)?,
        occurrence_id: row.get(4)?,
        kind,
        objective: row.get(6)?,
        config,
        status,
        priority: row.get(9)?,
        concurrency_key: row.get(10)?,
        scheduled_for: row.get(11)?,
        available_at: row.get(12)?,
        wake_at: row.get(13)?,
        attempt_count: nonnegative_i64(row, 14)? as u32,
        max_attempts: nonnegative_i64(row, 15)? as u32,
        lease_owner: row.get(16)?,
        lease_token: row.get(17)?,
        lease_epoch: optional_nonnegative_i64(row, 18)?.map(|value| value as u64),
        lease_expires_at: row.get(19)?,
        heartbeat_at: row.get(20)?,
        last_stop_reason: row.get(21)?,
        last_error: row.get(22)?,
        outcome,
        created_at: row.get(24)?,
        started_at: row.get(25)?,
        finished_at: row.get(26)?,
        updated_at: row.get(27)?,
    })
}

fn map_attempt(row: &Row<'_>) -> rusqlite::Result<MakoRunAttempt> {
    let outcome = parse_required(8, row.get::<_, String>(8)?, MakoRunAttemptOutcome::parse)?;
    Ok(MakoRunAttempt {
        id: row.get(0)?,
        run_id: row.get(1)?,
        attempt_no: nonnegative_i64(row, 2)? as u32,
        worker_id: row.get(3)?,
        lease_token: row.get(4)?,
        lease_epoch: nonnegative_i64(row, 5)? as u64,
        started_at: row.get(6)?,
        finished_at: row.get(7)?,
        outcome,
        stop_reason: row.get(9)?,
        error: row.get(10)?,
        retry_at: row.get(11)?,
        trace_sequence_start: row.get(12)?,
        trace_sequence_end: row.get(13)?,
    })
}

fn daemon_fence_is_current(tx: &Transaction<'_>, fence: &DaemonFence, now: &str) -> Result<bool> {
    anyhow::ensure!(
        !fence.lease_name.trim().is_empty(),
        "daemon lease name is empty"
    );
    anyhow::ensure!(
        !fence.owner_id.trim().is_empty(),
        "daemon owner id is empty"
    );
    anyhow::ensure!(
        fence.fencing_token <= i64::MAX as u64,
        "daemon fencing token exceeds SQLite integer range"
    );
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

fn normalize_optional_timestamp(value: Option<&str>) -> Result<Option<String>> {
    value
        .map(normalize_timestamp)
        .transpose()
        .map_err(Into::into)
}

fn nonnegative_i64(row: &Row<'_>, index: usize) -> rusqlite::Result<i64> {
    let value = row.get::<_, i64>(index)?;
    if value < 0 {
        Err(conversion_error(index, "negative unsigned value"))
    } else {
        Ok(value)
    }
}

fn optional_nonnegative_i64(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<i64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            if value < 0 {
                Err(conversion_error(index, "negative unsigned value"))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn parse_required<T>(
    index: usize,
    value: String,
    parse: impl FnOnce(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    parse(&value).ok_or_else(|| conversion_error(index, format!("invalid enum value: {value}")))
}

fn conversion_error(index: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(IoError::new(ErrorKind::InvalidData, message.into())),
    )
}
