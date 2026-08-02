use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::ai::types::Content;
use crate::hive::{canonical_timestamp, normalize_timestamp, HiveRunStatus};
use crate::storage::{hash_request_bytes, Database};

use super::{
    ClaimRunRequest, ClaimedHiveRun, DaemonFence, HiveRun, HiveRunAttempt, HiveRunAttemptOutcome,
    HiveRunKind, LeaseReconciliation, ReconciledRun, RunCompletion,
};

const RUN_COLUMNS: &str = "id, controller_id, session_id, schedule_id, occurrence_id, kind, objective, config_json, status, priority, concurrency_key, scheduled_for, available_at, wake_at, attempt_count, max_attempts, lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at, last_stop_reason, last_error, outcome_json, created_at, started_at, finished_at, updated_at";
const ATTEMPT_COLUMNS: &str = "id, run_id, attempt_no, worker_id, lease_token, lease_epoch, started_at, finished_at, outcome, stop_reason, error, retry_at, trace_sequence_start, trace_sequence_end";

pub struct HiveRunStore {
    db: Database,
}

impl HiveRunStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn insert_run(&self, run: &HiveRun) -> Result<()> {
        anyhow::ensure!(
            run.status == HiveRunStatus::Queued,
            "new Hive runs must enter the queue"
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
            "INSERT INTO hive_runs (
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

    pub fn get_run(&self, id: &str) -> Result<Option<HiveRun>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM hive_runs WHERE id = ?1");
        self.db
            .conn()
            .query_row(&sql, [id], map_run)
            .optional()
            .context("reading Hive run")
    }

    /// Revalidate every identity boundary immediately before a claimed run is
    /// handed to an execution host (and again before live controls are
    /// delivered). The caller's `ClaimedHiveRun` is an immutable snapshot;
    /// comparing the persisted objective/configuration prevents a later
    /// session or schedule edit from silently changing work that was already
    /// claimed.
    pub fn validate_claimed_execution_fenced(
        &self,
        claim: &ClaimedHiveRun,
        daemon_fence: &DaemonFence,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let Some(session_id) = claim.run.session_id.as_deref() else {
            return Ok(false);
        };
        if claim.run.lease_owner.as_deref() != Some(daemon_fence.owner_id.as_str())
            || claim.run.lease_token.as_deref() != Some(claim.lease_token.as_str())
            || claim.run.lease_epoch != Some(daemon_fence.fencing_token)
        {
            return Ok(false);
        }

        let now = canonical_timestamp(now);
        let row = self
            .db
            .conn()
            .query_row(
                "SELECT r.objective, r.config_json, r.kind,
                        m.session_id, m.role, m.content
                 FROM hive_runs r
                 JOIN hive_controllers c ON c.id = r.controller_id
                 JOIN sessions s ON s.id = r.session_id
                 JOIN hive_run_attempts a ON a.id = ?7
                 JOIN hive_daemon_leases d ON d.lease_name = ?8
                 LEFT JOIN messages m ON m.id = r.objective_message_id
                 WHERE r.id = ?1 AND r.controller_id = ?2 AND r.session_id = ?3
                   AND r.status = 'running'
                   AND r.lease_owner = ?4 AND r.lease_token = ?5
                   AND r.lease_epoch = ?6 AND r.lease_expires_at > ?9
                   AND c.session_id = r.session_id AND c.status = 'active'
                   AND c.user_id IS s.user_id AND s.session_type = 'hive'
                   AND a.run_id = r.id AND a.attempt_no = ?10
                   AND a.worker_id = ?4 AND a.lease_token = ?5
                   AND a.lease_epoch = ?6 AND a.finished_at IS NULL
                   AND d.owner_id = ?4 AND d.fencing_token = ?6
                   AND d.expires_at > ?9",
                params![
                    claim.run.id,
                    claim.run.controller_id,
                    session_id,
                    daemon_fence.owner_id,
                    claim.lease_token,
                    daemon_fence.fencing_token,
                    claim.attempt_id,
                    daemon_fence.lease_name,
                    now,
                    claim.attempt_no,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()
            .context("validating claimed Hive execution fence")?;
        let Some((objective, config_json, kind, message_session, message_role, message_content)) =
            row
        else {
            return Ok(false);
        };
        if objective != claim.run.objective {
            return Ok(false);
        }
        let persisted_config: serde_json::Value = serde_json::from_str(&config_json)
            .context("decoding claimed Hive run config during fence validation")?;
        if persisted_config != claim.run.config {
            return Ok(false);
        }

        if matches!(kind.as_str(), "scheduled" | "controller_child") {
            let expected = format!("Hive {kind} objective:\n{}", objective.trim());
            let materialized = message_content
                .as_deref()
                .map(serde_json::from_str::<Vec<Content>>)
                .transpose()
                .context("decoding materialized Hive objective message")?;
            let objective_matches = matches!(
                materialized.as_deref(),
                Some([Content::Text { text }]) if text == &expected
            );
            if message_session.as_deref() != Some(session_id)
                || message_role.as_deref() != Some("user")
                || !objective_matches
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub fn list_for_controller(&self, controller_id: &str, limit: usize) -> Result<Vec<HiveRun>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM hive_runs
             WHERE controller_id = ?1
             ORDER BY created_at DESC, id ASC
             LIMIT ?2"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let rows = statement
            .query_map(params![controller_id, limit as i64], map_run)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive runs for controller")?;
        Ok(rows)
    }

    /// Atomically claims the next runnable item and opens its durable attempt row.
    pub fn claim_next(&self, request: &ClaimRunRequest) -> Result<Option<ClaimedHiveRun>> {
        self.claim_next_inner(request, None)
    }

    /// Claim only while the caller still owns the current scheduler generation.
    pub fn claim_next_fenced(
        &self,
        request: &ClaimRunRequest,
        daemon_fence: &DaemonFence,
    ) -> Result<Option<ClaimedHiveRun>> {
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
    ) -> Result<Option<ClaimedHiveRun>> {
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
            .context("Hive worker lease duration exceeds chrono range")?;
        let lease_expires_at = canonical_timestamp(
            request
                .now
                .checked_add_signed(lease_delta)
                .context("Hive worker lease expiry overflow")?,
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
                 FROM hive_runs r
                 JOIN hive_controllers c ON c.id = r.controller_id
                 WHERE r.status = 'queued'
                   AND c.status = 'active'
                   AND r.available_at <= ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM hive_runs uncertain
                       WHERE uncertain.controller_id = r.controller_id
                         AND uncertain.status = 'recovery_required'
                   )
                   AND (SELECT COUNT(*) FROM hive_runs active
                        WHERE active.status IN ('leased', 'running')) < ?2
                   AND (SELECT COUNT(*) FROM hive_runs active
                        WHERE active.controller_id = r.controller_id
                          AND active.status IN ('leased', 'running')) < c.max_concurrent_runs
                   AND (
                       r.concurrency_key IS NULL OR NOT EXISTS (
                           SELECT 1 FROM hive_runs active
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
            "UPDATE hive_runs
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
            "SELECT attempt_count FROM hive_runs WHERE id = ?1",
            [&candidate_id],
            |row| Ok(nonnegative_i64(row, 0)? as u32),
        )?;
        tx.execute(
            "INSERT INTO hive_run_attempts (
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
        let select = format!("SELECT {RUN_COLUMNS} FROM hive_runs WHERE id = ?1");
        let run = tx.query_row(&select, [&candidate_id], map_run)?;
        tx.commit()?;

        Ok(Some(ClaimedHiveRun {
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
            "UPDATE hive_runs
             SET status = 'running', started_at = COALESCE(started_at, ?4),
                 heartbeat_at = ?4, updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status = 'leased' AND lease_expires_at > ?4
               AND EXISTS (
                   SELECT 1 FROM hive_controllers c
                   WHERE c.id = hive_runs.controller_id AND c.status = 'active'
               )",
            params![run_id, lease_token, lease_epoch, now],
        )?;
        if changed == 1 {
            let attempt_changed = tx.execute(
                "UPDATE hive_run_attempts
                 SET trace_sequence_start = ?4
                 WHERE run_id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
                   AND finished_at IS NULL",
                params![run_id, lease_token, lease_epoch, trace_sequence_start],
            )?;
            anyhow::ensure!(
                attempt_changed == 1,
                "leased Hive run has no matching open attempt"
            );
            materialize_objective_message(&tx, run_id, &now)?;
            update_derived_state(&tx, run_id, HiveRunStatus::Running, &now)?;
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
            "UPDATE hive_runs
             SET heartbeat_at = ?4, lease_expires_at = ?5, updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status IN ('leased', 'running') AND lease_expires_at > ?4
               AND EXISTS (
                   SELECT 1 FROM hive_controllers c
                   WHERE c.id = hive_runs.controller_id AND c.status = 'active'
               )",
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
            "UPDATE hive_runs
             SET heartbeat_at = ?4, lease_expires_at = ?5, updated_at = ?4
             WHERE id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
               AND status IN ('leased', 'running') AND lease_expires_at > ?4
               AND EXISTS (
                   SELECT 1 FROM hive_controllers c
                   WHERE c.id = hive_runs.controller_id AND c.status = 'active'
               )",
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
    ) -> Result<Option<HiveRunStatus>> {
        self.finish_claimed_inner(run_id, lease_token, lease_epoch, completion, None, false)
    }

    pub fn finish_claimed_fenced(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        completion: &RunCompletion,
        daemon_fence: &DaemonFence,
    ) -> Result<Option<HiveRunStatus>> {
        self.finish_claimed_inner(
            run_id,
            lease_token,
            lease_epoch,
            completion,
            Some(daemon_fence),
            false,
        )
    }

    /// Authoritatively close a user-cancelled running claim after the
    /// execution host's cooperative grace period. This is deliberately
    /// stricter than a normal completion: the exact worker lease and current
    /// daemon generation must still own the open attempt, and the controller
    /// must already be disabled by a committed, ownership-checked
    /// `CancelSession` mutation.
    pub fn finish_cancelled_claim_fenced(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        completion: &RunCompletion,
        daemon_fence: &DaemonFence,
    ) -> Result<Option<HiveRunStatus>> {
        anyhow::ensure!(
            completion.target_status == HiveRunStatus::Cancelled,
            "forced cancellation completion must target cancelled"
        );
        self.finish_claimed_inner(
            run_id,
            lease_token,
            lease_epoch,
            completion,
            Some(daemon_fence),
            true,
        )
    }

    fn finish_claimed_inner(
        &self,
        run_id: &str,
        lease_token: &str,
        lease_epoch: u64,
        completion: &RunCompletion,
        daemon_fence: Option<&DaemonFence>,
        require_committed_cancellation: bool,
    ) -> Result<Option<HiveRunStatus>> {
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
        let daemon_owner = daemon_fence.map(|fence| fence.owner_id.as_str());
        let state = tx
            .query_row(
                "SELECT r.status, r.attempt_count, r.max_attempts
                 FROM hive_runs r
                 JOIN hive_controllers c ON c.id = r.controller_id
                 WHERE r.id = ?1 AND r.lease_token = ?2 AND r.lease_epoch = ?3
                   AND r.status IN ('leased', 'running') AND r.lease_expires_at > ?4
                   AND (?5 IS NULL OR r.lease_owner = ?5)
                   AND (?6 = 0 OR c.status = 'disabled')
                   AND EXISTS (
                       SELECT 1 FROM hive_run_attempts a
                       WHERE a.run_id = r.id AND a.attempt_no = r.attempt_count
                         AND a.lease_token = ?2 AND a.lease_epoch = ?3
                         AND a.finished_at IS NULL
                   )",
                params![
                    run_id,
                    lease_token,
                    lease_epoch,
                    now,
                    daemon_owner,
                    require_committed_cancellation,
                ],
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
        let current = HiveRunStatus::parse(&current_raw)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted run status: {current_raw}"))?;
        let target =
            if completion.target_status == HiveRunStatus::RetryWait && attempt_no >= max_attempts {
                HiveRunStatus::DeadLetter
            } else {
                completion.target_status
            };
        current.ensure_transition_to(target)?;
        anyhow::ensure!(
            target != HiveRunStatus::RetryWait || completion.available_at.is_some(),
            "retry_wait requires available_at"
        );
        anyhow::ensure!(
            target != HiveRunStatus::Sleeping || completion.wake_at.is_some(),
            "sleeping requires wake_at"
        );

        let available_at = completion.available_at.map(canonical_timestamp);
        let wake_at = completion.wake_at.map(canonical_timestamp);
        let retry_at = if target == HiveRunStatus::RetryWait {
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
            "UPDATE hive_runs
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
        let attempt_changed = tx.execute(
            "UPDATE hive_run_attempts
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
        anyhow::ensure!(
            attempt_changed == 1,
            "claimed Hive run has no matching open attempt during completion"
        );
        update_derived_state(&tx, run_id, target, &now)?;
        if matches!(
            target,
            HiveRunStatus::Succeeded
                | HiveRunStatus::Failed
                | HiveRunStatus::Cancelled
                | HiveRunStatus::DeadLetter
                | HiveRunStatus::RecoveryRequired
        ) {
            discard_pending_controls(&tx, run_id, "run finished before control delivery", &now)?;
        }
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
             FROM hive_runs
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
            let (target, message) = if status == HiveRunStatus::Leased.as_str() {
                result.requeued_unstarted += 1;
                result.requeued_runs.push(ReconciledRun {
                    run_id: run_id.clone(),
                    attempt_no,
                });
                (
                    HiveRunStatus::Queued,
                    "worker lease expired before execution; requeued",
                )
            } else {
                result.recovery_required += 1;
                result.recovery_required_runs.push(ReconciledRun {
                    run_id: run_id.clone(),
                    attempt_no,
                });
                (
                    HiveRunStatus::RecoveryRequired,
                    "worker lease expired; side effects may be uncertain",
                )
            };
            tx.execute(
                "UPDATE hive_runs
                 SET status = ?2,
                     lease_owner = NULL, lease_token = NULL, lease_epoch = NULL,
                     lease_expires_at = NULL, heartbeat_at = NULL,
                     last_error = ?4, updated_at = ?3
                 WHERE id = ?1 AND status = ?5",
                params![run_id, target.to_string(), now, message, status],
            )?;
            if let Some(lease_token) = lease_token {
                tx.execute(
                    "UPDATE hive_run_attempts
                     SET finished_at = ?4, outcome = 'abandoned', error = ?5
                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3",
                    params![run_id, attempt_no, lease_token, now, message],
                )?;
            }
            update_derived_state(&tx, &run_id, target, &now)?;
            if target == HiveRunStatus::RecoveryRequired {
                discard_pending_controls(
                    &tx,
                    &run_id,
                    "run entered recovery before control delivery",
                    &now,
                )?;
            }
        }
        tx.commit()?;
        Ok(result)
    }

    pub fn promote_due_runs(&self, now: DateTime<Utc>) -> Result<usize> {
        Ok(self.promote_due_runs_inner(now, None)?.len())
    }

    pub fn promote_due_runs_fenced(
        &self,
        now: DateTime<Utc>,
        daemon_fence: &DaemonFence,
    ) -> Result<Vec<ReconciledRun>> {
        self.promote_due_runs_inner(now, Some(daemon_fence))
    }

    fn promote_due_runs_inner(
        &self,
        now: DateTime<Utc>,
        daemon_fence: Option<&DaemonFence>,
    ) -> Result<Vec<ReconciledRun>> {
        let now = canonical_timestamp(now);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        if let Some(daemon_fence) = daemon_fence {
            if !daemon_fence_is_current(&tx, daemon_fence, &now)? {
                tx.commit()?;
                return Ok(Vec::new());
            }
        }
        let run_ids = {
            let mut statement = tx.prepare(
                "SELECT id, attempt_count FROM hive_runs
                 WHERE (status = 'sleeping' AND wake_at IS NOT NULL AND wake_at <= ?1)
                    OR (status = 'retry_wait' AND available_at <= ?1)
                 ORDER BY id",
            )?;
            let run_ids = statement
                .query_map([&now], |row| {
                    Ok(ReconciledRun {
                        run_id: row.get(0)?,
                        attempt_no: nonnegative_i64(row, 1)? as u32,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            run_ids
        };
        tx.execute(
            "UPDATE hive_runs
             SET status = 'queued', wake_at = NULL, updated_at = ?1
             WHERE (status = 'sleeping' AND wake_at IS NOT NULL AND wake_at <= ?1)
                OR (status = 'retry_wait' AND available_at <= ?1)",
            [&now],
        )?;
        for run in &run_ids {
            update_derived_state(&tx, &run.run_id, HiveRunStatus::Queued, &now)?;
        }
        tx.commit()?;
        Ok(run_ids)
    }

    /// Cancels queued, delayed, or active work and fences any worker that still holds its lease.
    pub fn cancel(&self, run_id: &str, now: DateTime<Utc>, reason: &str) -> Result<bool> {
        anyhow::ensure!(!reason.trim().is_empty(), "cancellation reason is empty");
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT status, attempt_count, lease_token
                 FROM hive_runs WHERE id = ?1",
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
        let current = HiveRunStatus::parse(&current)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted Hive run status"))?;
        current.ensure_transition_to(HiveRunStatus::Cancelled)?;
        if current == HiveRunStatus::Cancelled {
            tx.commit()?;
            return Ok(true);
        }

        let now = canonical_timestamp(now);
        let changed = tx.execute(
            "UPDATE hive_runs
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
                    "UPDATE hive_run_attempts
                     SET finished_at = ?4, outcome = 'cancelled', stop_reason = ?5
                     WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                       AND finished_at IS NULL",
                    params![run_id, attempt_no, lease_token, now, reason],
                )?;
            }
            update_derived_state(&tx, run_id, HiveRunStatus::Cancelled, &now)?;
            discard_pending_controls(&tx, run_id, "run cancelled", &now)?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn requeue(&self, run_id: &str, now: DateTime<Utc>) -> Result<bool> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT status FROM hive_runs WHERE id = ?1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(current) = current else {
            tx.commit()?;
            return Ok(false);
        };
        let current = HiveRunStatus::parse(&current)
            .ok_or_else(|| anyhow::anyhow!("invalid persisted Hive run status"))?;
        current.ensure_transition_to(HiveRunStatus::Queued)?;
        let now = canonical_timestamp(now);
        let changed = tx.execute(
            "UPDATE hive_runs
             SET status = 'queued', available_at = ?2, wake_at = NULL,
                 finished_at = NULL, last_stop_reason = NULL, last_error = NULL,
                 outcome_json = NULL, updated_at = ?2
             WHERE id = ?1 AND status = ?3",
            params![run_id, now, current.to_string()],
        )?;
        if changed == 1 {
            update_derived_state(&tx, run_id, HiveRunStatus::Queued, &now)?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn list_attempts(&self, run_id: &str) -> Result<Vec<HiveRunAttempt>> {
        let sql = format!(
            "SELECT {ATTEMPT_COLUMNS} FROM hive_run_attempts
             WHERE run_id = ?1 ORDER BY attempt_no ASC"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let attempts = statement
            .query_map([run_id], map_attempt)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive run attempts")?;
        Ok(attempts)
    }
}

/// Keep client-facing occurrence/runtime projections in the same transaction
/// as the canonical run state. These tables are derived, but allowing them to
/// commit later would make crash recovery and takeover observations disagree.
fn update_derived_state(
    tx: &Transaction<'_>,
    run_id: &str,
    status: HiveRunStatus,
    now: &str,
) -> Result<()> {
    let (controller_id, session_id, occurrence_id) = tx.query_row(
        "SELECT controller_id, session_id, occurrence_id FROM hive_runs WHERE id = ?1",
        [run_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;

    if status == HiveRunStatus::RecoveryRequired {
        tx.execute(
            "UPDATE hive_controllers SET status = 'paused', updated_at = ?2 WHERE id = ?1",
            params![controller_id, now],
        )?;
    }

    if let Some(occurrence_id) = occurrence_id {
        let occurrence_status = match status {
            HiveRunStatus::Queued | HiveRunStatus::Leased => "queued",
            HiveRunStatus::Succeeded => "succeeded",
            HiveRunStatus::Cancelled => "cancelled",
            HiveRunStatus::Failed | HiveRunStatus::DeadLetter | HiveRunStatus::RecoveryRequired => {
                "failed"
            }
            HiveRunStatus::Running
            | HiveRunStatus::Sleeping
            | HiveRunStatus::RetryWait
            | HiveRunStatus::AwaitingInput => "running",
        };
        tx.execute(
            "UPDATE hive_schedule_occurrences
             SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![occurrence_id, occurrence_status, now],
        )?;
    }

    if let Some(session_id) = session_id {
        let recovery_run_id = tx
            .query_row(
                "SELECT id FROM hive_runs
                 WHERE controller_id = ?1 AND status = 'recovery_required'
                 ORDER BY updated_at DESC, id ASC LIMIT 1",
                [&controller_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let controller_status = tx.query_row(
            "SELECT status FROM hive_controllers WHERE id = ?1",
            [&controller_id],
            |row| row.get::<_, String>(0),
        )?;
        let active = tx
            .query_row(
                "SELECT id, status FROM hive_runs
                 WHERE controller_id = ?1
                   AND status IN ('queued', 'leased', 'running', 'sleeping', 'retry_wait', 'awaiting_input', 'recovery_required')
                 ORDER BY CASE status
                     WHEN 'running' THEN 0 WHEN 'leased' THEN 1
                     WHEN 'recovery_required' THEN 2 WHEN 'awaiting_input' THEN 3
                     WHEN 'sleeping' THEN 4 WHEN 'queued' THEN 5 ELSE 6 END,
                     updated_at DESC, id ASC LIMIT 1",
                [&controller_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let (runtime_status, current_run_id) = if let Some(recovery_run_id) = recovery_run_id {
            ("error", recovery_run_id)
        } else if controller_status == "paused" {
            (
                "paused",
                active
                    .map(|(active_run_id, _)| active_run_id)
                    .unwrap_or_else(|| run_id.to_string()),
            )
        } else if let Some((active_run_id, active_status)) = active {
            let runtime_status = match active_status.as_str() {
                "running" | "leased" => "running",
                "recovery_required" => "error",
                "awaiting_input" => "awaiting_input",
                "sleeping" => "sleeping",
                _ => "idle",
            };
            (runtime_status, active_run_id)
        } else {
            let runtime_status = match status {
                HiveRunStatus::Cancelled => "cancelled",
                HiveRunStatus::Failed
                | HiveRunStatus::DeadLetter
                | HiveRunStatus::RecoveryRequired => "error",
                _ => "idle",
            };
            (runtime_status, run_id.to_string())
        };
        tx.execute(
            "INSERT INTO hive_runtime_state (session_id, status, current_run_id, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET status = excluded.status,
                 current_run_id = excluded.current_run_id,
                 updated_at = excluded.updated_at",
            params![session_id, runtime_status, current_run_id, now],
        )?;
    }
    Ok(())
}

fn discard_pending_controls(
    tx: &Transaction<'_>,
    run_id: &str,
    reason: &str,
    now: &str,
) -> Result<()> {
    tx.execute(
        "UPDATE hive_control_outbox
         SET status = 'discarded', last_error = ?2, updated_at = ?3
         WHERE run_id = ?1 AND status = 'pending'",
        params![run_id, reason, now],
    )?;
    Ok(())
}

/// Scheduled and controller-child objectives are not necessarily represented
/// by an initiating chat message. Materialize them exactly once at the durable
/// running boundary so the hosted agent sees the actual objective in canonical
/// history before it begins, including after a process restart or retry.
fn materialize_objective_message(tx: &Transaction<'_>, run_id: &str, now: &str) -> Result<()> {
    let objective = tx
        .query_row(
            "SELECT session_id, kind, objective FROM hive_runs
             WHERE id = ?1 AND objective_message_id IS NULL
               AND kind IN ('scheduled', 'controller_child')",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((Some(session_id), kind, objective)) = objective else {
        return Ok(());
    };
    let objective = objective.trim();
    if objective.is_empty() {
        return Ok(());
    }
    let rendered = format!("Hive {kind} objective:\n{objective}");
    let content = serde_json::to_string(&vec![Content::Text {
        text: rendered.clone(),
    }])?;
    tx.execute(
        "INSERT INTO messages (session_id, role, content, created_at)
         VALUES (?1, 'user', ?2, ?3)",
        params![session_id, content, now],
    )?;
    let message_id = tx.last_insert_rowid();
    let body = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
    let body = truncate_utf8_bytes(&body, 16 * 1024).to_string();
    let mut hash_material = Vec::with_capacity("user".len() + 1 + body.len());
    hash_material.extend_from_slice(b"user");
    hash_material.push(0);
    hash_material.extend_from_slice(body.as_bytes());
    tx.execute(
        "INSERT INTO conversation_episodes (
            session_id, source_message_id, role, body, content_hash, occurred_at
         ) VALUES (?1, ?2, 'user', ?3, ?4, ?5)",
        params![
            session_id,
            message_id,
            body,
            hash_request_bytes(hash_material),
            now
        ],
    )?;
    tx.execute(
        "UPDATE hive_runs SET objective_message_id = ?2 WHERE id = ?1",
        params![run_id, message_id],
    )?;
    tx.execute(
        "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
        params![session_id, now],
    )?;
    Ok(())
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

fn attempt_outcome(status: HiveRunStatus) -> HiveRunAttemptOutcome {
    match status {
        HiveRunStatus::Succeeded => HiveRunAttemptOutcome::Succeeded,
        HiveRunStatus::Failed => HiveRunAttemptOutcome::Failed,
        HiveRunStatus::RetryWait => HiveRunAttemptOutcome::RetryScheduled,
        HiveRunStatus::Sleeping => HiveRunAttemptOutcome::Sleeping,
        HiveRunStatus::AwaitingInput => HiveRunAttemptOutcome::AwaitingInput,
        HiveRunStatus::RecoveryRequired => HiveRunAttemptOutcome::RecoveryRequired,
        HiveRunStatus::Cancelled => HiveRunAttemptOutcome::Cancelled,
        HiveRunStatus::DeadLetter => HiveRunAttemptOutcome::DeadLetter,
        HiveRunStatus::Queued | HiveRunStatus::Leased | HiveRunStatus::Running => {
            HiveRunAttemptOutcome::Abandoned
        }
    }
}

fn map_run(row: &Row<'_>) -> rusqlite::Result<HiveRun> {
    let kind = parse_required(5, row.get::<_, String>(5)?, HiveRunKind::parse)?;
    let config_json = row.get::<_, String>(7)?;
    let config = serde_json::from_str(&config_json)
        .map_err(|error| conversion_error(7, format!("invalid run config JSON: {error}")))?;
    let status = parse_required(8, row.get::<_, String>(8)?, HiveRunStatus::parse)?;
    let outcome_json = row.get::<_, Option<String>>(23)?;
    let outcome = outcome_json
        .map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| conversion_error(23, format!("invalid outcome JSON: {error}")))
        })
        .transpose()?;
    Ok(HiveRun {
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

fn map_attempt(row: &Row<'_>) -> rusqlite::Result<HiveRunAttempt> {
    let outcome = parse_required(8, row.get::<_, String>(8)?, HiveRunAttemptOutcome::parse)?;
    Ok(HiveRunAttempt {
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
             SELECT 1 FROM hive_daemon_leases
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
