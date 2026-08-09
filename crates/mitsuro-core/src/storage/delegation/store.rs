use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::storage::Database;

use super::model::{
    DelegationCapacityClass, DelegationCapacityFeedback, DelegationCapacityPolicy,
    DelegationCapacityRequest, DelegationCompletionPolicy, DelegationEventRecord,
    DelegationEventType, DelegationFailurePolicy, DelegationGroupContract, DelegationGroupRecord,
    DelegationGroupStartInput, DelegationGroupState, DelegationParentContinuationState,
    DelegationSynthesisLease, DelegationTaskLease, DelegationTaskRecord, DelegationTaskSpec,
    DelegationTaskState,
};

const MAX_ATTEMPT_ARTIFACT_BYTES: usize = 256 * 1024;
const REPLAY_OWNER_LEASE_TTL_MS: i64 = 30_000;

#[derive(Debug, Clone)]
pub struct DelegationTaskLeaseRenewal {
    pub delegation_task_id: String,
    pub lease_owner_id: String,
    pub lease_ttl_ms: i64,
}

#[derive(Debug, Clone)]
pub struct DelegationSynthesisLeaseRenewal {
    pub delegation_group_id: String,
    pub lease_owner_id: String,
    pub lease_ttl_ms: i64,
}

#[derive(Debug, Default)]
pub struct DelegationLeaseRenewalBatchResult {
    pub task_renewed: Vec<bool>,
    pub synthesis_renewed: Vec<bool>,
}

pub struct DelegationStore {
    db: Database,
}

impl DelegationStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Claim cross-process authority to reconstruct one replayable group.
    /// A periodic scan may adopt only after the previous lease expires.
    pub fn try_claim_replay_owner(&self, delegation_group_id: &str) -> Result<Option<String>> {
        let owner_id = Uuid::new_v4().to_string();
        let now_ms = Utc::now().timestamp_millis();
        let expires_at_ms = now_ms + REPLAY_OWNER_LEASE_TTL_MS;
        let now = Utc::now().to_rfc3339();
        let changed = self.db.conn().execute(
            "UPDATE delegation_groups
                SET replay_owner_id = ?2,
                    replay_lease_expires_at_ms = ?3,
                    replay_attempt_count = replay_attempt_count + 1,
                    updated_at = ?4
              WHERE delegation_group_id = ?1
                AND state IN ('queued', 'running', 'ready_for_parent', 'synthesizing')
                AND EXISTS (
                    SELECT 1 FROM delegation_tasks
                     WHERE delegation_tasks.delegation_group_id = delegation_groups.delegation_group_id
                )
                AND NOT EXISTS (
                    SELECT 1 FROM delegation_tasks
                     WHERE delegation_tasks.delegation_group_id = delegation_groups.delegation_group_id
                       AND (
                           executor_envelope_version != 1
                           OR executor_envelope_version IS NULL
                           OR executor_envelope_json IS NULL
                       )
                )
                AND (
                    replay_owner_id IS NULL
                    OR replay_lease_expires_at_ms IS NULL
                    OR replay_lease_expires_at_ms <= ?5
                )",
            params![
                delegation_group_id,
                owner_id,
                expires_at_ms,
                now,
                now_ms,
            ],
        )?;
        Ok((changed == 1).then_some(owner_id))
    }

    pub fn renew_replay_owner(&self, delegation_group_id: &str, owner_id: &str) -> Result<bool> {
        let now_ms = Utc::now().timestamp_millis();
        let changed = self.db.conn().execute(
            "UPDATE delegation_groups
                SET replay_lease_expires_at_ms = ?3, updated_at = ?4
              WHERE delegation_group_id = ?1
                AND replay_owner_id = ?2
                AND replay_lease_expires_at_ms > ?5
                AND state IN ('queued', 'running', 'ready_for_parent', 'synthesizing')",
            params![
                delegation_group_id,
                owner_id,
                now_ms + REPLAY_OWNER_LEASE_TTL_MS,
                Utc::now().to_rfc3339(),
                now_ms,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn replay_owner_is_current(
        &self,
        delegation_group_id: &str,
        owner_id: &str,
    ) -> Result<bool> {
        let now_ms = Utc::now().timestamp_millis();
        Ok(self.db.conn().query_row(
            "SELECT EXISTS (
                SELECT 1 FROM delegation_groups
                 WHERE delegation_group_id = ?1
                   AND replay_owner_id = ?2
                   AND replay_lease_expires_at_ms > ?3
            )",
            params![delegation_group_id, owner_id, now_ms],
            |row| row.get(0),
        )?)
    }

    pub fn release_replay_owner(&self, delegation_group_id: &str, owner_id: &str) -> Result<bool> {
        let changed = self.db.conn().execute(
            "UPDATE delegation_groups
                SET replay_owner_id = NULL, replay_lease_expires_at_ms = NULL,
                    updated_at = ?3
              WHERE delegation_group_id = ?1 AND replay_owner_id = ?2",
            params![delegation_group_id, owner_id, Utc::now().to_rfc3339()],
        )?;
        Ok(changed == 1)
    }

    /// Atomically establishes the immutable group contract and every logical
    /// task before any execution attempt is allowed to start.
    pub fn create_group(&self, input: &DelegationGroupStartInput) -> Result<DelegationGroupRecord> {
        ensure!(
            !input.delegation_group_id.trim().is_empty(),
            "delegation group id is required"
        );
        ensure!(
            !input.parent_session_id.trim().is_empty(),
            "parent session id is required"
        );
        input.contract.validate(input.tasks.len())?;

        let mut task_ids = BTreeSet::new();
        let mut task_keys = BTreeSet::new();
        for task in &input.tasks {
            task.validate()?;
            ensure!(
                task_ids.insert(task.delegation_task_id.as_str()),
                "duplicate delegation task id"
            );
            ensure!(
                task_keys.insert(task.task_key.as_str()),
                "duplicate delegation task key"
            );
            if let Some(envelope) = task.executor_envelope.as_ref() {
                ensure!(
                    input.contract.execution_mode
                        == super::model::DelegationExecutionMode::Detached,
                    "executor envelopes are only valid for detached delegation"
                );
                ensure!(
                    envelope.session_id == input.parent_session_id,
                    "executor envelope belongs to another parent session"
                );
                ensure!(
                    envelope.parent_tool_call_id == input.parent_tool_call_id,
                    "executor envelope belongs to another parent tool call"
                );
                let (session_type, user_id): (String, Option<String>) = self
                    .db
                    .conn()
                    .query_row(
                        "SELECT session_type, user_id FROM sessions WHERE id = ?1",
                        params![input.parent_session_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?
                    .context("executor envelope parent session does not exist")?;
                let expected_session_type = match envelope.session_type {
                    super::model::DelegationExecutorSessionType::Chat => "chat",
                    super::model::DelegationExecutorSessionType::Code => "code",
                };
                ensure!(
                    session_type == expected_session_type && envelope.user_id == user_id,
                    "executor envelope ownership differs from its parent session"
                );
            }
        }

        let now = Utc::now().to_rfc3339();
        let contract_json = serde_json::to_string(&input.contract)?;
        let (continuation_state, continuation_id) = match input.contract.execution_mode {
            super::model::DelegationExecutionMode::Foreground => {
                (DelegationParentContinuationState::NotRequested, None)
            }
            super::model::DelegationExecutionMode::Detached => (
                DelegationParentContinuationState::Pending,
                Some(format!("child-wake-{}", input.delegation_group_id)),
            ),
        };
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO delegation_groups (
                delegation_group_id, parent_session_id, parent_tool_call_id,
                state, contract_json, parent_continuation_state,
                parent_continuation_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'created', ?4, ?5, ?6, ?7, ?7)",
            params![
                input.delegation_group_id,
                input.parent_session_id,
                input.parent_tool_call_id,
                contract_json,
                continuation_state.as_str(),
                continuation_id,
                now,
            ],
        )?;
        for (ordinal, task) in input.tasks.iter().enumerate() {
            let specification_json = serde_json::to_string(task)?;
            let executor_envelope_json = task
                .executor_envelope
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let executor_envelope_version = task
                .executor_envelope
                .as_ref()
                .map(|envelope| envelope.version as i64);
            tx.execute(
                "INSERT INTO delegation_tasks (
                    delegation_task_id, delegation_group_id, task_key, ordinal,
                    role, state, specification_json, executor_envelope_version,
                    executor_envelope_json, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'created', ?6, ?7, ?8, ?9, ?9)",
                params![
                    task.delegation_task_id,
                    input.delegation_group_id,
                    task.task_key,
                    ordinal as i64,
                    task.role.as_str(),
                    specification_json,
                    executor_envelope_version,
                    executor_envelope_json,
                    now,
                ],
            )?;
        }
        append_event(
            &tx,
            &input.delegation_group_id,
            None,
            DelegationEventType::GroupCreated,
            &serde_json::json!({
                "task_count": input.tasks.len(),
                "execution_mode": input.contract.execution_mode,
                "tasks": input.tasks.iter().map(|task| serde_json::json!({
                    "delegation_task_id": task.delegation_task_id,
                    "task_key": task.task_key,
                })).collect::<Vec<_>>(),
            }),
            &now,
        )?;
        tx.commit()?;
        self.get_group(&input.delegation_group_id)?
            .context("created delegation group was not readable")
    }

    /// Make an entire logical operation schedulable in one transaction.
    pub fn queue_group(&self, delegation_group_id: &str) -> Result<DelegationGroupRecord> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_groups
                SET state = 'queued', updated_at = ?2
              WHERE delegation_group_id = ?1 AND state = 'created'",
            params![delegation_group_id, now],
        )?;
        ensure!(changed == 1, "delegation group is not in created state");
        tx.execute(
            "UPDATE delegation_tasks
                SET state = 'queued', updated_at = ?2
              WHERE delegation_group_id = ?1 AND state = 'created'",
            params![delegation_group_id, now],
        )?;
        append_event(
            &tx,
            delegation_group_id,
            None,
            DelegationEventType::GroupQueued,
            &serde_json::json!({}),
            &now,
        )?;
        tx.commit()?;
        self.get_group(delegation_group_id)?
            .context("queued delegation group was not readable")
    }

    /// Claim the next bounded set of logical tasks. Expired owners are
    /// reconciled before admission, and the immutable group contract supplies
    /// the per-operation ceiling rather than a caller-local default.
    pub fn claim_tasks(
        &self,
        delegation_group_id: &str,
        lease_owner_id: &str,
        requested: usize,
        lease_ttl_ms: i64,
    ) -> Result<Vec<DelegationTaskLease>> {
        ensure!(
            !lease_owner_id.trim().is_empty(),
            "delegation lease owner is required"
        );
        ensure!(
            requested > 0,
            "delegation claim size must be greater than zero"
        );
        ensure!(
            lease_ttl_ms > 0,
            "delegation lease TTL must be greater than zero"
        );

        let now_ms = Utc::now().timestamp_millis();
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms);
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let (state_text, contract_json) = tx
            .query_row(
                "SELECT state, contract_json
                   FROM delegation_groups
                  WHERE delegation_group_id = ?1",
                params![delegation_group_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        let group_state = DelegationGroupState::parse(&state_text)
            .context("invalid stored delegation group state")?;
        ensure!(
            matches!(
                group_state,
                DelegationGroupState::Queued | DelegationGroupState::Running
            ),
            "delegation group is not schedulable"
        );
        let contract: DelegationGroupContract = serde_json::from_str(&contract_json)?;

        // A lease is a fencing boundary. Reconcile each expired attempt with
        // append-only state events so reconnect consumers can replay the same
        // retry/failure decision that the authoritative snapshot contains.
        recover_expired_task_leases(&tx, delegation_group_id, now_ms, &now)?;

        let active: usize = tx.query_row(
            "SELECT COUNT(*)
               FROM delegation_tasks
              WHERE delegation_group_id = ?1
                AND state IN ('leased', 'running')",
            params![delegation_group_id],
            |row| row.get::<_, i64>(0).map(|count| count as usize),
        )?;
        let available = contract.governance.max_parallelism.saturating_sub(active);
        let claim_count = requested.min(available);
        if claim_count == 0 {
            tx.commit()?;
            let _ = self.reconcile_group(delegation_group_id)?;
            return Ok(Vec::new());
        }

        let task_ids = {
            let mut statement = tx.prepare(
                "SELECT delegation_task_id
                   FROM delegation_tasks
                  WHERE delegation_group_id = ?1 AND state = 'queued'
                  ORDER BY ordinal ASC
                  LIMIT ?2",
            )?;
            let task_ids = statement
                .query_map(params![delegation_group_id, claim_count as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            task_ids
        };
        for task_id in &task_ids {
            let changed = tx.execute(
                "UPDATE delegation_tasks
                    SET state = 'leased',
                        lease_owner_id = ?2,
                        lease_expires_at_ms = ?3,
                        updated_at = ?4
                  WHERE delegation_task_id = ?1 AND state = 'queued'",
                params![task_id, lease_owner_id, expires_at_ms, now],
            )?;
            ensure!(
                changed == 1,
                "delegation task claim lost its transaction fence"
            );
            let next_attempt_number: i64 = tx.query_row(
                "SELECT attempt_count FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![task_id],
                |row| row.get::<_, i64>(0).map(|count| count + 1),
            )?;
            append_event(
                &tx,
                delegation_group_id,
                Some(task_id),
                DelegationEventType::TaskClaimed,
                &serde_json::json!({"next_attempt_number": next_attempt_number}),
                &now,
            )?;
        }
        if !task_ids.is_empty() && group_state == DelegationGroupState::Queued {
            tx.execute(
                "UPDATE delegation_groups
                    SET state = 'running', updated_at = ?2
                  WHERE delegation_group_id = ?1 AND state = 'queued'",
                params![delegation_group_id, now],
            )?;
            append_event(
                &tx,
                delegation_group_id,
                None,
                DelegationEventType::GroupStateChanged,
                &serde_json::json!({
                    "from": DelegationGroupState::Queued,
                    "to": DelegationGroupState::Running,
                }),
                &now,
            )?;
        }
        tx.commit()?;

        if task_ids.is_empty() {
            let _ = self.reconcile_group(delegation_group_id)?;
        }

        task_ids
            .into_iter()
            .map(|task_id| {
                let task = self
                    .get_task(&task_id)?
                    .with_context(|| format!("claimed delegation task '{task_id}' disappeared"))?;
                Ok(DelegationTaskLease {
                    task,
                    lease_owner_id: lease_owner_id.to_string(),
                    lease_expires_at_ms: expires_at_ms,
                })
            })
            .collect()
    }

    /// Claim one named logical task while enforcing the same group-level
    /// admission and lease fencing as bulk claims. This is used when an
    /// already-materialized runtime task must remain paired with its durable
    /// objective instead of racing another worker for the next queue item.
    pub fn claim_task(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
        lease_ttl_ms: i64,
    ) -> Result<Option<DelegationTaskLease>> {
        ensure!(
            !lease_owner_id.trim().is_empty(),
            "delegation lease owner is required"
        );
        ensure!(
            lease_ttl_ms > 0,
            "delegation lease TTL must be greater than zero"
        );
        let now_ms = Utc::now().timestamp_millis();
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms);
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let (group_id, group_state_text, contract_json) = tx
            .query_row(
                "SELECT tasks.delegation_group_id, groups.state, groups.contract_json
                   FROM delegation_tasks AS tasks
                   JOIN delegation_groups AS groups
                     ON groups.delegation_group_id = tasks.delegation_group_id
                  WHERE tasks.delegation_task_id = ?1",
                params![delegation_task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .with_context(|| format!("unknown delegation task '{delegation_task_id}'"))?;
        let group_state = DelegationGroupState::parse(&group_state_text)
            .context("invalid stored delegation group state")?;
        if !matches!(
            group_state,
            DelegationGroupState::Queued | DelegationGroupState::Running
        ) {
            tx.commit()?;
            return Ok(None);
        }
        let contract: DelegationGroupContract = serde_json::from_str(&contract_json)?;

        recover_expired_task_leases(&tx, &group_id, now_ms, &now)?;
        let task_state = tx.query_row(
            "SELECT state FROM delegation_tasks WHERE delegation_task_id = ?1",
            params![delegation_task_id],
            |row| row.get::<_, String>(0),
        )?;
        if task_state != "queued" {
            tx.commit()?;
            let _ = self.reconcile_group(&group_id)?;
            return Ok(None);
        }
        let active = tx.query_row(
            "SELECT COUNT(*) FROM delegation_tasks
              WHERE delegation_group_id = ?1 AND state IN ('leased', 'running')",
            params![group_id],
            |row| row.get::<_, i64>(0),
        )? as usize;
        if active >= contract.governance.max_parallelism {
            tx.commit()?;
            return Ok(None);
        }
        let changed = tx.execute(
            "UPDATE delegation_tasks
                SET state = 'leased',
                    lease_owner_id = ?2, lease_expires_at_ms = ?3, updated_at = ?4
              WHERE delegation_task_id = ?1 AND state = 'queued'",
            params![delegation_task_id, lease_owner_id, expires_at_ms, now],
        )?;
        ensure!(
            changed == 1,
            "delegation task claim lost its transaction fence"
        );
        let next_attempt_number: i64 = tx.query_row(
            "SELECT attempt_count FROM delegation_tasks WHERE delegation_task_id = ?1",
            params![delegation_task_id],
            |row| row.get::<_, i64>(0).map(|count| count + 1),
        )?;
        append_event(
            &tx,
            &group_id,
            Some(delegation_task_id),
            DelegationEventType::TaskClaimed,
            &serde_json::json!({"next_attempt_number": next_attempt_number}),
            &now,
        )?;
        if group_state == DelegationGroupState::Queued {
            tx.execute(
                "UPDATE delegation_groups SET state = 'running', updated_at = ?2
                  WHERE delegation_group_id = ?1 AND state = 'queued'",
                params![group_id, now],
            )?;
            append_event(
                &tx,
                &group_id,
                None,
                DelegationEventType::GroupStateChanged,
                &serde_json::json!({
                    "from": DelegationGroupState::Queued,
                    "to": DelegationGroupState::Running,
                }),
                &now,
            )?;
        }
        tx.commit()?;
        let task = self
            .get_task(delegation_task_id)?
            .context("claimed delegation task disappeared")?;
        Ok(Some(DelegationTaskLease {
            task,
            lease_owner_id: lease_owner_id.to_string(),
            lease_expires_at_ms: expires_at_ms,
        }))
    }

    pub fn mark_task_running(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
        runtime_key: &str,
    ) -> Result<bool> {
        let now_ms = Utc::now().timestamp_millis();
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_tasks
                SET state = 'running', attempt_count = attempt_count + 1,
                    updated_at = ?3
              WHERE delegation_task_id = ?1
                AND lease_owner_id = ?2
                AND state = 'leased'
                AND lease_expires_at_ms >= ?4
                AND EXISTS (
                    SELECT 1 FROM delegation_groups AS groups
                     WHERE groups.delegation_group_id = delegation_tasks.delegation_group_id
                       AND groups.state = 'running'
                )",
            params![delegation_task_id, lease_owner_id, now, now_ms],
        )?;
        if changed == 1 {
            let group_id: String = tx.query_row(
                "SELECT delegation_group_id FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| row.get(0),
            )?;
            let attempt_number: i64 = tx.query_row(
                "SELECT attempt_count FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| row.get(0),
            )?;
            let attempt_id = format!("{delegation_task_id}:attempt:{attempt_number}");
            tx.execute(
                "INSERT INTO delegation_attempts (
                    attempt_id, delegation_group_id, delegation_task_id,
                    attempt_number, lease_owner_id, runtime_key, state,
                    started_at, last_heartbeat_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?7)",
                params![
                    attempt_id,
                    group_id,
                    delegation_task_id,
                    attempt_number,
                    lease_owner_id,
                    runtime_key,
                    now,
                ],
            )?;
            append_event(
                &tx,
                &group_id,
                Some(delegation_task_id),
                DelegationEventType::TaskRunning,
                &serde_json::json!({
                    "attempt_id": attempt_id,
                    "attempt_number": attempt_number,
                }),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Atomically acquires the durable host/domain/writer capacity lease and
    /// starts the already-claimed task attempt. SQLite's immediate transaction
    /// serializes admission across every process using this database. The
    /// process scheduler may decide who asks first, but cannot exceed this
    /// persisted ceiling.
    pub fn try_admit_and_start_task(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
        runtime_key: &str,
        request: &DelegationCapacityRequest,
        policy: DelegationCapacityPolicy,
    ) -> Result<bool> {
        policy.validate()?;
        ensure!(
            !request.authority_key.trim().is_empty(),
            "capacity authority key is required"
        );
        ensure!(
            !request.domain_key.trim().is_empty(),
            "capacity domain key is required"
        );
        ensure!(
            !request.partition_key.trim().is_empty(),
            "capacity partition key is required"
        );
        ensure!(
            request.authority_key.len() <= 512,
            "capacity authority key is too long"
        );
        ensure!(
            request.domain_key.len() <= 4 * 1024,
            "capacity domain key is too long"
        );
        ensure!(
            request.partition_key.len() <= 4 * 1024,
            "capacity partition key is too long"
        );

        let now_ms = Utc::now().timestamp_millis();
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        reconcile_expired_capacity(&tx, now_ms)?;

        tx.execute(
            "INSERT OR IGNORE INTO delegation_capacity_hosts (
                authority_key, target_limit, minimum_limit, maximum_limit,
                ramp_step, healthy_threshold, default_cooldown_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                request.authority_key,
                policy.initial_limit as i64,
                policy.minimum_limit as i64,
                policy.maximum_limit as i64,
                policy.ramp_step as i64,
                policy.healthy_completions_before_ramp as i64,
                policy.default_cooldown_ms,
                now_ms,
            ],
        )?;
        let host_initial_limit: i64 = tx.query_row(
            "SELECT target_limit FROM delegation_capacity_hosts WHERE authority_key = ?1",
            params![request.authority_key],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO delegation_capacity_domains (
                authority_key, domain_key, target_limit, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                request.authority_key,
                request.domain_key,
                host_initial_limit,
                now_ms
            ],
        )?;

        let task_lease = tx
            .query_row(
                "SELECT state, lease_owner_id, lease_expires_at_ms
                   FROM delegation_tasks
                  WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((state, owner, Some(task_expires_at_ms))) = task_lease else {
            tx.commit()?;
            return Ok(false);
        };
        if state != "leased"
            || owner.as_deref() != Some(lease_owner_id)
            || task_expires_at_ms < now_ms
        {
            tx.execute(
                "DELETE FROM delegation_capacity_waiters
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id],
            )?;
            tx.commit()?;
            return Ok(false);
        }

        tx.execute(
            "INSERT OR IGNORE INTO delegation_capacity_waiters (
                delegation_task_id, lease_owner_id, authority_key, domain_key,
                partition_key, scheduling_class, isolation_group,
                lease_expires_at_ms, enqueued_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                delegation_task_id,
                lease_owner_id,
                request.authority_key,
                request.domain_key,
                request.partition_key,
                request.scheduling_class.as_str(),
                request.isolation_group,
                task_expires_at_ms,
                now_ms,
            ],
        )?;
        let waiter = tx
            .query_row(
                "SELECT waiter_sequence, lease_owner_id, authority_key, domain_key,
                        partition_key, scheduling_class, isolation_group
                   FROM delegation_capacity_waiters
                  WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            waiter_sequence,
            waiter_owner,
            waiter_authority,
            waiter_domain,
            waiter_partition,
            waiter_class,
            waiter_isolation,
        )) = waiter
        else {
            tx.commit()?;
            return Ok(false);
        };
        ensure!(
            waiter_owner == lease_owner_id
                && waiter_authority == request.authority_key
                && waiter_domain == request.domain_key
                && waiter_partition == request.partition_key
                && waiter_class == request.scheduling_class.as_str()
                && waiter_isolation == request.isolation_group,
            "capacity waiter contract changed while the task lease was live"
        );

        let (host_limit, domain_limit, cooldown_until_ms): (i64, i64, Option<i64>) = tx.query_row(
            "SELECT hosts.target_limit, domains.target_limit, domains.cooldown_until_ms
               FROM delegation_capacity_hosts AS hosts
               JOIN delegation_capacity_domains AS domains
                 ON domains.authority_key = hosts.authority_key
              WHERE hosts.authority_key = ?1 AND domains.domain_key = ?2",
            params![request.authority_key, request.domain_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let host_active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM delegation_capacity_leases WHERE authority_key = ?1",
            params![request.authority_key],
            |row| row.get(0),
        )?;
        let domain_active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM delegation_capacity_leases
              WHERE authority_key = ?1 AND domain_key = ?2",
            params![request.authority_key, request.domain_key],
            |row| row.get(0),
        )?;
        // FIFO is authoritative within a provider capacity domain. Different
        // domains may bypass one another so a cooled-down provider cannot
        // head-of-line block healthy provider traffic.
        let earlier_domain_waiter: bool = tx.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM delegation_capacity_waiters
                 WHERE authority_key = ?1 AND domain_key = ?2
                   AND waiter_sequence < ?3
             )",
            params![request.authority_key, request.domain_key, waiter_sequence],
            |row| row.get(0),
        )?;
        let writer_conflict = if matches!(
            request.scheduling_class,
            DelegationCapacityClass::WriteShared | DelegationCapacityClass::WriteIsolated
        ) {
            let mut statement = tx.prepare(
                "SELECT scheduling_class, isolation_group
                   FROM delegation_capacity_leases
                  WHERE authority_key = ?1 AND partition_key = ?2
                    AND scheduling_class IN ('write_shared', 'write_isolated')",
            )?;
            let active_writers = statement
                .query_map(
                    params![request.authority_key, request.partition_key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            active_writers
                .into_iter()
                .any(|(class, isolation)| match request.scheduling_class {
                    DelegationCapacityClass::WriteShared => true,
                    DelegationCapacityClass::WriteIsolated => {
                        class != "write_isolated"
                            || request.isolation_group.is_none()
                            || isolation != request.isolation_group
                    }
                    DelegationCapacityClass::ReadOnly | DelegationCapacityClass::Verification => {
                        false
                    }
                })
        } else {
            false
        };
        let cooling_down = cooldown_until_ms.is_some_and(|until| until > now_ms);
        let blocked = host_active >= host_limit
            || domain_active >= domain_limit
            || cooling_down
            || earlier_domain_waiter
            || writer_conflict;
        if blocked {
            if host_active >= host_limit {
                tx.execute(
                    "UPDATE delegation_capacity_hosts
                        SET demand_observed = 1, updated_at_ms = ?2
                      WHERE authority_key = ?1",
                    params![request.authority_key, now_ms],
                )?;
            }
            if domain_active >= domain_limit || cooling_down || earlier_domain_waiter {
                tx.execute(
                    "UPDATE delegation_capacity_domains
                        SET demand_observed = 1, updated_at_ms = ?3
                      WHERE authority_key = ?1 AND domain_key = ?2",
                    params![request.authority_key, request.domain_key, now_ms],
                )?;
            }
            tx.commit()?;
            return Ok(false);
        }

        tx.execute(
            "INSERT INTO delegation_capacity_leases (
                delegation_task_id, lease_owner_id, authority_key, domain_key,
                partition_key, scheduling_class, isolation_group,
                waiter_sequence, lease_expires_at_ms, admitted_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                delegation_task_id,
                lease_owner_id,
                request.authority_key,
                request.domain_key,
                request.partition_key,
                request.scheduling_class.as_str(),
                request.isolation_group,
                waiter_sequence,
                task_expires_at_ms,
                now_ms,
            ],
        )?;
        tx.execute(
            "DELETE FROM delegation_capacity_waiters WHERE delegation_task_id = ?1",
            params![delegation_task_id],
        )?;
        let changed = tx.execute(
            "UPDATE delegation_tasks
                SET state = 'running', attempt_count = attempt_count + 1, updated_at = ?3
              WHERE delegation_task_id = ?1 AND lease_owner_id = ?2
                AND state = 'leased' AND lease_expires_at_ms >= ?4
                AND EXISTS (
                    SELECT 1 FROM delegation_groups AS groups
                     WHERE groups.delegation_group_id = delegation_tasks.delegation_group_id
                       AND groups.state = 'running'
                )",
            params![delegation_task_id, lease_owner_id, now, now_ms],
        )?;
        ensure!(changed == 1, "capacity admission lost its task lease fence");
        let (group_id, attempt_number): (String, i64) = tx.query_row(
            "SELECT delegation_group_id, attempt_count FROM delegation_tasks
              WHERE delegation_task_id = ?1",
            params![delegation_task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let attempt_id = format!("{delegation_task_id}:attempt:{attempt_number}");
        tx.execute(
            "INSERT INTO delegation_attempts (
                attempt_id, delegation_group_id, delegation_task_id,
                attempt_number, lease_owner_id, runtime_key, state,
                started_at, last_heartbeat_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?7)",
            params![
                attempt_id,
                group_id,
                delegation_task_id,
                attempt_number,
                lease_owner_id,
                runtime_key,
                now
            ],
        )?;
        append_event(
            &tx,
            &group_id,
            Some(delegation_task_id),
            DelegationEventType::TaskRunning,
            &serde_json::json!({
                "attempt_id": attempt_id,
                "attempt_number": attempt_number,
                "capacity_domain": request.domain_key,
                "capacity_waiter_sequence": waiter_sequence,
            }),
            &now,
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn renew_task_lease(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
        lease_ttl_ms: i64,
    ) -> Result<bool> {
        ensure!(
            lease_ttl_ms > 0,
            "delegation lease TTL must be greater than zero"
        );
        let expires_at_ms = Utc::now().timestamp_millis().saturating_add(lease_ttl_ms);
        let now_ms = Utc::now().timestamp_millis();
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_tasks
                SET lease_expires_at_ms = ?3, updated_at = ?4
              WHERE delegation_task_id = ?1
                AND lease_owner_id = ?2
                AND state IN ('leased', 'running')
                AND lease_expires_at_ms >= ?5
                AND EXISTS (
                    SELECT 1 FROM delegation_groups AS groups
                     WHERE groups.delegation_group_id = delegation_tasks.delegation_group_id
                       AND groups.state = 'running'
                )",
            params![
                delegation_task_id,
                lease_owner_id,
                expires_at_ms,
                now,
                now_ms,
            ],
        )?;
        if changed == 1 {
            tx.execute(
                "UPDATE delegation_capacity_waiters
                    SET lease_expires_at_ms = ?3
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id, expires_at_ms],
            )?;
            tx.execute(
                "UPDATE delegation_capacity_leases
                    SET lease_expires_at_ms = ?3
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id, expires_at_ms],
            )?;
            tx.execute(
                "UPDATE delegation_attempts
                    SET last_heartbeat_at = ?3
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2
                    AND state = 'running'",
                params![delegation_task_id, lease_owner_id, now],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Renew every due task/capacity and synthesis lease under one immediate
    /// transaction. Callers retain positional ownership of the result vectors;
    /// this method never discovers or claims additional work.
    pub fn renew_lease_batch(
        &self,
        task_renewals: &[DelegationTaskLeaseRenewal],
        synthesis_renewals: &[DelegationSynthesisLeaseRenewal],
    ) -> Result<DelegationLeaseRenewalBatchResult> {
        Self::renew_lease_batch_on_connection(self.db.conn(), task_renewals, synthesis_renewals)
    }

    pub fn renew_lease_batch_on_connection(
        connection: &rusqlite::Connection,
        task_renewals: &[DelegationTaskLeaseRenewal],
        synthesis_renewals: &[DelegationSynthesisLeaseRenewal],
    ) -> Result<DelegationLeaseRenewalBatchResult> {
        for renewal in task_renewals {
            ensure!(
                renewal.lease_ttl_ms > 0,
                "delegation lease TTL must be greater than zero"
            );
        }
        for renewal in synthesis_renewals {
            ensure!(
                renewal.lease_ttl_ms > 0,
                "delegation synthesis lease TTL must be greater than zero"
            );
        }
        let now_ms = Utc::now().timestamp_millis();
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
        let mut task_renewed = Vec::with_capacity(task_renewals.len());
        for renewal in task_renewals {
            let expires_at_ms = now_ms.saturating_add(renewal.lease_ttl_ms);
            let changed = tx.execute(
                "UPDATE delegation_tasks
                    SET lease_expires_at_ms = ?3, updated_at = ?4
                  WHERE delegation_task_id = ?1
                    AND lease_owner_id = ?2
                    AND state IN ('leased', 'running')
                    AND lease_expires_at_ms >= ?5
                    AND EXISTS (
                        SELECT 1 FROM delegation_groups AS groups
                         WHERE groups.delegation_group_id = delegation_tasks.delegation_group_id
                           AND groups.state = 'running'
                    )",
                params![
                    renewal.delegation_task_id,
                    renewal.lease_owner_id,
                    expires_at_ms,
                    now,
                    now_ms,
                ],
            )?;
            if changed == 1 {
                tx.execute(
                    "UPDATE delegation_capacity_waiters
                        SET lease_expires_at_ms = ?3
                      WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                    params![
                        renewal.delegation_task_id,
                        renewal.lease_owner_id,
                        expires_at_ms,
                    ],
                )?;
                tx.execute(
                    "UPDATE delegation_capacity_leases
                        SET lease_expires_at_ms = ?3
                      WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                    params![
                        renewal.delegation_task_id,
                        renewal.lease_owner_id,
                        expires_at_ms,
                    ],
                )?;
                tx.execute(
                    "UPDATE delegation_attempts
                        SET last_heartbeat_at = ?3
                      WHERE delegation_task_id = ?1 AND lease_owner_id = ?2
                        AND state = 'running'",
                    params![renewal.delegation_task_id, renewal.lease_owner_id, now,],
                )?;
            }
            task_renewed.push(changed == 1);
        }

        let mut synthesis_renewed = Vec::with_capacity(synthesis_renewals.len());
        for renewal in synthesis_renewals {
            let expires_at_ms = now_ms.saturating_add(renewal.lease_ttl_ms);
            let changed = tx.execute(
                "UPDATE delegation_groups
                    SET synthesis_lease_expires_at_ms = ?3, updated_at = ?4
                  WHERE delegation_group_id = ?1
                    AND synthesis_owner_id = ?2
                    AND state = 'synthesizing'
                    AND synthesis_lease_expires_at_ms >= ?5",
                params![
                    renewal.delegation_group_id,
                    renewal.lease_owner_id,
                    expires_at_ms,
                    now,
                    now_ms,
                ],
            )?;
            synthesis_renewed.push(changed == 1);
        }
        tx.commit()?;
        Ok(DelegationLeaseRenewalBatchResult {
            task_renewed,
            synthesis_renewed,
        })
    }

    pub fn release_task_claim(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_tasks
                SET state = 'queued', lease_owner_id = NULL,
                    lease_expires_at_ms = NULL, updated_at = ?3
              WHERE delegation_task_id = ?1
                AND lease_owner_id = ?2
                AND state = 'leased'",
            params![delegation_task_id, lease_owner_id, now],
        )?;
        if changed == 1 {
            tx.execute(
                "DELETE FROM delegation_capacity_waiters
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id],
            )?;
            tx.execute(
                "DELETE FROM delegation_capacity_leases
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id],
            )?;
            let group_id: String = tx.query_row(
                "SELECT delegation_group_id FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| row.get(0),
            )?;
            append_event(
                &tx,
                &group_id,
                Some(delegation_task_id),
                DelegationEventType::TaskStateChanged,
                &serde_json::json!({"state": DelegationTaskState::Queued, "reason": "scheduler_wait_cancelled"}),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn complete_task(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
        terminal_state: DelegationTaskState,
        result: Option<&Value>,
        error_summary: Option<&str>,
    ) -> Result<bool> {
        self.complete_task_with_capacity_feedback(
            delegation_task_id,
            lease_owner_id,
            terminal_state,
            result,
            error_summary,
            DelegationCapacityFeedback::Neutral,
        )
    }

    /// Complete the task, release its durable capacity slot, and update the
    /// shared adaptive domain in one immediate transaction. A crash cannot
    /// publish completion while leaving a ghost slot or feedback that only one
    /// process observed.
    pub fn complete_task_with_capacity_feedback(
        &self,
        delegation_task_id: &str,
        lease_owner_id: &str,
        terminal_state: DelegationTaskState,
        result: Option<&Value>,
        error_summary: Option<&str>,
        capacity_feedback: DelegationCapacityFeedback,
    ) -> Result<bool> {
        ensure!(
            terminal_state.is_terminal(),
            "delegation task completion requires a terminal state"
        );
        let now = Utc::now().to_rfc3339();
        let result_json = result.map(serde_json::to_string).transpose()?;
        ensure!(
            result_json
                .as_ref()
                .is_none_or(|artifact| artifact.len() <= MAX_ATTEMPT_ARTIFACT_BYTES),
            "delegation attempt artifact exceeds the durable size limit"
        );
        let now_ms = Utc::now().timestamp_millis();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let capacity_domain = tx
            .query_row(
                "SELECT authority_key, domain_key
                   FROM delegation_capacity_leases
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let changed = tx.execute(
            "UPDATE delegation_tasks
                SET state = ?3, result_json = ?4, error_summary = ?5,
                    lease_owner_id = NULL, lease_expires_at_ms = NULL,
                    updated_at = ?6, completed_at = COALESCE(completed_at, ?6)
              WHERE delegation_task_id = ?1
                AND lease_owner_id = ?2
                AND state IN ('leased', 'running')
                AND lease_expires_at_ms >= ?7
                AND EXISTS (
                    SELECT 1 FROM delegation_groups AS groups
                     WHERE groups.delegation_group_id = delegation_tasks.delegation_group_id
                       AND groups.state = 'running'
                )",
            params![
                delegation_task_id,
                lease_owner_id,
                terminal_state.as_str(),
                result_json,
                error_summary,
                now,
                now_ms,
            ],
        )?;
        let group_id = if changed == 1 {
            tx.execute(
                "DELETE FROM delegation_capacity_waiters
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id],
            )?;
            tx.execute(
                "DELETE FROM delegation_capacity_leases
                  WHERE delegation_task_id = ?1 AND lease_owner_id = ?2",
                params![delegation_task_id, lease_owner_id],
            )?;
            if let Some((authority_key, domain_key)) = capacity_domain.as_ref() {
                apply_capacity_feedback(&tx, authority_key, domain_key, capacity_feedback, now_ms)?;
            }
            let group_id: String = tx.query_row(
                "SELECT delegation_group_id FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| row.get(0),
            )?;
            let attempt_number: i64 = tx.query_row(
                "SELECT attempt_count FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| row.get(0),
            )?;
            let attempt_id = format!("{delegation_task_id}:attempt:{attempt_number}");
            tx.execute(
                "UPDATE delegation_attempts
                    SET state = ?3, artifact_json = ?4, error_summary = ?5,
                        last_heartbeat_at = ?6, completed_at = COALESCE(completed_at, ?6)
                  WHERE attempt_id = ?1 AND lease_owner_id = ?2 AND state = 'running'",
                params![
                    attempt_id,
                    lease_owner_id,
                    terminal_state.as_str(),
                    result_json,
                    error_summary,
                    now,
                ],
            )?;
            append_event(
                &tx,
                &group_id,
                Some(delegation_task_id),
                DelegationEventType::TaskStateChanged,
                &serde_json::json!({
                    "state": terminal_state,
                    "attempt_id": attempt_id,
                    "attempt_number": attempt_number,
                }),
                &now,
            )?;
            Some(group_id)
        } else {
            None
        };
        tx.commit()?;
        if let Some(group_id) = group_id {
            self.reconcile_group(&group_id)?;
        }
        Ok(changed == 1)
    }

    /// Reduce child state into exactly one parent-owned group transition.
    pub fn reconcile_group(&self, delegation_group_id: &str) -> Result<DelegationGroupState> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let (state_text, contract_json) = tx
            .query_row(
                "SELECT state, contract_json FROM delegation_groups WHERE delegation_group_id = ?1",
                params![delegation_group_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        let current = DelegationGroupState::parse(&state_text)
            .context("invalid stored delegation group state")?;
        if current.is_terminal() || current == DelegationGroupState::ReadyForParent {
            tx.commit()?;
            return Ok(current);
        }
        let contract: DelegationGroupContract = serde_json::from_str(&contract_json)?;
        let (total, complete, degraded, failed, terminal): (i64, i64, i64, i64, i64) = tx.query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN state = 'complete' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state = 'degraded' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state = 'failed' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state IN ('complete', 'degraded', 'failed', 'cancelled') THEN 1 ELSE 0 END)
               FROM delegation_tasks WHERE delegation_group_id = ?1",
            params![delegation_group_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        let usable = complete + degraded;
        let remaining = total - terminal;
        let next = if contract.failure_policy == DelegationFailurePolicy::FailFast && failed > 0 {
            Some(DelegationGroupState::Failed)
        } else {
            match contract.completion_policy {
                DelegationCompletionPolicy::AllSettled if remaining == 0 => Some(if usable > 0 {
                    DelegationGroupState::ReadyForParent
                } else {
                    DelegationGroupState::Failed
                }),
                DelegationCompletionPolicy::AnySuccess if complete > 0 => {
                    Some(DelegationGroupState::ReadyForParent)
                }
                DelegationCompletionPolicy::AnySuccess if remaining == 0 => {
                    Some(DelegationGroupState::Failed)
                }
                DelegationCompletionPolicy::Quorum { required } if usable >= required as i64 => {
                    Some(DelegationGroupState::ReadyForParent)
                }
                DelegationCompletionPolicy::Quorum { required }
                    if usable + remaining < required as i64 =>
                {
                    Some(DelegationGroupState::Failed)
                }
                _ => None,
            }
        };
        let Some(next) = next else {
            tx.commit()?;
            return Ok(current);
        };
        tx.execute(
            "UPDATE delegation_groups
                SET state = ?2, updated_at = ?3,
                    completed_at = CASE WHEN ?4 = 1 THEN COALESCE(completed_at, ?3) ELSE completed_at END
              WHERE delegation_group_id = ?1 AND state = ?5",
            params![
                delegation_group_id,
                next.as_str(),
                now,
                if next.is_terminal() { 1 } else { 0 },
                current.as_str(),
            ],
        )?;
        append_event(
            &tx,
            delegation_group_id,
            None,
            DelegationEventType::GroupStateChanged,
            &serde_json::json!({"from": current, "to": next}),
            &now,
        )?;
        // Early completion (AnySuccess/Quorum) and fail-fast both close the
        // group's execution epoch. Fence every sibling in the same transaction
        // before exposing ReadyForParent/Failed so a late worker cannot keep
        // side-effecting or strand durable capacity after synthesis begins.
        if matches!(
            next,
            DelegationGroupState::ReadyForParent | DelegationGroupState::Failed
        ) {
            let cancellation_reason = if next == DelegationGroupState::Failed {
                "group_failed"
            } else {
                "completion_policy_satisfied"
            };
            let cancelled_task_ids = {
                let mut statement = tx.prepare(
                    "SELECT delegation_task_id FROM delegation_tasks
                      WHERE delegation_group_id = ?1
                        AND state IN ('created', 'queued', 'leased', 'running', 'retrying')",
                )?;
                let task_ids = statement
                    .query_map(params![delegation_group_id], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                task_ids
            };
            for task_id in cancelled_task_ids {
                let changed = tx.execute(
                    "UPDATE delegation_tasks
                        SET state = 'cancelled', lease_owner_id = NULL,
                            lease_expires_at_ms = NULL, updated_at = ?2,
                            completed_at = COALESCE(completed_at, ?2)
                      WHERE delegation_task_id = ?1
                        AND state IN ('created', 'queued', 'leased', 'running', 'retrying')",
                    params![task_id, now],
                )?;
                if changed == 1 {
                    tx.execute(
                        "DELETE FROM delegation_capacity_waiters WHERE delegation_task_id = ?1",
                        params![task_id],
                    )?;
                    tx.execute(
                        "DELETE FROM delegation_capacity_leases WHERE delegation_task_id = ?1",
                        params![task_id],
                    )?;
                    tx.execute(
                        "UPDATE delegation_attempts
                            SET state = 'cancelled',
                                error_summary = COALESCE(error_summary, ?3),
                                last_heartbeat_at = ?2,
                                completed_at = COALESCE(completed_at, ?2)
                          WHERE delegation_task_id = ?1 AND state = 'running'",
                        params![
                            task_id,
                            now,
                            if next == DelegationGroupState::Failed {
                                "cancelled by delegation group failure policy"
                            } else {
                                "cancelled after delegation completion policy was satisfied"
                            }
                        ],
                    )?;
                    append_event(
                        &tx,
                        delegation_group_id,
                        Some(&task_id),
                        DelegationEventType::TaskStateChanged,
                        &serde_json::json!({
                            "state": DelegationTaskState::Cancelled,
                            "reason": cancellation_reason,
                        }),
                        &now,
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(next)
    }

    /// Fail closed when the durable background host disappears. This is an
    /// uncertainty handoff, not normal user cancellation: every live attempt
    /// is fenced and the detached parent continuation remains authorized.
    pub fn fail_group_recovery(
        &self,
        delegation_group_id: &str,
        reason: &str,
    ) -> Result<DelegationGroupRecord> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let previous: String = tx
            .query_row(
                "SELECT state FROM delegation_groups WHERE delegation_group_id = ?1",
                params![delegation_group_id],
                |row| row.get(0),
            )
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        let previous_state = DelegationGroupState::parse(&previous)
            .context("invalid stored delegation group state")?;
        if previous_state.is_terminal() {
            tx.commit()?;
            return self
                .get_group(delegation_group_id)?
                .context("terminal delegation group disappeared");
        }
        let task_ids = {
            let mut statement = tx.prepare(
                "SELECT delegation_task_id FROM delegation_tasks
                  WHERE delegation_group_id = ?1
                    AND state NOT IN ('complete', 'degraded', 'failed', 'cancelled')",
            )?;
            let ids = statement
                .query_map(params![delegation_group_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        for task_id in task_ids {
            tx.execute(
                "UPDATE delegation_tasks
                    SET state = 'cancelled', error_summary = COALESCE(error_summary, ?2),
                        lease_owner_id = NULL, lease_expires_at_ms = NULL,
                        updated_at = ?3, completed_at = COALESCE(completed_at, ?3)
                  WHERE delegation_task_id = ?1
                    AND state NOT IN ('complete', 'degraded', 'failed', 'cancelled')",
                params![task_id, reason, now],
            )?;
            tx.execute(
                "UPDATE delegation_attempts
                    SET state = 'cancelled', error_summary = COALESCE(error_summary, ?2),
                        last_heartbeat_at = ?3, completed_at = COALESCE(completed_at, ?3)
                  WHERE delegation_task_id = ?1 AND state = 'running'",
                params![task_id, reason, now],
            )?;
            append_event(
                &tx,
                delegation_group_id,
                Some(&task_id),
                DelegationEventType::TaskStateChanged,
                &serde_json::json!({
                    "state": DelegationTaskState::Cancelled,
                    "reason": "background_host_lost",
                }),
                &now,
            )?;
        }
        tx.execute(
            "UPDATE delegation_groups
                SET state = 'failed', updated_at = ?2,
                    synthesis_owner_id = NULL, synthesis_lease_expires_at_ms = NULL,
                    completed_at = COALESCE(completed_at, ?2)
              WHERE delegation_group_id = ?1
                AND state NOT IN ('complete', 'degraded', 'failed', 'cancelled')",
            params![delegation_group_id, now],
        )?;
        append_event(
            &tx,
            delegation_group_id,
            None,
            DelegationEventType::GroupStateChanged,
            &serde_json::json!({
                "from": previous_state,
                "to": DelegationGroupState::Failed,
                "reason": "background_host_lost",
            }),
            &now,
        )?;
        tx.commit()?;
        self.get_group(delegation_group_id)?
            .context("recovered delegation group disappeared")
    }

    pub fn get_group(&self, delegation_group_id: &str) -> Result<Option<DelegationGroupRecord>> {
        let group = self
            .db
            .conn()
            .query_row(
                "SELECT delegation_group_id, parent_session_id, parent_tool_call_id,
                        state, contract_json, parent_continuation_state,
                        parent_continuation_id, synthesis_owner_id,
                        synthesis_lease_expires_at_ms, synthesis_attempt_count,
                        created_at, updated_at, completed_at
                   FROM delegation_groups
                  WHERE delegation_group_id = ?1",
                params![delegation_group_id],
                row_to_group,
            )
            .optional()?;
        let Some(mut group) = group else {
            return Ok(None);
        };
        group.tasks = self.list_tasks(delegation_group_id)?;
        Ok(Some(group))
    }

    pub fn list_session_events_after(
        &self,
        parent_session_id: &str,
        cursor: i64,
        limit: usize,
    ) -> Result<Vec<DelegationEventRecord>> {
        let mut statement = self.db.conn().prepare(
            "SELECT event_id, parent_session_id, delegation_group_id,
                    delegation_task_id, event_type, payload_json, created_at
               FROM delegation_events
              WHERE parent_session_id = ?1 AND event_id > ?2
              ORDER BY event_id ASC
              LIMIT ?3",
        )?;
        let events = statement
            .query_map(
                params![parent_session_id, cursor.max(0), limit.clamp(1, 1000)],
                row_to_event,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(events)
    }

    pub fn list_latest_session_events(
        &self,
        parent_session_id: &str,
        limit: usize,
    ) -> Result<Vec<DelegationEventRecord>> {
        let mut statement = self.db.conn().prepare(
            "SELECT event_id, parent_session_id, delegation_group_id,
                    delegation_task_id, event_type, payload_json, created_at
               FROM delegation_events
              WHERE parent_session_id = ?1
              ORDER BY event_id DESC
              LIMIT ?2",
        )?;
        let mut events = statement
            .query_map(
                params![parent_session_id, limit.clamp(1, 1000)],
                row_to_event,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        events.reverse();
        Ok(events)
    }

    pub fn list_groups_for_session(
        &self,
        parent_session_id: &str,
        limit: usize,
    ) -> Result<Vec<DelegationGroupRecord>> {
        let mut statement = self.db.conn().prepare(
            "SELECT delegation_group_id, parent_session_id, parent_tool_call_id,
                    state, contract_json, parent_continuation_state,
                    parent_continuation_id, synthesis_owner_id,
                    synthesis_lease_expires_at_ms, synthesis_attempt_count,
                    created_at, updated_at, completed_at
              FROM delegation_groups
              WHERE parent_session_id = ?1
              ORDER BY CASE
                    WHEN state IN ('complete', 'degraded', 'failed', 'cancelled') THEN 1
                    ELSE 0
                  END ASC,
                  updated_at DESC
              LIMIT ?2",
        )?;
        let mut groups = statement
            .query_map(
                params![parent_session_id, limit.clamp(1, 1000)],
                row_to_group,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for group in &mut groups {
            group.tasks = self.list_tasks(&group.delegation_group_id)?;
        }
        Ok(groups)
    }

    /// Return the durable, nonterminal work that a newly started orchestration
    /// host must recover. Recovery order is oldest group first, with the group
    /// id as a stable tie-breaker, so lease reconciliation cannot reshuffle the
    /// inventory by changing `updated_at`.
    pub fn list_recoverable_groups(&self, limit: usize) -> Result<Vec<DelegationGroupRecord>> {
        let now_ms = Utc::now().timestamp_millis();
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let candidates = {
            let mut statement = tx.prepare(
                "SELECT delegation_group_id, state
                   FROM delegation_groups
                  WHERE state NOT IN ('complete', 'degraded', 'failed', 'cancelled')
                  ORDER BY created_at ASC, delegation_group_id ASC",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };

        for (delegation_group_id, state) in &candidates {
            if matches!(state.as_str(), "queued" | "running") {
                recover_expired_task_leases(&tx, delegation_group_id, now_ms, &now)?;
            }
        }
        tx.commit()?;

        let mut groups = Vec::with_capacity(candidates.len().min(limit.clamp(1, 1000)));
        for (delegation_group_id, state) in candidates {
            if state == "running"
                && self
                    .get_group(&delegation_group_id)?
                    .is_some_and(|group| group.state == DelegationGroupState::Running)
            {
                self.reconcile_group(&delegation_group_id)?;
            }
            if let Some(group) = self.get_group(&delegation_group_id)? {
                if !group.state.is_terminal() {
                    groups.push(group);
                }
            }
        }
        groups.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.delegation_group_id.cmp(&right.delegation_group_id))
        });
        groups.truncate(limit.clamp(1, 1000));
        Ok(groups)
    }

    pub fn authorize_parent_continuation(
        &self,
        delegation_group_id: &str,
        parent_continuation_id: &str,
    ) -> Result<bool> {
        let authorized = self.db.conn().query_row(
            "SELECT COUNT(*) FROM delegation_groups
              WHERE delegation_group_id = ?1
                AND parent_continuation_id = ?2
                AND parent_continuation_state IN ('pending', 'queued', 'promoted')
                AND state IN ('complete', 'degraded', 'failed')",
            params![delegation_group_id, parent_continuation_id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(authorized == 1)
    }

    pub fn mark_parent_continuation_queued(
        &self,
        delegation_group_id: &str,
        parent_continuation_id: &str,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_groups
                SET parent_continuation_state = 'queued', updated_at = ?3
              WHERE delegation_group_id = ?1
                AND parent_continuation_id = ?2
                AND parent_continuation_state = 'pending'
                AND state IN ('complete', 'degraded', 'failed')",
            params![delegation_group_id, parent_continuation_id, now,],
        )?;
        if changed == 1 {
            append_event(
                &tx,
                delegation_group_id,
                None,
                DelegationEventType::ParentContinuationQueued,
                &serde_json::json!({"parent_continuation_id": parent_continuation_id}),
                &now,
            )?;
        }
        let accepted = changed == 1
            || tx.query_row(
                "SELECT COUNT(*) FROM delegation_groups
                  WHERE delegation_group_id = ?1
                    AND parent_continuation_id = ?2
                    AND parent_continuation_state IN ('queued', 'promoted')",
                params![delegation_group_id, parent_continuation_id],
                |row| row.get::<_, i64>(0),
            )? == 1;
        tx.commit()?;
        Ok(accepted)
    }

    pub fn mark_parent_continuation_promoted(
        &self,
        delegation_group_id: &str,
        parent_continuation_id: &str,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_groups
                SET parent_continuation_state = 'promoted', updated_at = ?3
              WHERE delegation_group_id = ?1
                AND parent_continuation_id = ?2
                AND parent_continuation_state IN ('pending', 'queued')
                AND state IN ('complete', 'degraded', 'failed')",
            params![delegation_group_id, parent_continuation_id, now,],
        )?;
        if changed == 1 {
            append_event(
                &tx,
                delegation_group_id,
                None,
                DelegationEventType::ParentContinuationPromoted,
                &serde_json::json!({"parent_continuation_id": parent_continuation_id}),
                &now,
            )?;
        }
        let accepted = changed == 1
            || tx.query_row(
                "SELECT COUNT(*) FROM delegation_groups
                  WHERE delegation_group_id = ?1
                    AND parent_continuation_id = ?2
                    AND parent_continuation_state = 'promoted'",
                params![delegation_group_id, parent_continuation_id],
                |row| row.get::<_, i64>(0),
            )? == 1;
        tx.commit()?;
        Ok(accepted)
    }

    pub fn list_tasks(&self, delegation_group_id: &str) -> Result<Vec<DelegationTaskRecord>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT delegation_group_id, ordinal, specification_json, state,
                    attempt_count, result_json, error_summary,
                    created_at, updated_at, completed_at,
                    executor_envelope_version, executor_envelope_json
               FROM delegation_tasks
              WHERE delegation_group_id = ?1
              ORDER BY ordinal ASC",
        )?;
        let rows = stmt.query_map(params![delegation_group_id], row_to_task)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Claim the single aggregate synthesis authority for a settled group.
    /// An expired Synthesizing owner may be replaced; an unexpired owner is a
    /// hard fence against duplicate patch integration and parent publication.
    pub fn claim_synthesis(
        &self,
        delegation_group_id: &str,
        lease_owner_id: &str,
        lease_ttl_ms: i64,
    ) -> Result<Option<DelegationSynthesisLease>> {
        ensure!(
            !lease_owner_id.trim().is_empty(),
            "delegation synthesis lease owner is required"
        );
        ensure!(
            lease_ttl_ms > 0,
            "delegation synthesis lease TTL must be greater than zero"
        );
        let now_ms = Utc::now().timestamp_millis();
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms);
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let (state_text, current_expiry) = tx
            .query_row(
                "SELECT state, synthesis_lease_expires_at_ms
                   FROM delegation_groups WHERE delegation_group_id = ?1",
                params![delegation_group_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        let state = DelegationGroupState::parse(&state_text)
            .context("invalid stored delegation group state")?;
        let reclaiming = state == DelegationGroupState::Synthesizing
            && current_expiry.is_some_and(|expiry| expiry < now_ms);
        if state != DelegationGroupState::ReadyForParent && !reclaiming {
            tx.commit()?;
            return Ok(None);
        }
        let changed = tx.execute(
            "UPDATE delegation_groups
                SET state = 'synthesizing', synthesis_owner_id = ?2,
                    synthesis_lease_expires_at_ms = ?3,
                    synthesis_attempt_count = synthesis_attempt_count + 1,
                    updated_at = ?4
              WHERE delegation_group_id = ?1
                AND (
                    state = 'ready_for_parent'
                    OR (state = 'synthesizing' AND synthesis_lease_expires_at_ms < ?5)
                )",
            params![
                delegation_group_id,
                lease_owner_id,
                expires_at_ms,
                now,
                now_ms,
            ],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        let synthesis_attempt: i64 = tx.query_row(
            "SELECT synthesis_attempt_count FROM delegation_groups
              WHERE delegation_group_id = ?1",
            params![delegation_group_id],
            |row| row.get(0),
        )?;
        append_event(
            &tx,
            delegation_group_id,
            None,
            DelegationEventType::GroupStateChanged,
            &serde_json::json!({
                "from": state,
                "to": DelegationGroupState::Synthesizing,
                "reason": if reclaiming { "synthesis_lease_reclaimed" } else { "synthesis_claimed" },
                "synthesis_attempt": synthesis_attempt,
            }),
            &now,
        )?;
        tx.commit()?;
        let group = self
            .get_group(delegation_group_id)?
            .context("claimed synthesis group disappeared")?;
        Ok(Some(DelegationSynthesisLease {
            group,
            lease_owner_id: lease_owner_id.to_string(),
            lease_expires_at_ms: expires_at_ms,
        }))
    }

    pub fn renew_synthesis_lease(
        &self,
        delegation_group_id: &str,
        lease_owner_id: &str,
        lease_ttl_ms: i64,
    ) -> Result<bool> {
        ensure!(
            lease_ttl_ms > 0,
            "delegation synthesis lease TTL must be greater than zero"
        );
        let now_ms = Utc::now().timestamp_millis();
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms);
        let now = Utc::now().to_rfc3339();
        let changed = self.db.conn().execute(
            "UPDATE delegation_groups
                SET synthesis_lease_expires_at_ms = ?3, updated_at = ?4
              WHERE delegation_group_id = ?1
                AND synthesis_owner_id = ?2
                AND state = 'synthesizing'
                AND synthesis_lease_expires_at_ms >= ?5",
            params![
                delegation_group_id,
                lease_owner_id,
                expires_at_ms,
                now,
                now_ms,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn complete_synthesis(
        &self,
        delegation_group_id: &str,
        lease_owner_id: &str,
        terminal_state: DelegationGroupState,
    ) -> Result<bool> {
        ensure!(
            terminal_state.is_terminal(),
            "delegation synthesis completion requires a terminal group state"
        );
        let now_ms = Utc::now().timestamp_millis();
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE delegation_groups
                SET state = ?3, synthesis_owner_id = NULL,
                    synthesis_lease_expires_at_ms = NULL, updated_at = ?4,
                    completed_at = COALESCE(completed_at, ?4)
              WHERE delegation_group_id = ?1
                AND synthesis_owner_id = ?2
                AND state = 'synthesizing'
                AND synthesis_lease_expires_at_ms >= ?5",
            params![
                delegation_group_id,
                lease_owner_id,
                terminal_state.as_str(),
                now,
                now_ms,
            ],
        )?;
        if changed == 1 {
            append_event(
                &tx,
                delegation_group_id,
                None,
                DelegationEventType::GroupStateChanged,
                &serde_json::json!({
                    "from": DelegationGroupState::Synthesizing,
                    "to": terminal_state,
                    "reason": "synthesis_completed",
                }),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Direct transition helper for state-machine unit tests. Production code
    /// must use lease-, attempt-, capacity-, and synthesis-aware operations.
    #[cfg(test)]
    pub(crate) fn transition_group(
        &self,
        delegation_group_id: &str,
        next: DelegationGroupState,
    ) -> Result<DelegationGroupRecord> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT state FROM delegation_groups WHERE delegation_group_id = ?1",
                params![delegation_group_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        let current = DelegationGroupState::parse(&current)
            .context("invalid stored delegation group state")?;
        ensure!(
            current.can_transition_to(next),
            "invalid delegation group transition from {} to {}",
            current.as_str(),
            next.as_str()
        );
        tx.execute(
            "UPDATE delegation_groups
                SET state = ?2,
                    updated_at = ?3,
                    synthesis_owner_id = CASE WHEN ?4 = 1 THEN NULL ELSE synthesis_owner_id END,
                    synthesis_lease_expires_at_ms = CASE WHEN ?4 = 1 THEN NULL ELSE synthesis_lease_expires_at_ms END,
                    completed_at = CASE WHEN ?4 = 1 THEN COALESCE(completed_at, ?3) ELSE completed_at END
              WHERE delegation_group_id = ?1 AND state = ?5",
            params![
                delegation_group_id,
                next.as_str(),
                now,
                if next.is_terminal() { 1 } else { 0 },
                current.as_str(),
            ],
        )?;
        append_event(
            &tx,
            delegation_group_id,
            None,
            DelegationEventType::GroupStateChanged,
            &serde_json::json!({"from": current, "to": next}),
            &now,
        )?;
        tx.commit()?;
        self.get_group(delegation_group_id)?
            .context("transitioned delegation group was not readable")
    }

    /// Direct transition helper for state-machine unit tests. It deliberately
    /// is not part of the runtime store API because it bypasses execution
    /// ownership and attempt bookkeeping.
    #[cfg(test)]
    pub(crate) fn transition_task(
        &self,
        delegation_task_id: &str,
        next: DelegationTaskState,
    ) -> Result<DelegationTaskRecord> {
        let now = Utc::now().to_rfc3339();
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = tx
            .query_row(
                "SELECT state FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .with_context(|| format!("unknown delegation task '{delegation_task_id}'"))?;
        let current =
            DelegationTaskState::parse(&current).context("invalid stored delegation task state")?;
        ensure!(
            current.can_transition_to(next),
            "invalid delegation task transition from {} to {}",
            current.as_str(),
            next.as_str()
        );
        tx.execute(
            "UPDATE delegation_tasks
                SET state = ?2,
                    updated_at = ?3,
                    completed_at = CASE WHEN ?4 = 1 THEN COALESCE(completed_at, ?3) ELSE completed_at END
              WHERE delegation_task_id = ?1 AND state = ?5",
            params![
                delegation_task_id,
                next.as_str(),
                now,
                if next.is_terminal() { 1 } else { 0 },
                current.as_str(),
            ],
        )?;
        let group_id: String = tx.query_row(
            "SELECT delegation_group_id FROM delegation_tasks WHERE delegation_task_id = ?1",
            params![delegation_task_id],
            |row| row.get(0),
        )?;
        append_event(
            &tx,
            &group_id,
            Some(delegation_task_id),
            DelegationEventType::TaskStateChanged,
            &serde_json::json!({"from": current, "to": next, "state": next}),
            &now,
        )?;
        tx.commit()?;
        self.get_task(delegation_task_id)?
            .context("transitioned delegation task was not readable")
    }

    pub fn get_task(&self, delegation_task_id: &str) -> Result<Option<DelegationTaskRecord>> {
        self.db
            .conn()
            .query_row(
                "SELECT delegation_group_id, ordinal, specification_json, state,
                        attempt_count, result_json, error_summary,
                        created_at, updated_at, completed_at,
                        executor_envelope_version, executor_envelope_json
                   FROM delegation_tasks
                  WHERE delegation_task_id = ?1",
                params![delegation_task_id],
                row_to_task,
            )
            .optional()
            .map_err(Into::into)
    }
}

fn parse_datetime(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    value.parse::<DateTime<Utc>>().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, error.into())
    })
}

fn reconcile_expired_capacity(tx: &Transaction<'_>, now_ms: i64) -> Result<()> {
    tx.execute(
        "DELETE FROM delegation_capacity_waiters
          WHERE lease_expires_at_ms < ?1
             OR NOT EXISTS (
                SELECT 1 FROM delegation_tasks AS tasks
                 WHERE tasks.delegation_task_id = delegation_capacity_waiters.delegation_task_id
                   AND tasks.lease_owner_id = delegation_capacity_waiters.lease_owner_id
                   AND tasks.state IN ('leased', 'running')
                   AND tasks.lease_expires_at_ms >= ?1
             )",
        params![now_ms],
    )?;
    tx.execute(
        "DELETE FROM delegation_capacity_leases
          WHERE lease_expires_at_ms < ?1
             OR NOT EXISTS (
                SELECT 1 FROM delegation_tasks AS tasks
                 WHERE tasks.delegation_task_id = delegation_capacity_leases.delegation_task_id
                   AND tasks.lease_owner_id = delegation_capacity_leases.lease_owner_id
                   AND tasks.state = 'running'
                   AND tasks.lease_expires_at_ms >= ?1
             )",
        params![now_ms],
    )?;
    tx.execute(
        "UPDATE delegation_capacity_domains
            SET cooldown_until_ms = NULL, updated_at_ms = ?1
          WHERE cooldown_until_ms IS NOT NULL AND cooldown_until_ms <= ?1",
        params![now_ms],
    )?;
    Ok(())
}

fn apply_capacity_feedback(
    tx: &Transaction<'_>,
    authority_key: &str,
    domain_key: &str,
    feedback: DelegationCapacityFeedback,
    now_ms: i64,
) -> Result<()> {
    match feedback {
        DelegationCapacityFeedback::Healthy => {
            tx.execute(
                "UPDATE delegation_capacity_hosts
                    SET healthy_streak = healthy_streak + 1, updated_at_ms = ?2
                  WHERE authority_key = ?1",
                params![authority_key, now_ms],
            )?;
            tx.execute(
                "UPDATE delegation_capacity_hosts
                    SET target_limit = MIN(maximum_limit, target_limit + ramp_step),
                        healthy_streak = 0, demand_observed = 0, updated_at_ms = ?2
                  WHERE authority_key = ?1
                    AND demand_observed = 1
                    AND healthy_streak >= healthy_threshold",
                params![authority_key, now_ms],
            )?;
            tx.execute(
                "UPDATE delegation_capacity_domains
                    SET healthy_streak = healthy_streak + 1, updated_at_ms = ?3
                  WHERE authority_key = ?1 AND domain_key = ?2",
                params![authority_key, domain_key, now_ms],
            )?;
            tx.execute(
                "UPDATE delegation_capacity_domains
                    SET target_limit = MIN(
                            (SELECT maximum_limit FROM delegation_capacity_hosts
                              WHERE authority_key = ?1),
                            target_limit + (SELECT ramp_step FROM delegation_capacity_hosts
                              WHERE authority_key = ?1)
                        ),
                        healthy_streak = 0, demand_observed = 0, updated_at_ms = ?3
                  WHERE authority_key = ?1 AND domain_key = ?2
                    AND demand_observed = 1
                    AND healthy_streak >= (
                        SELECT healthy_threshold FROM delegation_capacity_hosts
                         WHERE authority_key = ?1
                    )",
                params![authority_key, domain_key, now_ms],
            )?;
        }
        DelegationCapacityFeedback::Neutral => {}
        DelegationCapacityFeedback::Timeout
        | DelegationCapacityFeedback::RateLimited { .. }
        | DelegationCapacityFeedback::ServiceUnavailable { .. }
        | DelegationCapacityFeedback::Overloaded { .. } => {
            let retry_after_ms = match feedback {
                DelegationCapacityFeedback::RateLimited { retry_after_ms }
                | DelegationCapacityFeedback::ServiceUnavailable { retry_after_ms }
                | DelegationCapacityFeedback::Overloaded { retry_after_ms } => retry_after_ms,
                DelegationCapacityFeedback::Timeout => None,
                DelegationCapacityFeedback::Healthy | DelegationCapacityFeedback::Neutral => {
                    unreachable!()
                }
            };
            let (minimum_limit, default_cooldown_ms): (i64, i64) = tx.query_row(
                "SELECT minimum_limit, default_cooldown_ms
                   FROM delegation_capacity_hosts WHERE authority_key = ?1",
                params![authority_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let cooldown_ms = retry_after_ms
                .filter(|duration| *duration > 0)
                .unwrap_or(default_cooldown_ms);
            tx.execute(
                "UPDATE delegation_capacity_domains
                    SET target_limit = MAX(?3, target_limit / 2),
                        healthy_streak = 0, demand_observed = 0,
                        cooldown_until_ms = ?4, updated_at_ms = ?5
                  WHERE authority_key = ?1 AND domain_key = ?2",
                params![
                    authority_key,
                    domain_key,
                    minimum_limit,
                    now_ms.saturating_add(cooldown_ms),
                    now_ms,
                ],
            )?;
        }
    }
    Ok(())
}

fn recover_expired_task_leases(
    tx: &Transaction<'_>,
    delegation_group_id: &str,
    now_ms: i64,
    now: &str,
) -> Result<()> {
    reconcile_expired_capacity(tx, now_ms)?;
    let expired = {
        let mut statement = tx.prepare(
            "SELECT delegation_task_id, state, attempt_count,
                    CAST(json_extract(specification_json, '$.max_attempts') AS INTEGER)
               FROM delegation_tasks
              WHERE delegation_group_id = ?1
                AND state IN ('leased', 'running')
                AND lease_expires_at_ms IS NOT NULL
                AND lease_expires_at_ms < ?2
              ORDER BY ordinal ASC",
        )?;
        let expired = statement
            .query_map(params![delegation_group_id, now_ms], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        expired
    };

    for (task_id, previous_state, attempt_count, max_attempts) in expired {
        if previous_state == "leased" {
            let changed = tx.execute(
                "UPDATE delegation_tasks
                    SET state = 'queued', lease_owner_id = NULL,
                        lease_expires_at_ms = NULL, updated_at = ?3
                  WHERE delegation_task_id = ?1 AND state = 'leased'
                    AND lease_expires_at_ms IS NOT NULL
                    AND lease_expires_at_ms < ?2",
                params![task_id, now_ms, now],
            )?;
            if changed == 1 {
                append_event(
                    tx,
                    delegation_group_id,
                    Some(&task_id),
                    DelegationEventType::TaskStateChanged,
                    &serde_json::json!({
                        "state": DelegationTaskState::Queued,
                        "reason": "admission_lease_expired",
                        "next_attempt_number": attempt_count.saturating_add(1),
                    }),
                    now,
                )?;
            }
            continue;
        }
        if attempt_count < max_attempts {
            let changed = tx.execute(
                "UPDATE delegation_tasks
                    SET state = 'retrying', lease_owner_id = NULL,
                        lease_expires_at_ms = NULL, updated_at = ?3
                  WHERE delegation_task_id = ?1
                    AND state IN ('leased', 'running')
                    AND lease_expires_at_ms IS NOT NULL
                    AND lease_expires_at_ms < ?2",
                params![task_id, now_ms, now],
            )?;
            if changed != 1 {
                continue;
            }
            let attempt_id = format!("{task_id}:attempt:{attempt_count}");
            tx.execute(
                "UPDATE delegation_attempts
                    SET state = 'expired', error_summary = COALESCE(error_summary, 'delegation task lease expired'),
                        last_heartbeat_at = ?2, completed_at = COALESCE(completed_at, ?2)
                  WHERE attempt_id = ?1 AND state = 'running'",
                params![attempt_id, now],
            )?;
            append_event(
                tx,
                delegation_group_id,
                Some(&task_id),
                DelegationEventType::TaskStateChanged,
                &serde_json::json!({
                    "state": DelegationTaskState::Retrying,
                    "reason": "lease_expired",
                    "attempt_id": attempt_id,
                    "attempt_number": attempt_count,
                }),
                now,
            )?;
            tx.execute(
                "UPDATE delegation_tasks SET state = 'queued', updated_at = ?2
                  WHERE delegation_task_id = ?1 AND state = 'retrying'",
                params![task_id, now],
            )?;
            append_event(
                tx,
                delegation_group_id,
                Some(&task_id),
                DelegationEventType::TaskStateChanged,
                &serde_json::json!({
                    "state": DelegationTaskState::Queued,
                    "reason": "retry_scheduled",
                    "attempt_number": attempt_count.saturating_add(1),
                }),
                now,
            )?;
        } else {
            let changed = tx.execute(
                "UPDATE delegation_tasks
                    SET state = 'failed',
                        error_summary = COALESCE(error_summary, 'delegation task lease expired after final attempt'),
                        lease_owner_id = NULL, lease_expires_at_ms = NULL,
                        updated_at = ?3, completed_at = COALESCE(completed_at, ?3)
                  WHERE delegation_task_id = ?1
                    AND state IN ('leased', 'running')
                    AND lease_expires_at_ms IS NOT NULL
                    AND lease_expires_at_ms < ?2",
                params![task_id, now_ms, now],
            )?;
            if changed == 1 {
                let attempt_id = format!("{task_id}:attempt:{attempt_count}");
                tx.execute(
                    "UPDATE delegation_attempts
                        SET state = 'expired', error_summary = COALESCE(error_summary, 'delegation task lease expired after final attempt'),
                            last_heartbeat_at = ?2, completed_at = COALESCE(completed_at, ?2)
                      WHERE attempt_id = ?1 AND state = 'running'",
                    params![attempt_id, now],
                )?;
                append_event(
                    tx,
                    delegation_group_id,
                    Some(&task_id),
                    DelegationEventType::TaskStateChanged,
                    &serde_json::json!({
                        "state": DelegationTaskState::Failed,
                        "reason": "lease_expired",
                        "attempt_id": attempt_id,
                        "attempt_number": attempt_count,
                    }),
                    now,
                )?;
            }
        }
    }
    Ok(())
}

fn append_event(
    tx: &Transaction<'_>,
    delegation_group_id: &str,
    delegation_task_id: Option<&str>,
    event_type: DelegationEventType,
    payload: &Value,
    created_at: &str,
) -> Result<()> {
    let payload_json = serde_json::to_string(payload)?;
    let changed = tx.execute(
        "INSERT INTO delegation_events (
            parent_session_id, delegation_group_id, delegation_task_id,
            event_type, payload_json, created_at
         )
         SELECT parent_session_id, delegation_group_id, ?2, ?3, ?4, ?5
           FROM delegation_groups
          WHERE delegation_group_id = ?1",
        params![
            delegation_group_id,
            delegation_task_id,
            event_type.as_str(),
            payload_json,
            created_at,
        ],
    )?;
    ensure!(changed == 1, "delegation event lost its group authority");
    Ok(())
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<DelegationEventRecord> {
    let event_type_text: String = row.get(4)?;
    let event_type = DelegationEventType::parse(&event_type_text);
    let payload_json: String = row.get(5)?;
    let payload = serde_json::from_str(&payload_json)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(5, Type::Text, error.into()))?;
    Ok(DelegationEventRecord {
        event_id: row.get(0)?,
        parent_session_id: row.get(1)?,
        delegation_group_id: row.get(2)?,
        delegation_task_id: row.get(3)?,
        event_type,
        payload,
        created_at: parse_datetime(row.get(6)?, 6)?,
    })
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<DelegationGroupRecord> {
    let state_text: String = row.get(3)?;
    let state = DelegationGroupState::parse(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            Type::Text,
            "invalid delegation group state".into(),
        )
    })?;
    let contract_json: String = row.get(4)?;
    let contract = serde_json::from_str::<DelegationGroupContract>(&contract_json)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(4, Type::Text, error.into()))?;
    let continuation_text: String = row.get(5)?;
    let parent_continuation_state = DelegationParentContinuationState::parse(&continuation_text)
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                Type::Text,
                "invalid delegation parent continuation state".into(),
            )
        })?;
    Ok(DelegationGroupRecord {
        delegation_group_id: row.get(0)?,
        parent_session_id: row.get(1)?,
        parent_tool_call_id: row.get(2)?,
        state,
        contract,
        parent_continuation_state,
        parent_continuation_id: row.get(6)?,
        synthesis_owner_id: row.get(7)?,
        synthesis_lease_expires_at_ms: row.get(8)?,
        synthesis_attempt_count: row.get::<_, i64>(9)? as usize,
        tasks: Vec::new(),
        created_at: parse_datetime(row.get(10)?, 10)?,
        updated_at: parse_datetime(row.get(11)?, 11)?,
        completed_at: row
            .get::<_, Option<String>>(12)?
            .map(|value| parse_datetime(value, 12))
            .transpose()?,
    })
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<DelegationTaskRecord> {
    let specification_json: String = row.get(2)?;
    let mut specification = serde_json::from_str::<DelegationTaskSpec>(&specification_json)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(2, Type::Text, error.into()))?;
    let envelope_version = row.get::<_, Option<i64>>(10)?;
    let envelope_json = row.get::<_, Option<String>>(11)?;
    specification.executor_envelope = match (envelope_version, envelope_json) {
        (None, None) => None,
        (Some(version), Some(value)) => {
            let mut envelope =
                serde_json::from_str::<super::model::DelegationExecutorEnvelopeV1>(&value)
                    .unwrap_or_else(|_| {
                        super::model::DelegationExecutorEnvelopeV1::invalid(
                            &specification.delegation_task_id,
                            specification.role.clone(),
                        )
                    });
            if i64::from(envelope.version) != version {
                envelope.version = 0;
            }
            Some(envelope)
        }
        _ => None,
    };
    let state_text: String = row.get(3)?;
    let state = DelegationTaskState::parse(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            Type::Text,
            "invalid delegation task state".into(),
        )
    })?;
    let result = row
        .get::<_, Option<String>>(5)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(5, Type::Text, error.into()))?;
    Ok(DelegationTaskRecord {
        delegation_group_id: row.get(0)?,
        ordinal: row.get::<_, i64>(1)? as usize,
        specification,
        state,
        attempt_count: row.get::<_, i64>(4)? as usize,
        result,
        error_summary: row.get(6)?,
        created_at: parse_datetime(row.get(7)?, 7)?,
        updated_at: parse_datetime(row.get(8)?, 8)?,
        completed_at: row
            .get::<_, Option<String>>(9)?
            .map(|value| parse_datetime(value, 9))
            .transpose()?,
    })
}
