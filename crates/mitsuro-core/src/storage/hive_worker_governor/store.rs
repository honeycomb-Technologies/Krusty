use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};

use crate::agent::{WorkerGoalEvidence, WorkerGoalEvidenceKind};
use crate::ai::types::Usage;
use crate::hive::{canonical_timestamp, parse_timezone, parse_utc_timestamp, HiveRunStatus};
use crate::storage::{
    hash_request_bytes, load_worker_with_conn, update_derived_state_for_run_in_transaction,
    Database,
};
use crate::tools::registry::PermissionMode;

use super::model::{
    BeginWorkerProviderCall, BeginWorkerProviderCallResult, FinishWorkerProviderCall,
    FinishWorkerProviderCallResult, FrozenModelPriceSnapshot, GrantWorkerGovernorOverride,
    HiveWorkerGovernorPolicy, HiveWorkerGovernorPolicyUpdate, HiveWorkerGovernorProjection,
    ProviderCallRemoteAcceptance, ProviderCallTerminalState, ReconcileUnknownProviderCall,
    RecordWorkerIdleOutcome, WorkerConversationLane, WorkerGovernorCurrencyCost,
    WorkerGovernorDailyCostProjection, WorkerGovernorDailyUsage, WorkerGovernorDecision,
    WorkerGovernorDisposition, WorkerGovernorGateReason, WorkerGovernorIdleProjection,
    WorkerGovernorLaneDecisionProjection, WorkerGovernorOverrideGrant, WorkerGovernorPolicyCas,
    WorkerIdleOutcome, WorkerProviderCall, WorkerProviderCallOutcome, WorkerRunGovernorProjection,
    WorkerRunOrigin, DEFAULT_WORKER_DAILY_CALL_LIMIT, DEFAULT_WORKER_DAILY_TOKEN_LIMIT,
    DEFAULT_WORKER_GOVERNOR_TIMEZONE, DEFAULT_WORKER_IDLE_BASE_SECS, DEFAULT_WORKER_IDLE_MAX_SECS,
    MAX_WORKER_DAILY_CALL_LIMIT, MAX_WORKER_DAILY_TOKEN_LIMIT, MAX_WORKER_GOVERNOR_CURRENCY_BYTES,
    MAX_WORKER_GOVERNOR_ID_BYTES, MAX_WORKER_GOVERNOR_LANE_BYTES, MAX_WORKER_GOVERNOR_REASON_BYTES,
    MAX_WORKER_IDLE_SECS, WORKER_GOVERNOR_RECOVERY_GRANT_TTL_SECS,
};
use super::time::{worker_local_day_window, worker_quiet_window_at};

const POLICY_COLUMNS: &str = "worker_id, revision, daily_call_limit, daily_token_limit, timezone, quiet_start_minute, quiet_end_minute, quiet_gap_policy, quiet_fold_policy, idle_base_secs, idle_max_secs, tracking_started_at, created_at, updated_at";
const OWNED_POLICY_COLUMNS: &str = "policy.worker_id, policy.revision, policy.daily_call_limit, policy.daily_token_limit, policy.timezone, policy.quiet_start_minute, policy.quiet_end_minute, policy.quiet_gap_policy, policy.quiet_fold_policy, policy.idle_base_secs, policy.idle_max_secs, policy.tracking_started_at, policy.created_at, policy.updated_at";
const CALL_COLUMNS: &str = "provider_call_id, worker_id, worker_revision, owner_user_id, session_id, group_id, run_id, run_lease_token, run_lease_epoch, run_lease_expires_at, workflow_goal_id, workflow_attempt_id, origin, lane_key, call_kind, provider_id, model_id, model_key_json, model_key_fingerprint, model_catalog_revision, permission_mode, pricing_snapshot_json, policy_revision, timezone, local_day, reserved_tokens, override_grant_id, started_at";
const OUTCOME_COLUMNS: &str = "provider_call_id, state, outcome, remote_acceptance, usage_json, usage_total_tokens, estimated_cost_microunits, unknown_reason, finished_at";
const OVERRIDE_COLUMNS: &str = "id, operation_id, worker_id, owner_user_id, bypass_unresolved_provider_call, bypass_daily_call_cap, bypass_daily_token_cap, bypass_quiet_hours, bypass_idle_backoff, reason, created_at, expires_at";
const IDLE_COLUMNS: &str =
    "lane_key, idle_streak, not_before, last_material_at, last_outcome_run_id";
const READ_PROJECTION_RESERVATION_TOKENS: u64 = 1;
const RECOVERABLE_DIRECT_DM_CALL: &str = "source_run.kind = 'worker_conversation'
    AND source_run.schedule_id IS NULL AND source_run.occurrence_id IS NULL
    AND source_run.group_id IS NULL
    AND source_run.workflow_goal_id IS NULL AND source_run.workflow_attempt_id IS NULL
    AND source_run.worker_id = call.worker_id AND source_run.session_id = call.session_id
    AND source_run.governor_origin = 'user_dm' AND source_run.governor_lane_key = 'dm'
    AND call.group_id IS NULL AND call.workflow_goal_id IS NULL
    AND call.workflow_attempt_id IS NULL AND call.origin = 'user_dm'
    AND call.lane_key = 'dm'";
const ACKNOWLEDGEABLE_PROVIDER_CALL: &str = "(
    (
        source_run.kind = 'worker_conversation'
        AND source_run.schedule_id IS NULL AND source_run.occurrence_id IS NULL
        AND source_run.group_id IS NULL
        AND source_run.workflow_goal_id IS NULL
        AND source_run.workflow_attempt_id IS NULL
        AND source_run.worker_id = call.worker_id
        AND source_run.session_id = call.session_id
        AND source_run.governor_origin = 'user_dm'
        AND source_run.governor_lane_key = 'dm'
        AND call.group_id IS NULL AND call.workflow_goal_id IS NULL
        AND call.workflow_attempt_id IS NULL AND call.origin = 'user_dm'
        AND call.lane_key = 'dm'
    )
    OR source_run.status IN ('succeeded', 'failed', 'cancelled', 'dead_letter')
)";
const UNRESOLVED_PROVIDER_CALL: &str = "call.worker_id = ?1
    AND (
        outcome.state = 'unknown'
        OR (
            outcome.provider_call_id IS NULL
            AND NOT EXISTS (
                SELECT 1 FROM hive_runs active_run
                WHERE active_run.id = call.run_id AND active_run.status = 'running'
                  AND active_run.lease_token = call.run_lease_token
                  AND active_run.lease_epoch = call.run_lease_epoch
                  AND active_run.lease_expires_at > ?2
            )
        )
    )
    AND NOT EXISTS (
        SELECT 1
        FROM hive_worker_governor_override_grants acknowledged_grant
        JOIN hive_worker_governor_override_consumptions acknowledged
          ON acknowledged.grant_id = acknowledged_grant.id
        WHERE acknowledged_grant.worker_id = call.worker_id
          AND acknowledged.provider_call_id <> call.provider_call_id
          AND acknowledged_grant.owner_user_id IS call.owner_user_id
          AND acknowledged_grant.bypass_unresolved_provider_call = 1
          AND acknowledged_grant.bypass_daily_call_cap = 0
          AND acknowledged_grant.bypass_daily_token_cap = 0
          AND acknowledged_grant.bypass_quiet_hours = 0
          AND acknowledged_grant.bypass_idle_backoff = 0
          AND acknowledged_grant.created_at > call.started_at
          AND (
              (
                  source_run.kind = 'worker_conversation'
                  AND source_run.schedule_id IS NULL
                  AND source_run.occurrence_id IS NULL
                  AND source_run.group_id IS NULL
                  AND source_run.workflow_goal_id IS NULL
                  AND source_run.workflow_attempt_id IS NULL
                  AND source_run.worker_id = call.worker_id
                  AND source_run.session_id = call.session_id
                  AND source_run.governor_origin = 'user_dm'
                  AND source_run.governor_lane_key = 'dm'
                  AND call.group_id IS NULL
                  AND call.workflow_goal_id IS NULL
                  AND call.workflow_attempt_id IS NULL
                  AND call.origin = 'user_dm' AND call.lane_key = 'dm'
              )
              OR source_run.status IN (
                  'succeeded', 'failed', 'cancelled', 'dead_letter'
              )
          )
    )";

#[derive(Debug, thiserror::Error)]
pub enum GrantWorkerGovernorRecoveryError {
    #[error("Hive Worker was not found")]
    WorkerNotFound,
    #[error("Hive Worker recovery owner mismatch")]
    OwnerMismatch,
    #[error("only an active Hive Worker can receive recovery authority")]
    WorkerInactive,
    #[error("Hive Worker has no owner-acknowledgeable unresolved provider call to recover")]
    NoEligibleUnresolved,
    #[error(
        "Hive Worker has an active background, group, Goal, Introduction, or review recovery boundary that this action cannot bypass"
    )]
    UnsupportedBoundary,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerGovernorRecoveryRunBinding {
    Unbound,
    Bound {
        run_id: String,
    },
    Rebound {
        run_id: String,
        replaced_grant_id: String,
    },
    BlockedInFlight {
        run_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerGovernorOverrideAdmission {
    Available,
    ConsumedRecoveryProvenance,
}

impl From<rusqlite::Error> for GrantWorkerGovernorRecoveryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Internal(error.into())
    }
}

/// Durable Worker spend, quiet-hour, and no-progress admission authority.
///
/// `runtime_traces` deliberately are not consulted here: they are a prunable,
/// best-effort observability projection rather than accounting evidence.
pub struct HiveWorkerGovernorStore {
    db: Database,
}

impl HiveWorkerGovernorStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn get_policy(
        &self,
        worker_id: &str,
        owner_user_id: Option<&str>,
    ) -> Result<Option<HiveWorkerGovernorPolicy>> {
        load_owned_policy(self.db.conn(), worker_id, owner_user_id)
    }

    pub fn compare_and_swap_policy(
        &self,
        worker_id: &str,
        owner_user_id: Option<&str>,
        expected_revision: u64,
        update: &HiveWorkerGovernorPolicyUpdate,
        now: DateTime<Utc>,
    ) -> Result<WorkerGovernorPolicyCas> {
        validate_id("worker id", worker_id)?;
        validate_policy_update(update)?;
        anyhow::ensure!(
            expected_revision < i64::MAX as u64,
            "Hive Worker governor policy revision is out of range"
        );
        let now = canonical_timestamp(now);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let current = load_owned_policy(&tx, worker_id, owner_user_id)?
            .ok_or_else(|| anyhow!("Hive Worker governor policy was not found for exact owner"))?;
        if current.revision != expected_revision {
            tx.commit()?;
            return Ok(WorkerGovernorPolicyCas::Conflict(current));
        }
        let next_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| anyhow!("Hive Worker governor policy revision overflow"))?;
        let changed = tx.execute(
            "UPDATE hive_worker_governor_policies
             SET revision = ?3, daily_call_limit = ?4, daily_token_limit = ?5,
                 timezone = ?6, quiet_start_minute = ?7, quiet_end_minute = ?8,
                 quiet_gap_policy = ?9, quiet_fold_policy = ?10,
                 idle_base_secs = ?11, idle_max_secs = ?12, updated_at = ?13
             WHERE worker_id = ?1 AND revision = ?2",
            params![
                worker_id,
                expected_revision,
                next_revision,
                update.daily_call_limit,
                update.daily_token_limit,
                update.timezone,
                update.quiet_start_minute,
                update.quiet_end_minute,
                update.quiet_gap_policy.as_str(),
                update.quiet_fold_policy.as_str(),
                update.idle_base_secs,
                update.idle_max_secs,
                now,
            ],
        )?;
        anyhow::ensure!(
            changed == 1,
            "Hive Worker governor policy changed during CAS"
        );
        let worker_changed: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_workers
                 WHERE id = ?1
                   AND ((?2 IS NULL AND user_id IS NULL) OR user_id = ?2)
                   AND status <> 'archived'
             )",
            params![worker_id, owner_user_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            worker_changed,
            "Hive Worker identity changed during governor policy CAS"
        );
        let updated = load_policy(&tx, worker_id)?
            .ok_or_else(|| anyhow!("updated Hive Worker governor policy disappeared"))?;
        tx.commit()?;
        Ok(WorkerGovernorPolicyCas::Updated(updated))
    }

    pub fn grant_one_call_override(
        &self,
        input: &GrantWorkerGovernorOverride,
    ) -> Result<WorkerGovernorOverrideGrant> {
        validate_override(input)?;
        let created_at = canonical_timestamp(input.created_at);
        let expires_at = canonical_timestamp(input.expires_at);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let worker = load_worker_with_conn(&tx, &input.worker_id)?
            .ok_or_else(|| anyhow!("Hive Worker was not found"))?;
        anyhow::ensure!(
            worker.user_id.as_deref() == input.owner_user_id.as_deref(),
            "Hive Worker override owner mismatch"
        );
        anyhow::ensure!(
            worker.status == crate::storage::HiveWorkerStatus::Active,
            "only an active Hive Worker can receive a provider-call override"
        );
        tx.execute(
            "INSERT INTO hive_worker_governor_override_grants (
                 id, operation_id, worker_id, owner_user_id,
                 bypass_unresolved_provider_call, bypass_daily_call_cap,
                 bypass_daily_token_cap, bypass_quiet_hours,
                 bypass_idle_backoff, reason, created_at, expires_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12
             )",
            params![
                input.id,
                input.operation_id,
                input.worker_id,
                input.owner_user_id,
                input.bypass_unresolved_provider_call,
                input.bypass_daily_call_cap,
                input.bypass_daily_token_cap,
                input.bypass_quiet_hours,
                input.bypass_idle_backoff,
                input.reason,
                created_at,
                expires_at,
            ],
        )?;
        let grant = load_override_grant(&tx, &input.id)?
            .ok_or_else(|| anyhow!("inserted Hive Worker override disappeared"))?;
        tx.commit()?;
        Ok(grant)
    }

    /// Atomically reserve one provider call before any remote request begins.
    ///
    /// A returned `AlreadyStarted` is accounting replay only. The caller must
    /// not send the remote request again because its previous acceptance may
    /// be unknowable after a crash.
    pub fn begin_provider_call(
        &self,
        input: &BeginWorkerProviderCall,
    ) -> Result<BeginWorkerProviderCallResult> {
        validate_begin(input)?;
        let started_at = canonical_timestamp(input.started_at);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;

        let model_key_json = serde_json::to_string(&input.expected_model_key)?;
        let model_key_fingerprint = hash_request_bytes(model_key_json.as_bytes());
        let pricing_snapshot_json = input
            .pricing
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        // Replay the durable reservation before consulting mutable Worker or
        // run state. Once Started exists, a later pause, model edit, or lease
        // expiry must not tempt the caller to mint a replacement call and
        // cross the uncertain remote boundary again.
        if let Some(existing) = load_provider_call(&tx, &input.provider_call_id)? {
            ensure_same_begin(
                &existing,
                input,
                &model_key_json,
                &model_key_fingerprint,
                pricing_snapshot_json.as_deref(),
            )?;
            tx.commit()?;
            return Ok(BeginWorkerProviderCallResult::AlreadyStarted(existing));
        }

        let worker = load_worker_with_conn(&tx, &input.worker_id)?
            .ok_or_else(|| anyhow!("Hive Worker was not found"))?;
        anyhow::ensure!(
            worker.user_id == input.owner_user_id,
            "Hive Worker provider-call owner mismatch"
        );
        anyhow::ensure!(
            worker.status == crate::storage::HiveWorkerStatus::Active,
            "Hive Worker is not active"
        );
        anyhow::ensure!(
            worker.revision == input.expected_worker_revision,
            "Hive Worker profile revision changed before provider-call admission"
        );
        anyhow::ensure!(
            worker.model_key.as_ref() == Some(&input.expected_model_key)
                && worker.model.as_deref() == Some(input.expected_model_key.model_id.as_str()),
            "Hive Worker provider-call model binding changed"
        );
        anyhow::ensure!(
            worker.model_catalog_revision == input.expected_model_catalog_revision,
            "Hive Worker provider-call catalog revision changed"
        );
        anyhow::ensure!(
            worker.permission_mode == input.expected_permission_mode,
            "Hive Worker provider-call permission mode changed"
        );
        validate_conversation_lane(&tx, input, worker.dm_session_id.as_deref())?;

        let run = load_run_fence(&tx, &input.run_id)?
            .ok_or_else(|| anyhow!("Hive Worker provider-call run was not found"))?;
        validate_run_fence(input, &run, &started_at)?;

        let Some(policy) = load_policy(&tx, &input.worker_id)? else {
            let decision = policy_unavailable_decision(input, &started_at)?;
            persist_run_decision(&tx, input, &decision, input.override_grant_id.as_deref())?;
            tx.commit()?;
            return Ok(BeginWorkerProviderCallResult::Gated(decision));
        };
        let day = worker_local_day_window(&policy, input.started_at)?;
        let daily = load_daily_usage(&tx, &policy, &day)?;
        let idle = load_idle_projection(&tx, &input.worker_id, &input.lane_key)?;
        let stale_or_unknown = has_unresolved_provider_call(
            &tx,
            &input.worker_id,
            &input.run_id,
            &input.provider_call_id,
            &started_at,
        )?;
        let quiet = if input.origin.is_autonomous() {
            worker_quiet_window_at(&policy, input.started_at)?
        } else {
            None
        };
        let mut decision = evaluate_decision(
            &policy,
            daily,
            idle,
            input.origin,
            input.reserved_tokens,
            stale_or_unknown,
            quiet.as_ref().map(|window| window.ends_at),
            input.started_at,
        );

        let mut consumed_override = None;
        if let Some(grant_id) = input.override_grant_id.as_deref() {
            let grant = load_override_grant(&tx, grant_id)?
                .ok_or_else(|| anyhow!("Hive Worker governor override was not found"))?;
            let override_admission = validate_override_for_begin(&tx, input, &grant, &started_at)?;
            if override_admission == WorkerGovernorOverrideAdmission::Available {
                let unresolved_covered = !decision
                    .reasons
                    .contains(&WorkerGovernorGateReason::UnresolvedProviderCall)
                    || unresolved_provider_calls_covered_by_grant(&tx, &grant, &started_at)?;
                let remaining = decision
                    .reasons
                    .iter()
                    .copied()
                    .filter(|reason| {
                        !grant_bypasses(&grant, *reason)
                            || (*reason == WorkerGovernorGateReason::UnresolvedProviderCall
                                && !unresolved_covered)
                    })
                    .collect::<Vec<_>>();
                decision.reasons = remaining;
                decision.primary_reason = decision.reasons.first().copied();
                if decision.reasons.is_empty() {
                    consumed_override = Some(grant.id.clone());
                    decision.disposition = WorkerGovernorDisposition::Allow;
                    decision.next_eligible_at = None;
                    decision.override_grant_id = Some(grant.id);
                }
            }
        }

        if !decision.reasons.is_empty() {
            persist_run_decision(&tx, input, &decision, input.override_grant_id.as_deref())?;
            tx.commit()?;
            return Ok(BeginWorkerProviderCallResult::Gated(decision));
        }

        let group_id = match &input.conversation_lane {
            WorkerConversationLane::DirectMessage => None,
            WorkerConversationLane::Group { group_id } => Some(group_id.as_str()),
        };
        tx.execute(
            "INSERT INTO hive_worker_provider_calls (
                 provider_call_id, worker_id, worker_revision, owner_user_id,
                 session_id, group_id,
                 run_id, run_lease_token, run_lease_epoch, run_lease_expires_at,
                 workflow_goal_id, workflow_attempt_id, origin, lane_key, call_kind,
                 provider_id, model_id, model_key_json, model_key_fingerprint,
                 model_catalog_revision, permission_mode, pricing_snapshot_json,
                 policy_revision, timezone, local_day, reserved_tokens,
                 override_grant_id, started_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                 ?25, ?26, ?27, ?28
             )",
            params![
                input.provider_call_id,
                input.worker_id,
                input.expected_worker_revision,
                input.owner_user_id,
                input.session_id,
                group_id,
                input.run_id,
                input.run_lease_token,
                input.run_lease_epoch,
                run.lease_expires_at,
                input.workflow_goal_id,
                input.workflow_attempt_id,
                input.origin.as_str(),
                input.lane_key,
                input.call_kind,
                input.expected_model_key.provider.storage_key(),
                input.expected_model_key.model_id,
                model_key_json,
                model_key_fingerprint,
                input.expected_model_catalog_revision,
                input.expected_permission_mode.as_str(),
                pricing_snapshot_json,
                policy.revision,
                policy.timezone,
                day.local_day,
                input.reserved_tokens,
                consumed_override,
                started_at,
            ],
        )?;
        if let Some(grant_id) = consumed_override.as_deref() {
            tx.execute(
                "INSERT INTO hive_worker_governor_override_consumptions (
                     grant_id, provider_call_id, consumed_at
                 ) VALUES (?1, ?2, ?3)",
                params![grant_id, input.provider_call_id, started_at],
            )?;
        }
        // A consumed narrow recovery grant remains immutable provenance on the
        // exact run. Later model calls carry no new override authority or
        // consumption row, but the host/run fence must retain the binding.
        persist_run_decision(&tx, input, &decision, input.override_grant_id.as_deref())?;
        let call = load_provider_call(&tx, &input.provider_call_id)?
            .ok_or_else(|| anyhow!("inserted Hive Worker provider call disappeared"))?;
        tx.commit()?;
        Ok(BeginWorkerProviderCallResult::Started(call))
    }

    pub fn finish_provider_call(
        &self,
        input: &FinishWorkerProviderCall,
    ) -> Result<FinishWorkerProviderCallResult> {
        validate_finish(input)?;
        anyhow::ensure!(
            input.state == ProviderCallTerminalState::Completed,
            "Unknown provider calls may only be recorded through fenced reconciliation"
        );
        anyhow::ensure!(
            input.unknown_reason.is_none(),
            "completed provider call cannot carry an unknown reason"
        );
        let finished_at = canonical_timestamp(input.finished_at);
        let usage_json = input
            .usage
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let usage_total_tokens = input
            .usage
            .as_ref()
            .map(|usage| u64::try_from(usage.logical_total_tokens()))
            .transpose()
            .context("provider usage exceeds the durable integer range")?;
        anyhow::ensure!(
            usage_total_tokens.is_none_or(|tokens| tokens <= i64::MAX as u64),
            "provider usage exceeds the SQLite integer range"
        );
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let call = load_provider_call(&tx, &input.provider_call_id)?
            .ok_or_else(|| anyhow!("Hive Worker provider-call Started row was not found"))?;
        anyhow::ensure!(
            call.worker_id == input.worker_id && call.run_id == input.run_id,
            "provider-call terminal identity does not match Started provenance"
        );
        anyhow::ensure!(
            parse_utc_timestamp(&finished_at)? >= parse_utc_timestamp(&call.started_at)?,
            "provider-call terminal precedes its Started row"
        );
        let candidate = WorkerProviderCallOutcome {
            provider_call_id: input.provider_call_id.clone(),
            state: input.state,
            outcome: input.outcome.clone(),
            remote_acceptance: input.remote_acceptance,
            usage: input.usage.clone(),
            usage_total_tokens,
            estimated_cost_microunits: input.estimated_cost_microunits,
            unknown_reason: None,
            finished_at,
        };
        if let Some(existing) = load_provider_call_outcome(&tx, &input.provider_call_id)? {
            anyhow::ensure!(
                same_terminal_outcome(&existing, &candidate),
                "conflicting provider-call terminal outcome already exists"
            );
            tx.commit()?;
            return Ok(FinishWorkerProviderCallResult::AlreadyRecorded(existing));
        }
        insert_provider_call_outcome(&tx, &candidate, usage_json.as_deref())?;
        tx.commit()?;
        Ok(FinishWorkerProviderCallResult::Inserted(candidate))
    }

    /// Append an Unknown terminal only after a current daemon proves that the
    /// original run lease can no longer be executing. This method never
    /// changes or deletes the Started row.
    pub fn reconcile_unknown_provider_call(
        &self,
        input: &ReconcileUnknownProviderCall,
    ) -> Result<FinishWorkerProviderCallResult> {
        validate_id("provider call id", &input.provider_call_id)?;
        validate_id("worker id", &input.worker_id)?;
        validate_id("run id", &input.run_id)?;
        validate_id("daemon lease name", &input.daemon_lease_name)?;
        validate_id("daemon owner id", &input.daemon_owner_id)?;
        validate_reason("unknown provider-call reason", &input.reason)?;
        anyhow::ensure!(
            input.daemon_fencing_token <= i64::MAX as u64,
            "daemon fencing token is out of range"
        );
        let reconciled_at = canonical_timestamp(input.reconciled_at);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let daemon_current: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_daemon_leases
                 WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3
                   AND expires_at > ?4
             )",
            params![
                input.daemon_lease_name,
                input.daemon_owner_id,
                input.daemon_fencing_token,
                reconciled_at,
            ],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            daemon_current,
            "stale daemon cannot reconcile an uncertain provider call"
        );
        let call = load_provider_call(&tx, &input.provider_call_id)?
            .ok_or_else(|| anyhow!("Hive Worker provider-call Started row was not found"))?;
        anyhow::ensure!(
            call.worker_id == input.worker_id && call.run_id == input.run_id,
            "Unknown reconciliation identity does not match Started provenance"
        );
        let candidate = WorkerProviderCallOutcome {
            provider_call_id: input.provider_call_id.clone(),
            state: ProviderCallTerminalState::Unknown,
            outcome: "executor_lost".to_string(),
            remote_acceptance: ProviderCallRemoteAcceptance::PossiblySent,
            usage: None,
            usage_total_tokens: None,
            estimated_cost_microunits: None,
            unknown_reason: Some(input.reason.clone()),
            finished_at: reconciled_at.clone(),
        };
        if let Some(existing) = load_provider_call_outcome(&tx, &input.provider_call_id)? {
            anyhow::ensure!(
                same_terminal_outcome(&existing, &candidate),
                "conflicting provider-call terminal outcome already exists"
            );
            tx.commit()?;
            return Ok(FinishWorkerProviderCallResult::AlreadyRecorded(existing));
        }

        let original_lease_expired = parse_utc_timestamp(&call.run_lease_expires_at)?
            <= parse_utc_timestamp(&reconciled_at)?;
        let original_attempt_finished: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_run_attempts
                 WHERE run_id = ?1 AND lease_token = ?2 AND lease_epoch = ?3
                   AND finished_at IS NOT NULL
             )",
            params![call.run_id, call.run_lease_token, call.run_lease_epoch],
            |row| row.get(0),
        )?;
        let original_run_still_active: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_runs
                 WHERE id = ?1 AND status = 'running' AND lease_token = ?2
                   AND lease_epoch = ?3 AND lease_expires_at > ?4
             )",
            params![
                call.run_id,
                call.run_lease_token,
                call.run_lease_epoch,
                reconciled_at,
            ],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            !original_run_still_active && (original_lease_expired || original_attempt_finished),
            "provider call remains protected by a potentially live run lease"
        );
        insert_provider_call_outcome(&tx, &candidate, None)?;
        tx.commit()?;
        Ok(FinishWorkerProviderCallResult::Inserted(candidate))
    }

    pub fn get_provider_call(&self, provider_call_id: &str) -> Result<Option<WorkerProviderCall>> {
        load_provider_call(self.db.conn(), provider_call_id)
    }

    pub fn get_provider_call_outcome(
        &self,
        provider_call_id: &str,
    ) -> Result<Option<WorkerProviderCallOutcome>> {
        load_provider_call_outcome(self.db.conn(), provider_call_id)
    }

    pub fn evaluate_worker(
        &self,
        worker_id: &str,
        owner_user_id: Option<&str>,
        origin: WorkerRunOrigin,
        lane_key: &str,
        reserved_tokens: u64,
        now: DateTime<Utc>,
    ) -> Result<WorkerGovernorDecision> {
        validate_id("worker id", worker_id)?;
        validate_lane_key(lane_key)?;
        anyhow::ensure!(
            origin != WorkerRunOrigin::ControllerChild,
            "ControllerChild must inherit a concrete root origin"
        );
        let worker = load_worker_with_conn(self.db.conn(), worker_id)?
            .ok_or_else(|| anyhow!("Hive Worker was not found"))?;
        anyhow::ensure!(
            worker.user_id.as_deref() == owner_user_id,
            "Hive Worker governor projection owner mismatch"
        );
        let Some(policy) = load_policy(self.db.conn(), worker_id)? else {
            return policy_unavailable_projection(worker_id, origin, lane_key, now);
        };
        let day = worker_local_day_window(&policy, now)?;
        let daily = load_daily_usage(self.db.conn(), &policy, &day)?;
        let idle = load_idle_projection(self.db.conn(), worker_id, lane_key)?;
        let unresolved = has_unresolved_provider_call(
            self.db.conn(),
            worker_id,
            "",
            "",
            &canonical_timestamp(now),
        )?;
        let quiet = if origin.is_autonomous() {
            worker_quiet_window_at(&policy, now)?
        } else {
            None
        };
        Ok(evaluate_decision(
            &policy,
            daily,
            idle,
            origin,
            reserved_tokens,
            unresolved,
            quiet.as_ref().map(|window| window.ends_at),
            now,
        ))
    }

    /// Load an aggregate-only Worker governor view from one SQLite snapshot.
    ///
    /// A missing result deliberately conflates a missing Worker, another
    /// owner, a missing private DM, and a non-DM/internal session binding. The
    /// HTTP boundary can therefore return one non-enumerating 404 response.
    pub fn get_worker_dm_projection(
        &self,
        worker_id: &str,
        owner_user_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Option<HiveWorkerGovernorProjection>> {
        validate_id("worker id", worker_id)?;
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Deferred)?;
        let binding = tx
            .query_row(
                "SELECT worker.revision, worker.dm_session_id
                 FROM hive_workers worker
                 JOIN sessions session ON session.id = worker.dm_session_id
                 WHERE worker.id = ?1
                   AND (
                       (?2 IS NULL
                        AND worker.user_id IS NULL
                        AND session.user_id IS NULL)
                       OR
                       (?2 IS NOT NULL
                        AND worker.user_id = ?2
                        AND session.user_id = ?2)
                   )
                   AND session.session_type = 'hive'
                   AND NOT EXISTS (
                       SELECT 1 FROM hive_group_worker_lanes lane
                       WHERE lane.session_id = session.id
                   )",
                params![worker_id, owner_user_id],
                |row| {
                    Ok((
                        u64::try_from(nonnegative(row, 0)?)
                            .map_err(|_| conversion_error(0, "Worker revision is out of range"))?,
                        row.get::<_, String>(1)?,
                    ))
                },
            )
            .optional()
            .context("loading exact-owner Hive Worker DM governor binding")?;
        let Some((worker_revision, dm_session_id)) = binding else {
            tx.commit()?;
            return Ok(None);
        };
        let policy = load_owned_policy(&tx, worker_id, owner_user_id)?
            .ok_or_else(|| anyhow!("Hive Worker governor policy is unavailable"))?;
        let day = worker_local_day_window(&policy, now)?;
        let daily = load_daily_usage(&tx, &policy, &day)?;
        let idle = load_idle_projection(&tx, worker_id, "dm")?;
        let evaluated_at = canonical_timestamp(now);
        let unresolved_started_count =
            unresolved_provider_call_count(&tx, worker_id, &evaluated_at)?;
        let response_loss_recovery_required =
            worker_governor_response_loss_recovery_required_in_transaction(
                &tx,
                worker_id,
                owner_user_id,
            )?;
        let unresolved = unresolved_started_count > 0;
        let quiet_ends_at = worker_quiet_window_at(&policy, now)?.map(|window| window.ends_at);
        let autonomous_decision = evaluate_decision(
            &policy,
            daily.clone(),
            idle.clone(),
            WorkerRunOrigin::WorkflowRollover,
            READ_PROJECTION_RESERVATION_TOKENS,
            unresolved,
            quiet_ends_at,
            now,
        );
        let foreground_decision = evaluate_decision(
            &policy,
            daily.clone(),
            idle,
            WorkerRunOrigin::UserDm,
            READ_PROJECTION_RESERVATION_TOKENS,
            unresolved,
            None,
            now,
        );
        let estimated_daily_cost = load_daily_cost_projection(&tx, &policy, &day)?;
        let projection = HiveWorkerGovernorProjection {
            schema_version: 1,
            worker_id: worker_id.to_string(),
            worker_revision,
            dm_session_id,
            evaluated_at,
            policy,
            daily,
            autonomous_dm: WorkerGovernorLaneDecisionProjection {
                origin: WorkerRunOrigin::WorkflowRollover,
                lane_key: "dm".to_string(),
                reservation_tokens: READ_PROJECTION_RESERVATION_TOKENS,
                decision: autonomous_decision,
            },
            foreground_dm: WorkerGovernorLaneDecisionProjection {
                origin: WorkerRunOrigin::UserDm,
                lane_key: "dm".to_string(),
                reservation_tokens: READ_PROJECTION_RESERVATION_TOKENS,
                decision: foreground_decision,
            },
            unresolved_started_count,
            response_loss_recovery_required,
            estimated_daily_cost,
        };
        tx.commit()?;
        Ok(Some(projection))
    }

    /// Update one autonomous lane exactly once for a succeeded run. Model
    /// prose is intentionally absent from this API; callers must supply a
    /// trusted material-effect classification.
    pub fn record_idle_outcome(
        &self,
        input: &RecordWorkerIdleOutcome,
    ) -> Result<WorkerIdleOutcome> {
        validate_id("worker id", &input.worker_id)?;
        validate_id("run id", &input.run_id)?;
        validate_lane_key(&input.lane_key)?;
        anyhow::ensure!(
            input.origin.is_autonomous(),
            "foreground runs do not mutate autonomous idle backoff"
        );
        let completed_at = canonical_timestamp(input.completed_at);
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)?;
        let worker = load_worker_with_conn(&tx, &input.worker_id)?
            .ok_or_else(|| anyhow!("Hive Worker was not found"))?;
        anyhow::ensure!(
            worker.user_id == input.owner_user_id,
            "Hive Worker idle outcome owner mismatch"
        );
        let run_matches: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1
                 FROM hive_runs run
                 JOIN hive_controllers controller ON controller.id = run.controller_id
                 WHERE run.id = ?1
                   AND COALESCE(run.worker_id, controller.worker_id) = ?2
                   AND run.status = 'succeeded' AND run.finished_at = ?3
                   AND run.governor_origin = ?4 AND run.governor_lane_key = ?5
             )",
            params![
                input.run_id,
                input.worker_id,
                completed_at,
                input.origin.as_str(),
                input.lane_key,
            ],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            run_matches,
            "idle outcome does not match an exact succeeded Worker run"
        );
        let outcome = record_idle_projection_in_transaction(
            &tx,
            &input.worker_id,
            &input.run_id,
            &input.lane_key,
            input.material,
            &completed_at,
        )?;
        tx.commit()?;
        Ok(outcome)
    }

    pub fn get_run_governor_projection(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkerRunGovernorProjection>> {
        self.db
            .conn()
            .query_row(
                "SELECT id, governor_origin, governor_lane_key,
                        governor_gate_reason, governor_next_eligible_at,
                        governor_policy_revision, governor_override_id
                 FROM hive_runs WHERE id = ?1",
                [run_id],
                map_run_projection,
            )
            .optional()
            .context("loading Worker run governor projection")
    }

    #[cfg(test)]
    pub(super) fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }
}

/// Project a trusted autonomous outcome at the same SQLite boundary that made
/// its exact run successful.
///
/// This deliberately has a three-way classification. A successful heartbeat
/// or scheduled neutral turn is structurally idle. A Worker Workflow outcome
/// is material only when its immutable typed effect/evidence classifier wrote
/// `no_progress_streak = 0`. A positive streak is idle only when all evidence
/// is structurally classifiable; an opaque runtime effect proves neither
/// material work nor idleness and remains unrecorded. Peer and group
/// conversation output, provider completion, and assistant prose likewise
/// remain unrecorded.
pub(crate) fn record_trusted_worker_idle_outcome_in_transaction(
    tx: &Transaction<'_>,
    run_id: &str,
) -> Result<Option<WorkerIdleOutcome>> {
    let binding = tx
        .query_row(
            "SELECT run.worker_id, run.kind, run.governor_origin,
                    run.governor_lane_key, run.finished_at,
                    (
                        SELECT outcome.no_progress_streak
                        FROM hive_worker_goal_outcomes outcome
                        WHERE outcome.run_id = run.id
                          AND outcome.worker_id = run.worker_id
                          AND outcome.workflow_goal_id = run.workflow_goal_id
                          AND outcome.workflow_attempt_id = run.workflow_attempt_id
                    ),
                    (
                        SELECT outcome.evidence_json
                        FROM hive_worker_goal_outcomes outcome
                        WHERE outcome.run_id = run.id
                          AND outcome.worker_id = run.worker_id
                          AND outcome.workflow_goal_id = run.workflow_goal_id
                          AND outcome.workflow_attempt_id = run.workflow_attempt_id
                    )
             FROM hive_runs run
             JOIN hive_workers worker ON worker.id = run.worker_id
             WHERE run.id = ?1 AND run.status = 'succeeded'",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((worker_id, kind, origin, lane_key, completed_at, no_progress_streak, evidence_json)) =
        binding
    else {
        return Ok(None);
    };
    let Some(origin) = WorkerRunOrigin::parse(&origin).filter(|origin| origin.is_autonomous())
    else {
        return Ok(None);
    };
    let material = match (origin, kind.as_str()) {
        (WorkerRunOrigin::Heartbeat, "worker_heartbeat")
        | (WorkerRunOrigin::Scheduled, "scheduled") => Some(false),
        (WorkerRunOrigin::WorkflowRollover, "worker_workflow") => {
            let streak = no_progress_streak.ok_or_else(|| {
                anyhow!("succeeded autonomous Worker Workflow has no exact typed outcome")
            })?;
            anyhow::ensure!(
                streak >= 0,
                "Worker Workflow no-progress streak is negative"
            );
            if streak == 0 {
                Some(true)
            } else {
                let evidence_json = evidence_json.ok_or_else(|| {
                    anyhow!("succeeded autonomous Worker Workflow has no exact typed evidence")
                })?;
                let evidence: Vec<WorkerGoalEvidence> = serde_json::from_str(&evidence_json)
                    .context("parsing trusted Worker Workflow evidence")?;
                if evidence
                    .iter()
                    .any(|item| item.kind() == WorkerGoalEvidenceKind::Runtime)
                {
                    None
                } else {
                    Some(false)
                }
            }
        }
        _ => None,
    };
    let Some(material) = material else {
        return Ok(None);
    };
    validate_lane_key(&lane_key)?;
    let outcome = record_idle_projection_in_transaction(
        tx,
        &worker_id,
        run_id,
        &lane_key,
        material,
        &completed_at,
    )?;
    Ok(Some(outcome))
}

fn record_idle_projection_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    run_id: &str,
    lane_key: &str,
    material: bool,
    completed_at: &str,
) -> Result<WorkerIdleOutcome> {
    let policy = load_policy(tx, worker_id)?
        .ok_or_else(|| anyhow!("Hive Worker governor policy was not found"))?;
    let current = load_idle_projection(tx, worker_id, lane_key)?;
    if current.last_outcome_run_id.as_deref() == Some(run_id) {
        return Ok(WorkerIdleOutcome::AlreadyRecorded(current));
    }
    if current.last_outcome_run_id.is_some() {
        let last_outcome_at: Option<String> = tx
            .query_row(
                "SELECT updated_at FROM hive_worker_idle_state
                 WHERE worker_id = ?1 AND lane_key = ?2",
                params![worker_id, lane_key],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            last_outcome_at
                .as_deref()
                .map(parse_utc_timestamp)
                .transpose()?
                .is_none_or(|last| parse_utc_timestamp(completed_at).is_ok_and(|at| at > last)),
            "stale Worker run cannot overwrite a newer idle outcome"
        );
    }

    let (idle_streak, not_before, last_material_at) = if material {
        (0_u32, None, Some(completed_at.to_string()))
    } else {
        let idle_streak = current.idle_streak.saturating_add(1);
        let delay = idle_delay_seconds(&policy, idle_streak);
        let not_before = canonical_timestamp(
            parse_utc_timestamp(completed_at)?
                .checked_add_signed(Duration::seconds(delay as i64))
                .ok_or_else(|| anyhow!("Worker idle backoff timestamp overflow"))?,
        );
        (idle_streak, Some(not_before), current.last_material_at)
    };
    tx.execute(
        "INSERT INTO hive_worker_idle_state (
             worker_id, lane_key, idle_streak, not_before, last_material_at,
             last_outcome_run_id, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(worker_id, lane_key) DO UPDATE SET
             idle_streak = excluded.idle_streak,
             not_before = excluded.not_before,
             last_material_at = excluded.last_material_at,
             last_outcome_run_id = excluded.last_outcome_run_id,
             updated_at = excluded.updated_at",
        params![
            worker_id,
            lane_key,
            idle_streak,
            not_before,
            last_material_at,
            run_id,
            completed_at,
        ],
    )?;
    Ok(WorkerIdleOutcome::Updated(load_idle_projection(
        tx, worker_id, lane_key,
    )?))
}

/// Commit one narrowly scoped recovery grant inside the daemon's idempotency
/// transaction. A second outstanding unbound grant is coalesced so repeated
/// UI actions cannot stockpile future provider authority.
#[doc(hidden)]
pub fn grant_worker_governor_recovery_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    owner_user_id: Option<&str>,
    operation_id: &str,
    now: DateTime<Utc>,
) -> std::result::Result<(WorkerGovernorOverrideGrant, bool), GrantWorkerGovernorRecoveryError> {
    validate_id("worker id", worker_id)?;
    validate_id("override operation id", operation_id)?;
    let now_text = canonical_timestamp(now);

    if let Some(existing) = load_override_grant_by_operation(tx, worker_id, operation_id)? {
        ensure_recovery_grant_shape(&existing, worker_id, owner_user_id)?;
        return Ok((existing, false));
    }

    let worker = load_worker_with_conn(tx, worker_id)?
        .ok_or(GrantWorkerGovernorRecoveryError::WorkerNotFound)?;
    if worker.user_id.as_deref() != owner_user_id {
        return Err(GrantWorkerGovernorRecoveryError::OwnerMismatch);
    }
    if worker.status != crate::storage::HiveWorkerStatus::Active {
        return Err(GrantWorkerGovernorRecoveryError::WorkerInactive);
    }
    let (unresolved_count, acknowledgeable_count) =
        unresolved_provider_call_counts(tx, worker_id, &now_text)?;
    if unresolved_count == 0 {
        return Err(GrantWorkerGovernorRecoveryError::NoEligibleUnresolved);
    }
    if unresolved_count != acknowledgeable_count {
        return Err(GrantWorkerGovernorRecoveryError::UnsupportedBoundary);
    }

    let active_unbound_sql = format!(
        "SELECT {OVERRIDE_COLUMNS}
         FROM hive_worker_governor_override_grants grant_row
         WHERE grant_row.worker_id = ?1 AND grant_row.owner_user_id IS ?2
           AND grant_row.bypass_unresolved_provider_call = 1
           AND grant_row.bypass_daily_call_cap = 0
           AND grant_row.bypass_daily_token_cap = 0
           AND grant_row.bypass_quiet_hours = 0
           AND grant_row.bypass_idle_backoff = 0
           AND grant_row.created_at <= ?3 AND grant_row.expires_at > ?3
           AND NOT EXISTS (
               SELECT 1 FROM hive_worker_governor_override_consumptions used
               WHERE used.grant_id = grant_row.id
           )
           AND (
               NOT EXISTS (
                   SELECT 1 FROM hive_runs run
                   WHERE run.governor_override_id = grant_row.id
               )
               OR (
                   (SELECT COUNT(*) FROM hive_runs run
                    WHERE run.governor_override_id = grant_row.id) = 1
                   AND EXISTS (
                       SELECT 1
                       FROM hive_runs run
                       JOIN hive_workers bound_worker
                         ON bound_worker.id = run.worker_id
                       JOIN sessions session ON session.id = run.session_id
                       JOIN hive_controllers controller
                         ON controller.id = run.controller_id
                       WHERE run.governor_override_id = grant_row.id
                         AND run.kind = 'worker_conversation'
                         AND run.status IN (
                             'queued', 'retry_wait', 'sleeping', 'awaiting_input',
                             'leased', 'running'
                         )
                         AND run.schedule_id IS NULL
                         AND run.occurrence_id IS NULL
                         AND run.group_id IS NULL
                         AND run.workflow_goal_id IS NULL
                         AND run.workflow_attempt_id IS NULL
                         AND run.governor_origin = 'user_dm'
                         AND run.governor_lane_key = 'dm'
                         AND bound_worker.id = grant_row.worker_id
                         AND bound_worker.user_id IS grant_row.owner_user_id
                         AND bound_worker.status = 'active'
                         AND bound_worker.dm_session_id = run.session_id
                         AND session.user_id IS bound_worker.user_id
                         AND session.session_type = 'hive'
                         AND controller.worker_id = bound_worker.id
                         AND controller.session_id = session.id
                         AND controller.user_id IS bound_worker.user_id
                         AND controller.status = 'active'
                         AND NOT EXISTS (
                             SELECT 1 FROM hive_worker_provider_calls call
                             WHERE call.run_id = run.id
                         )
                   )
               )
           )
         ORDER BY grant_row.created_at ASC, grant_row.id ASC
         LIMIT 1"
    );
    if let Some(existing) = tx
        .query_row(
            &active_unbound_sql,
            params![worker_id, owner_user_id, now_text],
            map_override_grant,
        )
        .optional()?
    {
        return Ok((existing, false));
    }

    let grant_id = format!(
        "worker-governor-recovery-{}",
        hash_request_bytes(
            [
                worker_id.as_bytes(),
                &[0],
                owner_user_id.unwrap_or("").as_bytes(),
                &[0],
                operation_id.as_bytes(),
            ]
            .concat(),
        )
    );
    let expires_at = canonical_timestamp(
        now.checked_add_signed(Duration::seconds(WORKER_GOVERNOR_RECOVERY_GRANT_TTL_SECS))
            .ok_or_else(|| anyhow!("Worker recovery grant expiry overflow"))?,
    );
    tx.execute(
        "INSERT INTO hive_worker_governor_override_grants (
             id, operation_id, worker_id, owner_user_id,
             bypass_unresolved_provider_call, bypass_daily_call_cap,
             bypass_daily_token_cap, bypass_quiet_hours,
             bypass_idle_backoff, reason, created_at, expires_at
         ) VALUES (?1, ?2, ?3, ?4, 1, 0, 0, 0, 0, ?5, ?6, ?7)",
        params![
            grant_id,
            operation_id,
            worker_id,
            owner_user_id,
            "Owner authorized one direct-message recovery call for unresolved provider accounting",
            now_text,
            expires_at,
        ],
    )?;
    let grant = load_override_grant(tx, &grant_id)?
        .ok_or_else(|| anyhow!("inserted Worker recovery grant disappeared"))?;
    Ok((grant, true))
}

/// Bind the oldest eligible recovery grant to one exact newly materialized
/// direct user-DM WorkerConversation run. Background, group, Goal,
/// Introduction/review, and any already-bound run fail closed with no change.
#[doc(hidden)]
pub fn bind_worker_governor_recovery_grant_to_run_in_transaction(
    tx: &Transaction<'_>,
    run_id: &str,
    now: &str,
) -> Result<Option<String>> {
    validate_id("run id", run_id)?;
    parse_utc_timestamp(now)?;
    let eligible_run = tx
        .query_row(
            "SELECT run.worker_id, worker.user_id
             FROM hive_runs run
             JOIN hive_workers worker ON worker.id = run.worker_id
             JOIN sessions session ON session.id = run.session_id
             JOIN hive_controllers controller ON controller.id = run.controller_id
             WHERE run.id = ?1 AND run.kind = 'worker_conversation'
               AND run.status = 'queued' AND run.schedule_id IS NULL
               AND run.occurrence_id IS NULL
               AND run.group_id IS NULL AND run.workflow_goal_id IS NULL
               AND run.workflow_attempt_id IS NULL
               AND run.governor_origin = 'user_dm'
               AND run.governor_lane_key = 'dm'
               AND run.governor_override_id IS NULL
               AND worker.status = 'active'
               AND worker.dm_session_id = run.session_id
               AND worker.user_id IS session.user_id
               AND controller.worker_id = worker.id
               AND controller.user_id IS worker.user_id
               AND json_valid(run.execution_context_json)
               AND json_extract(run.execution_context_json, '$.mode.kind')
                   IN ('worker_conversation_neutral', 'worker_workspace_attached')
               AND json_extract(run.execution_context_json, '$.mode.lane.kind')
                   = 'direct_message'
               AND json_extract(run.execution_context_json, '$.mode.worker_id')
                   = worker.id
               AND json_extract(run.execution_context_json, '$.mode.worker_revision')
                   = worker.revision",
            [run_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((worker_id, owner_user_id)) = eligible_run else {
        return Ok(None);
    };
    let sql = format!(
        "SELECT {OVERRIDE_COLUMNS}
         FROM hive_worker_governor_override_grants grant_row
         WHERE grant_row.worker_id = ?1 AND grant_row.owner_user_id IS ?2
           AND grant_row.bypass_unresolved_provider_call = 1
           AND grant_row.bypass_daily_call_cap = 0
           AND grant_row.bypass_daily_token_cap = 0
           AND grant_row.bypass_quiet_hours = 0
           AND grant_row.bypass_idle_backoff = 0
           AND grant_row.created_at <= ?3 AND grant_row.expires_at > ?3
           AND NOT EXISTS (
               SELECT 1 FROM hive_worker_governor_override_consumptions used
               WHERE used.grant_id = grant_row.id
           )
           AND NOT EXISTS (
               SELECT 1 FROM hive_runs referenced
               WHERE referenced.governor_override_id = grant_row.id
           )
         ORDER BY grant_row.created_at ASC, grant_row.id ASC
         LIMIT 1"
    );
    let grant = tx
        .query_row(
            &sql,
            params![worker_id, owner_user_id, now],
            map_override_grant,
        )
        .optional()?;
    let Some(grant) = grant else {
        return Ok(None);
    };
    if !unresolved_provider_calls_covered_by_grant(tx, &grant, now)? {
        return Ok(None);
    }
    let changed = tx.execute(
        "UPDATE hive_runs
         SET governor_override_id = ?2
         WHERE id = ?1 AND kind = 'worker_conversation' AND status = 'queued'
           AND schedule_id IS NULL AND occurrence_id IS NULL AND group_id IS NULL
           AND workflow_goal_id IS NULL AND workflow_attempt_id IS NULL
           AND governor_origin = 'user_dm' AND governor_lane_key = 'dm'
           AND governor_override_id IS NULL",
        params![run_id, grant.id],
    )?;
    anyhow::ensure!(
        changed == 1,
        "Worker recovery grant run binding changed during materialization"
    );
    Ok(Some(grant.id))
}

/// Move one still-unconsumed recovery grant from an exact provider-free
/// terminal direct-message run to the queued successor it just materialized.
///
/// Expiry is deliberately not consulted: preserving the binding lets the
/// ordinary refresh path replace an expired grant without losing the durable
/// recovery chain. Consumed, shared, specialized, provider-started, leased,
/// or otherwise ambiguous bindings fail closed without mutation.
#[doc(hidden)]
pub fn transfer_worker_governor_recovery_grant_to_successor_in_transaction(
    tx: &Transaction<'_>,
    predecessor_run_id: &str,
    successor_run_id: &str,
) -> Result<Option<String>> {
    validate_id("predecessor run id", predecessor_run_id)?;
    validate_id("successor run id", successor_run_id)?;
    if predecessor_run_id == successor_run_id {
        return Ok(None);
    }

    let grant_id = tx
        .query_row(
            "SELECT grant_row.id
             FROM hive_runs predecessor
             JOIN hive_runs successor ON successor.id = ?2
             JOIN hive_workers worker ON worker.id = predecessor.worker_id
             JOIN sessions session ON session.id = predecessor.session_id
             JOIN hive_controllers controller
               ON controller.id = predecessor.controller_id
             JOIN hive_worker_governor_override_grants grant_row
               ON grant_row.id = predecessor.governor_override_id
             WHERE predecessor.id = ?1
               AND predecessor.status IN ('failed', 'dead_letter', 'cancelled')
               AND predecessor.finished_at IS NOT NULL
               AND predecessor.kind = 'worker_conversation'
               AND predecessor.schedule_id IS NULL
               AND predecessor.occurrence_id IS NULL
               AND predecessor.group_id IS NULL
               AND predecessor.workflow_goal_id IS NULL
               AND predecessor.workflow_attempt_id IS NULL
               AND predecessor.governor_origin = 'user_dm'
               AND predecessor.governor_lane_key = 'dm'
               AND predecessor.objective_message_id IS NOT NULL
               AND predecessor.conversation_through_message_id
                   = predecessor.objective_message_id
               AND predecessor.response_message_id IS NULL
               AND predecessor.response_group_message_id IS NULL
               AND predecessor.response_provider_call_id IS NULL
               AND predecessor.lease_owner IS NULL
               AND predecessor.lease_token IS NULL
               AND predecessor.lease_epoch IS NULL
               AND predecessor.lease_expires_at IS NULL
               AND predecessor.heartbeat_at IS NULL
               AND successor.status = 'queued'
               AND successor.kind = 'worker_conversation'
               AND successor.worker_id = predecessor.worker_id
               AND successor.session_id = predecessor.session_id
               AND successor.controller_id = predecessor.controller_id
               AND successor.schedule_id IS NULL
               AND successor.occurrence_id IS NULL
               AND successor.group_id IS NULL
               AND successor.workflow_goal_id IS NULL
               AND successor.workflow_attempt_id IS NULL
               AND successor.governor_origin = 'user_dm'
               AND successor.governor_lane_key = 'dm'
               AND successor.objective_message_id IS NOT NULL
               AND successor.conversation_through_message_id
                   = successor.objective_message_id
               AND successor.response_message_id IS NULL
               AND successor.response_group_message_id IS NULL
               AND successor.response_provider_call_id IS NULL
               AND successor.governor_override_id IS NULL
               AND successor.lease_owner IS NULL
               AND successor.lease_token IS NULL
               AND successor.lease_epoch IS NULL
               AND successor.lease_expires_at IS NULL
               AND successor.heartbeat_at IS NULL
               AND successor.config_json = predecessor.config_json
               AND successor.execution_context_json
                   = predecessor.execution_context_json
               AND worker.status = 'active'
               AND worker.dm_session_id = predecessor.session_id
               AND worker.user_id IS session.user_id
               AND session.session_type = 'hive'
               AND controller.worker_id = worker.id
               AND controller.session_id = session.id
               AND controller.user_id IS worker.user_id
               AND controller.status = 'active'
               AND grant_row.worker_id = worker.id
               AND grant_row.owner_user_id IS worker.user_id
               AND grant_row.bypass_unresolved_provider_call = 1
               AND grant_row.bypass_daily_call_cap = 0
               AND grant_row.bypass_daily_token_cap = 0
               AND grant_row.bypass_quiet_hours = 0
               AND grant_row.bypass_idle_backoff = 0
               AND json_valid(predecessor.execution_context_json)
               AND json_extract(
                   predecessor.execution_context_json, '$.mode.kind'
               ) IN ('worker_conversation_neutral', 'worker_workspace_attached')
               AND json_extract(
                   predecessor.execution_context_json, '$.mode.lane.kind'
               ) = 'direct_message'
               AND json_extract(
                   predecessor.execution_context_json, '$.mode.worker_id'
               ) = worker.id
               AND json_extract(
                   predecessor.execution_context_json, '$.mode.worker_revision'
               ) = worker.revision
               AND (
                   (
                       json_extract(
                           predecessor.execution_context_json, '$.mode.kind'
                       ) = 'worker_conversation_neutral'
                       AND session.workspace_mode = 'neutral'
                       AND (session.working_dir IS NULL OR session.working_dir = '')
                       AND (session.project_dir IS NULL OR session.project_dir = '')
                   )
                   OR (
                       session.workspace_mode = json_extract(
                           predecessor.execution_context_json, '$.mode.workspace_mode'
                       )
                       AND session.working_dir = json_extract(
                           predecessor.execution_context_json, '$.mode.working_dir'
                       )
                       AND session.project_dir IS json_extract(
                           predecessor.execution_context_json, '$.mode.project_dir'
                       )
                   )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_governor_override_consumptions used
                   WHERE used.grant_id = grant_row.id
               )
               AND (
                   SELECT COUNT(*) FROM hive_runs referenced
                   WHERE referenced.governor_override_id = grant_row.id
               ) = 1
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_provider_calls call
                   WHERE call.run_id IN (predecessor.id, successor.id)
               )
               AND NOT EXISTS (
                   SELECT 1 FROM hive_run_attempts attempt
                   WHERE attempt.run_id IN (predecessor.id, successor.id)
                     AND attempt.finished_at IS NULL
               )",
            params![predecessor_run_id, successor_run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(grant_id) = grant_id else {
        return Ok(None);
    };

    let changed = tx.execute(
        "UPDATE hive_runs
         SET governor_override_id = CASE
             WHEN id = ?1 THEN NULL
             WHEN id = ?2 THEN ?3
         END
         WHERE (id = ?1 AND governor_override_id = ?3)
            OR (id = ?2 AND governor_override_id IS NULL)",
        params![predecessor_run_id, successor_run_id, grant_id],
    )?;
    anyhow::ensure!(
        changed == 2,
        "Worker recovery grant transfer changed after eligibility validation"
    );
    Ok(Some(grant_id))
}

/// Project the one provider-acknowledged/no-canonical-response ordinary DM
/// boundary that can be settled by the owner without minting bypass authority.
#[doc(hidden)]
pub fn worker_governor_response_loss_recovery_required_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    owner_user_id: Option<&str>,
) -> Result<bool> {
    validate_id("worker id", worker_id)?;
    tx.query_row(
        "SELECT COUNT(*) = 1
         FROM hive_runs boundary
         JOIN hive_workers worker ON worker.id = boundary.worker_id
         JOIN sessions session ON session.id = boundary.session_id
         JOIN hive_controllers controller ON controller.id = boundary.controller_id
         WHERE boundary.worker_id = ?1
           AND worker.user_id IS ?2
           AND worker.status = 'active'
           AND worker.dm_session_id = boundary.session_id
           AND session.user_id IS worker.user_id
           AND session.session_type = 'hive'
           AND controller.worker_id = worker.id
           AND controller.user_id IS worker.user_id
           AND controller.session_id = session.id
           AND controller.status = 'paused'
           AND boundary.status = 'recovery_required'
           AND boundary.kind = 'worker_conversation'
           AND boundary.schedule_id IS NULL
           AND boundary.occurrence_id IS NULL
           AND boundary.group_id IS NULL
           AND boundary.workflow_goal_id IS NULL
           AND boundary.workflow_attempt_id IS NULL
           AND boundary.governor_origin = 'user_dm'
           AND boundary.governor_lane_key = 'dm'
           AND boundary.objective_message_id IS NOT NULL
           AND boundary.conversation_through_message_id
               = boundary.objective_message_id
           AND boundary.response_message_id IS NULL
           AND boundary.response_group_message_id IS NULL
           AND boundary.response_provider_call_id IS NULL
           AND boundary.finished_at IS NULL
           AND boundary.lease_owner IS NULL
           AND boundary.lease_token IS NULL
           AND boundary.lease_epoch IS NULL
           AND boundary.lease_expires_at IS NULL
           AND boundary.heartbeat_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM hive_run_attempts open_attempt
               WHERE open_attempt.run_id = boundary.id
                 AND open_attempt.finished_at IS NULL
           )
           AND json_valid(boundary.execution_context_json)
           AND json_extract(
               boundary.execution_context_json, '$.mode.kind'
           ) IN ('worker_conversation_neutral', 'worker_workspace_attached')
           AND json_extract(
               boundary.execution_context_json, '$.mode.lane.kind'
           ) = 'direct_message'
           AND json_extract(
               boundary.execution_context_json, '$.mode.worker_id'
           ) = worker.id
           AND json_extract(
               boundary.execution_context_json, '$.mode.worker_revision'
           ) = worker.revision
           AND (
               (
                   json_extract(
                       boundary.execution_context_json, '$.mode.kind'
                   ) = 'worker_conversation_neutral'
                   AND session.workspace_mode = 'neutral'
                   AND (session.working_dir IS NULL OR session.working_dir = '')
                   AND (session.project_dir IS NULL OR session.project_dir = '')
               )
               OR (
                   session.workspace_mode = json_extract(
                       boundary.execution_context_json, '$.mode.workspace_mode'
                   )
                   AND session.working_dir = json_extract(
                       boundary.execution_context_json, '$.mode.working_dir'
                   )
                   AND session.project_dir IS json_extract(
                       boundary.execution_context_json, '$.mode.project_dir'
                   )
               )
           )
           AND (
               SELECT COUNT(*) FROM hive_runs active_boundary
               WHERE active_boundary.controller_id = controller.id
                 AND active_boundary.status = 'recovery_required'
           ) = 1
           AND (
               SELECT COUNT(*) FROM hive_worker_provider_calls only_call
               WHERE only_call.run_id = boundary.id
           ) = 1
           AND EXISTS (
               SELECT 1
               FROM hive_worker_provider_calls call
               JOIN hive_worker_provider_call_outcomes outcome
                 ON outcome.provider_call_id = call.provider_call_id
               WHERE call.run_id = boundary.id
                 AND call.worker_id = worker.id
                 AND call.worker_revision = worker.revision
                 AND call.owner_user_id IS worker.user_id
                 AND call.session_id = session.id
                 AND call.group_id IS NULL
                 AND call.workflow_goal_id IS NULL
                 AND call.workflow_attempt_id IS NULL
                 AND call.origin = 'user_dm'
                 AND call.lane_key = 'dm'
                 AND call.call_kind = 'agent_turn'
                 AND outcome.state = 'completed'
                 AND outcome.outcome = 'completed'
                 AND outcome.remote_acceptance = 'acknowledged'
           )",
        params![worker_id, owner_user_id],
        |row| row.get(0),
    )
    .context("projecting exact Worker response-loss recovery boundary")
}

/// Canonical worker-wide unresolved-provider projection for recovery seams.
/// It shares the immutable consumption acknowledgment cutoff used by provider
/// admission and the public aggregate governor projection.
#[doc(hidden)]
pub fn worker_has_unacknowledged_unresolved_provider_calls_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    now: &str,
) -> Result<bool> {
    validate_id("worker id", worker_id)?;
    parse_utc_timestamp(now)?;
    Ok(unresolved_provider_call_count(tx, worker_id, now)? > 0)
}

/// Preserve one-call recovery across a queue delay or daemon restart. An
/// active grant already bound to the exact not-started direct run is reported
/// as-is. An expired, unconsumed grant can be replaced only before any
/// provider Started row exists; a retry-waiting run is made immediately
/// claimable again. Leased/running or specialized work is never detached.
#[doc(hidden)]
pub fn refresh_worker_governor_recovery_run_binding_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    owner_user_id: Option<&str>,
    grant_id: &str,
    now: &str,
) -> Result<WorkerGovernorRecoveryRunBinding> {
    validate_id("worker id", worker_id)?;
    validate_id("override grant id", grant_id)?;
    parse_utc_timestamp(now)?;
    let grant = load_override_grant(tx, grant_id)?
        .ok_or_else(|| anyhow!("Worker recovery grant was not found"))?;
    ensure_recovery_grant_shape(&grant, worker_id, owner_user_id)?;
    let worker = load_worker_with_conn(tx, worker_id)?
        .ok_or_else(|| anyhow!("Hive Worker was not found"))?;
    anyhow::ensure!(
        worker.user_id.as_deref() == owner_user_id
            && worker.status == crate::storage::HiveWorkerStatus::Active,
        "Worker recovery grant no longer has an active exact owner"
    );
    anyhow::ensure!(
        grant.created_at.as_str() <= now && grant.expires_at.as_str() > now,
        "Worker recovery grant is not currently valid"
    );
    let consumed: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM hive_worker_governor_override_consumptions
             WHERE grant_id = ?1
         )",
        [grant_id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!consumed, "Worker recovery grant was already consumed");

    let referenced = recovery_grant_run_references(tx, grant_id)?;
    if !referenced.is_empty() {
        anyhow::ensure!(
            referenced.len() == 1
                && eligible_recovery_bound_run(tx, &referenced[0], worker_id, owner_user_id, true,)?,
            "Worker recovery grant is referenced by ineligible or multiple runs"
        );
        return Ok(WorkerGovernorRecoveryRunBinding::Bound {
            run_id: referenced[0].clone(),
        });
    }

    let expired = {
        let mut statement = tx.prepare(
            "SELECT run.id, old_grant.id
             FROM hive_runs run
             JOIN hive_worker_governor_override_grants old_grant
               ON old_grant.id = run.governor_override_id
             WHERE run.worker_id = ?1
               AND old_grant.worker_id = ?1
               AND old_grant.owner_user_id IS ?2
               AND old_grant.bypass_unresolved_provider_call = 1
               AND old_grant.bypass_daily_call_cap = 0
               AND old_grant.bypass_daily_token_cap = 0
               AND old_grant.bypass_quiet_hours = 0
               AND old_grant.bypass_idle_backoff = 0
               AND old_grant.expires_at <= ?3
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_governor_override_consumptions used
                   WHERE used.grant_id = old_grant.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_provider_calls call
                   WHERE call.run_id = run.id
               )
             ORDER BY old_grant.created_at ASC, old_grant.id ASC",
        )?;
        let expired = statement
            .query_map(params![worker_id, owner_user_id, now], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        expired
    };
    let mut eligible = Vec::new();
    let mut blocked_in_flight = Vec::new();
    for (run_id, old_grant_id) in expired {
        if eligible_recovery_bound_run(tx, &run_id, worker_id, owner_user_id, false)? {
            eligible.push((run_id, old_grant_id));
        } else if eligible_recovery_bound_run(tx, &run_id, worker_id, owner_user_id, true)? {
            blocked_in_flight.push(run_id);
        }
    }
    anyhow::ensure!(
        eligible.len() <= 1,
        "multiple expired Worker recovery grants are bound to eligible direct runs"
    );
    anyhow::ensure!(
        blocked_in_flight.len() <= 1,
        "multiple expired Worker recovery grants are bound to in-flight direct runs"
    );
    if let Some(run_id) = blocked_in_flight.into_iter().next() {
        return Ok(WorkerGovernorRecoveryRunBinding::BlockedInFlight { run_id });
    }
    let Some((run_id, replaced_grant_id)) = eligible.into_iter().next() else {
        return Ok(WorkerGovernorRecoveryRunBinding::Unbound);
    };
    let changed = tx.execute(
        "UPDATE hive_runs
         SET governor_override_id = ?2,
             status = CASE
                 WHEN status IN ('retry_wait', 'awaiting_input') THEN 'queued'
                 ELSE status
             END,
             available_at = CASE
                 WHEN status IN ('retry_wait', 'awaiting_input') THEN ?3
                 ELSE available_at
             END,
             wake_at = CASE
                 WHEN status IN ('retry_wait', 'awaiting_input') THEN NULL
                 ELSE wake_at
             END,
             last_error = CASE
                 WHEN status IN ('retry_wait', 'awaiting_input') THEN NULL
                 ELSE last_error
             END,
             governor_gate_reason = NULL, governor_next_eligible_at = NULL,
             updated_at = ?3
         WHERE id = ?1 AND governor_override_id = ?4
           AND status IN ('queued', 'retry_wait', 'sleeping', 'awaiting_input')
           AND NOT EXISTS (
               SELECT 1 FROM hive_worker_provider_calls call
               WHERE call.run_id = hive_runs.id
           )",
        params![run_id, grant_id, now, replaced_grant_id],
    )?;
    anyhow::ensure!(
        changed == 1,
        "expired Worker recovery run binding changed during replacement"
    );
    let rebound_status_raw: String = tx.query_row(
        "SELECT status FROM hive_runs WHERE id = ?1",
        [&run_id],
        |row| row.get(0),
    )?;
    let rebound_status = HiveRunStatus::parse(&rebound_status_raw).ok_or_else(|| {
        anyhow!("invalid rebound Worker recovery run status: {rebound_status_raw}")
    })?;
    update_derived_state_for_run_in_transaction(tx, &run_id, rebound_status, now)?;
    Ok(WorkerGovernorRecoveryRunBinding::Rebound {
        run_id,
        replaced_grant_id,
    })
}

fn recovery_grant_run_references(tx: &Transaction<'_>, grant_id: &str) -> Result<Vec<String>> {
    let mut statement = tx.prepare(
        "SELECT id FROM hive_runs
         WHERE governor_override_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let references = statement
        .query_map([grant_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(references)
}

fn eligible_recovery_bound_run(
    tx: &Transaction<'_>,
    run_id: &str,
    worker_id: &str,
    owner_user_id: Option<&str>,
    allow_claimed: bool,
) -> Result<bool> {
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM hive_runs run
             JOIN hive_workers worker ON worker.id = run.worker_id
             JOIN sessions session ON session.id = run.session_id
             JOIN hive_controllers controller ON controller.id = run.controller_id
             WHERE run.id = ?1 AND run.worker_id = ?2
               AND worker.user_id IS ?3 AND worker.status = 'active'
               AND worker.dm_session_id = run.session_id
               AND session.user_id IS worker.user_id
               AND session.session_type = 'hive'
               AND controller.worker_id = worker.id
               AND controller.session_id = session.id
               AND controller.user_id IS worker.user_id
               AND controller.status = 'active'
               AND run.kind = 'worker_conversation'
               AND run.status IN (
                   'queued', 'retry_wait', 'sleeping', 'awaiting_input',
                   'leased', 'running'
               )
               AND run.schedule_id IS NULL AND run.occurrence_id IS NULL
               AND run.group_id IS NULL AND run.workflow_goal_id IS NULL
               AND run.workflow_attempt_id IS NULL
               AND run.governor_origin = 'user_dm'
               AND run.governor_lane_key = 'dm'
               AND run.objective_message_id IS NOT NULL
               AND run.conversation_through_message_id = run.objective_message_id
               AND run.response_message_id IS NULL
               AND run.response_group_message_id IS NULL
               AND run.response_provider_call_id IS NULL
               AND (
                   (
                       run.status IN (
                           'queued', 'retry_wait', 'sleeping', 'awaiting_input'
                       )
                       AND run.lease_owner IS NULL AND run.lease_token IS NULL
                       AND run.lease_epoch IS NULL AND run.lease_expires_at IS NULL
                       AND run.heartbeat_at IS NULL
                       AND NOT EXISTS (
                           SELECT 1 FROM hive_run_attempts attempt
                           WHERE attempt.run_id = run.id
                             AND attempt.finished_at IS NULL
                       )
                   )
                   OR (
                       ?4 = 1 AND run.status IN ('leased', 'running')
                       AND run.lease_owner IS NOT NULL
                       AND run.lease_token IS NOT NULL
                       AND run.lease_epoch IS NOT NULL
                       AND run.lease_expires_at IS NOT NULL
                       AND EXISTS (
                           SELECT 1 FROM hive_run_attempts attempt
                           WHERE attempt.run_id = run.id
                             AND attempt.lease_token = run.lease_token
                             AND attempt.lease_epoch = run.lease_epoch
                             AND attempt.finished_at IS NULL
                       )
                   )
               )
               AND json_valid(run.execution_context_json)
               AND json_extract(run.execution_context_json, '$.mode.kind')
                   IN ('worker_conversation_neutral', 'worker_workspace_attached')
               AND json_extract(run.execution_context_json, '$.mode.lane.kind')
                   = 'direct_message'
               AND json_extract(run.execution_context_json, '$.mode.worker_id')
                   = worker.id
               AND json_extract(run.execution_context_json, '$.mode.worker_revision')
                   = worker.revision
               AND (
                   (
                       json_extract(run.execution_context_json, '$.mode.kind')
                           = 'worker_conversation_neutral'
                       AND session.workspace_mode = 'neutral'
                       AND (session.working_dir IS NULL OR session.working_dir = '')
                       AND (session.project_dir IS NULL OR session.project_dir = '')
                   )
                   OR (
                       session.workspace_mode = json_extract(
                           run.execution_context_json, '$.mode.workspace_mode'
                       )
                       AND session.working_dir = json_extract(
                           run.execution_context_json, '$.mode.working_dir'
                       )
                       AND session.project_dir IS json_extract(
                           run.execution_context_json, '$.mode.project_dir'
                       )
                   )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_provider_calls call
                   WHERE call.run_id = run.id
               )
         )",
        params![run_id, worker_id, owner_user_id, allow_claimed],
        |row| row.get(0),
    )
    .context("validating not-started direct Worker recovery run")
}

/// Validate the exact still-unbound recovery authority before the ordinary-DM
/// recovery state machine retires an ambiguous predecessor.
#[doc(hidden)]
pub(crate) fn validate_unbound_worker_governor_recovery_grant_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    owner_user_id: Option<&str>,
    grant_id: &str,
    now: &str,
) -> Result<WorkerGovernorOverrideGrant> {
    validate_id("worker id", worker_id)?;
    validate_id("override grant id", grant_id)?;
    parse_utc_timestamp(now)?;
    let grant = load_override_grant(tx, grant_id)?
        .ok_or_else(|| anyhow!("Worker recovery grant was not found"))?;
    ensure_recovery_grant_shape(&grant, worker_id, owner_user_id)?;
    let worker = load_worker_with_conn(tx, worker_id)?
        .ok_or_else(|| anyhow!("Hive Worker was not found"))?;
    anyhow::ensure!(
        worker.user_id.as_deref() == owner_user_id
            && worker.status == crate::storage::HiveWorkerStatus::Active,
        "Worker recovery grant no longer has an active exact owner"
    );
    anyhow::ensure!(
        grant.created_at.as_str() <= now && grant.expires_at.as_str() > now,
        "Worker recovery grant is not currently valid"
    );
    let (consumed, referenced): (bool, bool) = tx.query_row(
        "SELECT
             EXISTS(SELECT 1 FROM hive_worker_governor_override_consumptions
                    WHERE grant_id = ?1),
             EXISTS(SELECT 1 FROM hive_runs WHERE governor_override_id = ?1)",
        [grant_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    anyhow::ensure!(!consumed, "Worker recovery grant was already consumed");
    anyhow::ensure!(
        !referenced,
        "Worker recovery grant is already bound to a run"
    );
    Ok(grant)
}

#[doc(hidden)]
pub(crate) fn worker_governor_recovery_grant_covers_unresolved_in_transaction(
    tx: &Transaction<'_>,
    grant_id: &str,
    now: &str,
) -> Result<bool> {
    validate_id("override grant id", grant_id)?;
    parse_utc_timestamp(now)?;
    let grant = load_override_grant(tx, grant_id)?
        .ok_or_else(|| anyhow!("Worker recovery grant was not found"))?;
    unresolved_provider_calls_covered_by_grant(tx, &grant, now)
}

#[doc(hidden)]
pub(crate) fn unresolved_worker_governor_recovery_calls_belong_to_run_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    run_id: &str,
    grant_created_at: &str,
    now: &str,
) -> Result<bool> {
    validate_id("worker id", worker_id)?;
    validate_id("run id", run_id)?;
    parse_utc_timestamp(grant_created_at)?;
    parse_utc_timestamp(now)?;
    let (unresolved, _) = unresolved_provider_call_counts(tx, worker_id, now)?;
    if unresolved == 0 {
        return Ok(false);
    }
    let sql = format!(
        "SELECT COUNT(*)
         FROM hive_worker_provider_calls call
         JOIN hive_runs source_run ON source_run.id = call.run_id
         LEFT JOIN hive_worker_provider_call_outcomes outcome
           ON outcome.provider_call_id = call.provider_call_id
         WHERE {UNRESOLVED_PROVIDER_CALL}
           AND (
               ({RECOVERABLE_DIRECT_DM_CALL} AND source_run.id = ?3)
               OR source_run.status IN (
                   'succeeded', 'failed', 'cancelled', 'dead_letter'
               )
           )
           AND call.started_at < ?4"
    );
    let covered: i64 = tx.query_row(
        &sql,
        params![worker_id, now, run_id, grant_created_at],
        |row| row.get(0),
    )?;
    anyhow::ensure!(covered >= 0, "negative predecessor recovery call count");
    Ok(covered as u64 == unresolved)
}

#[derive(Debug)]
struct PersistedRunFence {
    status: HiveRunStatus,
    worker_id: Option<String>,
    session_id: Option<String>,
    lease_token: Option<String>,
    lease_epoch: Option<u64>,
    lease_expires_at: Option<String>,
    governor_origin: Option<WorkerRunOrigin>,
    governor_lane_key: Option<String>,
    governor_override_id: Option<String>,
    worker_revision: Option<u64>,
}

fn load_run_fence(tx: &Transaction<'_>, run_id: &str) -> Result<Option<PersistedRunFence>> {
    tx.query_row(
        "SELECT run.status, COALESCE(run.worker_id, controller.worker_id),
                run.session_id, run.lease_token, run.lease_epoch,
                run.lease_expires_at, run.governor_origin,
                run.governor_lane_key, run.governor_override_id,
                CAST(json_extract(
                    run.execution_context_json, '$.mode.worker_revision'
                ) AS INTEGER)
         FROM hive_runs run
         JOIN hive_controllers controller ON controller.id = run.controller_id
         WHERE run.id = ?1",
        [run_id],
        |row| {
            let status_raw: String = row.get(0)?;
            let status = HiveRunStatus::parse(&status_raw)
                .ok_or_else(|| conversion_error(0, format!("invalid run status: {status_raw}")))?;
            let origin = row
                .get::<_, Option<String>>(6)?
                .map(|value| {
                    WorkerRunOrigin::parse(&value).ok_or_else(|| {
                        conversion_error(6, format!("invalid Worker run origin: {value}"))
                    })
                })
                .transpose()?;
            Ok(PersistedRunFence {
                status,
                worker_id: row.get(1)?,
                session_id: row.get(2)?,
                lease_token: row.get(3)?,
                lease_epoch: optional_nonnegative(row, 4)?.map(|value| value as u64),
                lease_expires_at: row.get(5)?,
                governor_origin: origin,
                governor_lane_key: row.get(7)?,
                governor_override_id: row.get(8)?,
                worker_revision: optional_nonnegative(row, 9)?.map(|value| value as u64),
            })
        },
    )
    .optional()
    .context("loading Hive Worker provider-call run fence")
}

fn validate_run_fence(
    input: &BeginWorkerProviderCall,
    run: &PersistedRunFence,
    started_at: &str,
) -> Result<()> {
    anyhow::ensure!(
        run.status == HiveRunStatus::Running,
        "Hive run is not running"
    );
    anyhow::ensure!(
        run.worker_id.as_deref() == Some(input.worker_id.as_str()),
        "Hive run is not bound to the exact Worker"
    );
    anyhow::ensure!(
        run.worker_revision == Some(input.expected_worker_revision),
        "Hive run is not bound to the exact Worker revision"
    );
    anyhow::ensure!(
        run.session_id.as_deref() == Some(input.session_id.as_str()),
        "Hive run is not bound to the exact Worker conversation"
    );
    anyhow::ensure!(
        run.lease_token.as_deref() == Some(input.run_lease_token.as_str())
            && run.lease_epoch == Some(input.run_lease_epoch),
        "Hive run lease changed before provider-call admission"
    );
    let lease_expires_at = run
        .lease_expires_at
        .as_deref()
        .ok_or_else(|| anyhow!("running Hive run has no lease expiry"))?;
    anyhow::ensure!(
        parse_utc_timestamp(lease_expires_at)? > parse_utc_timestamp(started_at)?,
        "Hive run lease expired before provider-call admission"
    );
    anyhow::ensure!(
        run.governor_origin == Some(input.origin),
        "Hive run governor origin is absent or changed"
    );
    anyhow::ensure!(
        run.governor_lane_key.as_deref() == Some(input.lane_key.as_str()),
        "Hive run governor lane is absent or changed"
    );
    anyhow::ensure!(
        run.governor_override_id == input.override_grant_id,
        "Hive run governor override binding is absent or changed"
    );
    Ok(())
}

fn validate_conversation_lane(
    tx: &Transaction<'_>,
    input: &BeginWorkerProviderCall,
    worker_dm_session_id: Option<&str>,
) -> Result<()> {
    match &input.conversation_lane {
        WorkerConversationLane::DirectMessage => anyhow::ensure!(
            worker_dm_session_id == Some(input.session_id.as_str()),
            "provider call is not bound to the Worker's exact DM session"
        ),
        WorkerConversationLane::Group { group_id } => {
            validate_id("group id", group_id)?;
            let valid: bool = tx.query_row(
                "SELECT EXISTS(
                     SELECT 1
                     FROM hive_group_worker_lanes lane
                     JOIN hive_groups group_room ON group_room.id = lane.group_id
                     JOIN hive_group_members member
                       ON member.group_id = lane.group_id
                      AND member.worker_id = lane.worker_id
                     WHERE lane.group_id = ?1 AND lane.worker_id = ?2
                       AND lane.session_id = ?3 AND group_room.status = 'active'
                 )",
                params![group_id, input.worker_id, input.session_id],
                |row| row.get(0),
            )?;
            anyhow::ensure!(
                valid,
                "provider call is not bound to a validated group lane"
            );
        }
    }
    Ok(())
}

fn load_owned_policy(
    conn: &rusqlite::Connection,
    worker_id: &str,
    owner_user_id: Option<&str>,
) -> Result<Option<HiveWorkerGovernorPolicy>> {
    let sql = format!(
        "SELECT {OWNED_POLICY_COLUMNS}
         FROM hive_worker_governor_policies policy
         JOIN hive_workers worker ON worker.id = policy.worker_id
         WHERE policy.worker_id = ?1
           AND ((?2 IS NULL AND worker.user_id IS NULL) OR worker.user_id = ?2)"
    );
    conn.query_row(&sql, params![worker_id, owner_user_id], map_policy)
        .optional()
        .context("loading exact-owner Hive Worker governor policy")
}

fn load_policy(
    conn: &rusqlite::Connection,
    worker_id: &str,
) -> Result<Option<HiveWorkerGovernorPolicy>> {
    let sql = format!(
        "SELECT {POLICY_COLUMNS}
         FROM hive_worker_governor_policies WHERE worker_id = ?1"
    );
    conn.query_row(&sql, [worker_id], map_policy)
        .optional()
        .context("loading Hive Worker governor policy")
}

fn map_policy(row: &Row<'_>) -> rusqlite::Result<HiveWorkerGovernorPolicy> {
    let gap_raw: String = row.get(7)?;
    let gap = crate::hive::DstGapPolicy::parse(&gap_raw)
        .ok_or_else(|| conversion_error(7, format!("invalid DST gap policy: {gap_raw}")))?;
    let fold_raw: String = row.get(8)?;
    let fold = crate::hive::DstFoldPolicy::parse(&fold_raw)
        .ok_or_else(|| conversion_error(8, format!("invalid DST fold policy: {fold_raw}")))?;
    Ok(HiveWorkerGovernorPolicy {
        worker_id: row.get(0)?,
        revision: nonnegative(row, 1)? as u64,
        daily_call_limit: nonnegative(row, 2)? as u64,
        daily_token_limit: nonnegative(row, 3)? as u64,
        timezone: row.get(4)?,
        quiet_start_minute: optional_nonnegative(row, 5)?
            .map(u16::try_from)
            .transpose()
            .map_err(|_| conversion_error(5, "quiet start minute out of range"))?,
        quiet_end_minute: optional_nonnegative(row, 6)?
            .map(u16::try_from)
            .transpose()
            .map_err(|_| conversion_error(6, "quiet end minute out of range"))?,
        quiet_gap_policy: gap,
        quiet_fold_policy: fold,
        idle_base_secs: nonnegative(row, 9)? as u64,
        idle_max_secs: nonnegative(row, 10)? as u64,
        tracking_started_at: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn load_override_grant(
    conn: &rusqlite::Connection,
    grant_id: &str,
) -> Result<Option<WorkerGovernorOverrideGrant>> {
    let sql = format!(
        "SELECT {OVERRIDE_COLUMNS}
         FROM hive_worker_governor_override_grants WHERE id = ?1"
    );
    conn.query_row(&sql, [grant_id], map_override_grant)
        .optional()
        .context("loading Hive Worker governor override")
}

fn load_override_grant_by_operation(
    conn: &rusqlite::Connection,
    worker_id: &str,
    operation_id: &str,
) -> Result<Option<WorkerGovernorOverrideGrant>> {
    let sql = format!(
        "SELECT {OVERRIDE_COLUMNS}
         FROM hive_worker_governor_override_grants
         WHERE worker_id = ?1 AND operation_id = ?2"
    );
    conn.query_row(&sql, params![worker_id, operation_id], map_override_grant)
        .optional()
        .context("loading Worker recovery grant by operation")
}

fn ensure_recovery_grant_shape(
    grant: &WorkerGovernorOverrideGrant,
    worker_id: &str,
    owner_user_id: Option<&str>,
) -> Result<()> {
    anyhow::ensure!(
        grant.worker_id == worker_id && grant.owner_user_id.as_deref() == owner_user_id,
        "Worker recovery grant identity mismatch"
    );
    anyhow::ensure!(
        grant.bypass_unresolved_provider_call
            && !grant.bypass_daily_call_cap
            && !grant.bypass_daily_token_cap
            && !grant.bypass_quiet_hours
            && !grant.bypass_idle_backoff,
        "Worker recovery grant has broader bypass authority"
    );
    Ok(())
}

fn map_override_grant(row: &Row<'_>) -> rusqlite::Result<WorkerGovernorOverrideGrant> {
    Ok(WorkerGovernorOverrideGrant {
        id: row.get(0)?,
        operation_id: row.get(1)?,
        worker_id: row.get(2)?,
        owner_user_id: row.get(3)?,
        bypass_unresolved_provider_call: row.get(4)?,
        bypass_daily_call_cap: row.get(5)?,
        bypass_daily_token_cap: row.get(6)?,
        bypass_quiet_hours: row.get(7)?,
        bypass_idle_backoff: row.get(8)?,
        reason: row.get(9)?,
        created_at: row.get(10)?,
        expires_at: row.get(11)?,
    })
}

fn load_provider_call(
    conn: &rusqlite::Connection,
    provider_call_id: &str,
) -> Result<Option<WorkerProviderCall>> {
    let sql = format!(
        "SELECT {CALL_COLUMNS}
         FROM hive_worker_provider_calls WHERE provider_call_id = ?1"
    );
    conn.query_row(&sql, [provider_call_id], map_provider_call)
        .optional()
        .context("loading Hive Worker provider call")
}

fn map_provider_call(row: &Row<'_>) -> rusqlite::Result<WorkerProviderCall> {
    let origin_raw: String = row.get(12)?;
    let origin = WorkerRunOrigin::parse(&origin_raw)
        .ok_or_else(|| conversion_error(12, format!("invalid Worker run origin: {origin_raw}")))?;
    let permission_raw: String = row.get(20)?;
    let permission_mode = permission_raw
        .parse::<PermissionMode>()
        .map_err(|error| conversion_error(20, error))?;
    let pricing = row
        .get::<_, Option<String>>(21)?
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                conversion_error(21, format!("invalid frozen price snapshot: {error}"))
            })
        })
        .transpose()?;
    Ok(WorkerProviderCall {
        provider_call_id: row.get(0)?,
        worker_id: row.get(1)?,
        worker_revision: nonnegative(row, 2)? as u64,
        owner_user_id: row.get(3)?,
        session_id: row.get(4)?,
        group_id: row.get(5)?,
        run_id: row.get(6)?,
        run_lease_token: row.get(7)?,
        run_lease_epoch: nonnegative(row, 8)? as u64,
        run_lease_expires_at: row.get(9)?,
        workflow_goal_id: row.get(10)?,
        workflow_attempt_id: row.get(11)?,
        origin,
        lane_key: row.get(13)?,
        call_kind: row.get(14)?,
        provider_id: row.get(15)?,
        model_id: row.get(16)?,
        model_key_json: row.get(17)?,
        model_key_fingerprint: row.get(18)?,
        model_catalog_revision: row.get(19)?,
        permission_mode,
        pricing,
        policy_revision: nonnegative(row, 22)? as u64,
        timezone: row.get(23)?,
        local_day: row.get(24)?,
        reserved_tokens: nonnegative(row, 25)? as u64,
        override_grant_id: row.get(26)?,
        started_at: row.get(27)?,
    })
}

fn load_provider_call_outcome(
    conn: &rusqlite::Connection,
    provider_call_id: &str,
) -> Result<Option<WorkerProviderCallOutcome>> {
    let sql = format!(
        "SELECT {OUTCOME_COLUMNS}
         FROM hive_worker_provider_call_outcomes WHERE provider_call_id = ?1"
    );
    conn.query_row(&sql, [provider_call_id], map_provider_call_outcome)
        .optional()
        .context("loading Hive Worker provider-call outcome")
}

fn map_provider_call_outcome(row: &Row<'_>) -> rusqlite::Result<WorkerProviderCallOutcome> {
    let state_raw: String = row.get(1)?;
    let state = ProviderCallTerminalState::parse(&state_raw).ok_or_else(|| {
        conversion_error(
            1,
            format!("invalid provider-call terminal state: {state_raw}"),
        )
    })?;
    let acceptance_raw: String = row.get(3)?;
    let remote_acceptance =
        ProviderCallRemoteAcceptance::parse(&acceptance_raw).ok_or_else(|| {
            conversion_error(
                3,
                format!("invalid provider remote acceptance: {acceptance_raw}"),
            )
        })?;
    let usage = row
        .get::<_, Option<String>>(4)?
        .map(|value| {
            serde_json::from_str::<Usage>(&value)
                .map_err(|error| conversion_error(4, format!("invalid usage JSON: {error}")))
        })
        .transpose()?;
    Ok(WorkerProviderCallOutcome {
        provider_call_id: row.get(0)?,
        state,
        outcome: row.get(2)?,
        remote_acceptance,
        usage,
        usage_total_tokens: optional_nonnegative(row, 5)?.map(|value| value as u64),
        estimated_cost_microunits: optional_nonnegative(row, 6)?.map(|value| value as u64),
        unknown_reason: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

fn insert_provider_call_outcome(
    tx: &Transaction<'_>,
    outcome: &WorkerProviderCallOutcome,
    usage_json: Option<&str>,
) -> Result<()> {
    tx.execute(
        "INSERT INTO hive_worker_provider_call_outcomes (
             provider_call_id, state, outcome, remote_acceptance, usage_json,
             usage_total_tokens, estimated_cost_microunits, unknown_reason,
             finished_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            outcome.provider_call_id,
            outcome.state.as_str(),
            outcome.outcome,
            outcome.remote_acceptance.as_str(),
            usage_json,
            outcome.usage_total_tokens,
            outcome.estimated_cost_microunits,
            outcome.unknown_reason,
            outcome.finished_at,
        ],
    )?;
    Ok(())
}

fn ensure_same_begin(
    existing: &WorkerProviderCall,
    input: &BeginWorkerProviderCall,
    model_key_json: &str,
    model_key_fingerprint: &str,
    pricing_snapshot_json: Option<&str>,
) -> Result<()> {
    let group_id = match &input.conversation_lane {
        WorkerConversationLane::DirectMessage => None,
        WorkerConversationLane::Group { group_id } => Some(group_id.as_str()),
    };
    let existing_pricing = existing
        .pricing
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    anyhow::ensure!(
        existing.worker_id == input.worker_id
            && existing.worker_revision == input.expected_worker_revision
            && existing.owner_user_id == input.owner_user_id
            && existing.session_id == input.session_id
            && existing.group_id.as_deref() == group_id
            && existing.run_id == input.run_id
            && existing.run_lease_token == input.run_lease_token
            && existing.run_lease_epoch == input.run_lease_epoch
            && existing.workflow_goal_id == input.workflow_goal_id
            && existing.workflow_attempt_id == input.workflow_attempt_id
            && existing.origin == input.origin
            && existing.lane_key == input.lane_key
            && existing.call_kind == input.call_kind
            && existing.provider_id == input.expected_model_key.provider.storage_key()
            && existing.model_id == input.expected_model_key.model_id
            && existing.model_key_json == model_key_json
            && existing.model_key_fingerprint == model_key_fingerprint
            && existing.model_catalog_revision == input.expected_model_catalog_revision
            && existing.permission_mode == input.expected_permission_mode
            && existing_pricing.as_deref() == pricing_snapshot_json
            && existing.reserved_tokens == input.reserved_tokens
            && (existing.override_grant_id.is_none()
                || existing.override_grant_id == input.override_grant_id),
        "provider_call_id already names different Started provenance"
    );
    Ok(())
}

fn same_terminal_outcome(
    existing: &WorkerProviderCallOutcome,
    candidate: &WorkerProviderCallOutcome,
) -> bool {
    existing.provider_call_id == candidate.provider_call_id
        && existing.state == candidate.state
        && existing.outcome == candidate.outcome
        && existing.remote_acceptance == candidate.remote_acceptance
        && existing.usage == candidate.usage
        && existing.usage_total_tokens == candidate.usage_total_tokens
        && existing.estimated_cost_microunits == candidate.estimated_cost_microunits
        && existing.unknown_reason == candidate.unknown_reason
}

fn load_daily_usage(
    conn: &rusqlite::Connection,
    policy: &HiveWorkerGovernorPolicy,
    day: &super::time::WorkerLocalDayWindow,
) -> Result<WorkerGovernorDailyUsage> {
    let starts_at = canonical_timestamp(day.starts_at);
    let resets_at = canonical_timestamp(day.ends_at);
    let (calls_used, tokens_used): (i64, i64) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(
                    CASE
                        WHEN outcome.state = 'completed'
                         AND outcome.usage_total_tokens IS NOT NULL
                        THEN outcome.usage_total_tokens
                        ELSE call.reserved_tokens
                    END
                ), 0)
         FROM hive_worker_provider_calls call
         LEFT JOIN hive_worker_provider_call_outcomes outcome
           ON outcome.provider_call_id = call.provider_call_id
         WHERE call.worker_id = ?1
           AND call.started_at >= ?2 AND call.started_at < ?3",
        params![policy.worker_id, starts_at, resets_at],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    anyhow::ensure!(
        calls_used >= 0 && tokens_used >= 0,
        "negative Hive Worker usage projection"
    );
    Ok(WorkerGovernorDailyUsage {
        local_day: day.local_day.clone(),
        timezone: policy.timezone.clone(),
        starts_at,
        resets_at,
        calls_used: calls_used as u64,
        calls_limit: policy.daily_call_limit,
        tokens_used_or_reserved: tokens_used as u64,
        tokens_limit: policy.daily_token_limit,
    })
}

fn load_daily_cost_projection(
    conn: &rusqlite::Connection,
    policy: &HiveWorkerGovernorPolicy,
    day: &super::time::WorkerLocalDayWindow,
) -> Result<WorkerGovernorDailyCostProjection> {
    let starts_at = canonical_timestamp(day.starts_at);
    let resets_at = canonical_timestamp(day.ends_at);
    let mut statement = conn.prepare(
        "SELECT call.pricing_snapshot_json, outcome.estimated_cost_microunits
         FROM hive_worker_provider_calls call
         LEFT JOIN hive_worker_provider_call_outcomes outcome
           ON outcome.provider_call_id = call.provider_call_id
         WHERE call.worker_id = ?1
           AND call.started_at >= ?2 AND call.started_at < ?3
         ORDER BY call.provider_call_id ASC",
    )?;
    let mut rows = statement.query(params![policy.worker_id, starts_at, resets_at])?;
    let mut currencies = BTreeMap::<String, (u64, u64)>::new();
    let mut unpriced_call_count = 0_u64;
    while let Some(row) = rows.next()? {
        let pricing = row
            .get::<_, Option<String>>(0)?
            .map(|value| {
                serde_json::from_str::<FrozenModelPriceSnapshot>(&value).map_err(|error| {
                    conversion_error(0, format!("invalid frozen pricing JSON: {error}"))
                })
            })
            .transpose()?;
        let cost = optional_nonnegative(row, 1)?.map(|value| value as u64);
        let currency = pricing
            .and_then(|snapshot| snapshot.currency)
            .filter(|value| !value.is_empty());
        let (Some(currency), Some(cost)) = (currency, cost) else {
            unpriced_call_count = unpriced_call_count
                .checked_add(1)
                .ok_or_else(|| anyhow!("unpriced Worker call count overflow"))?;
            continue;
        };
        let entry = currencies.entry(currency).or_insert((0, 0));
        entry.0 = entry
            .0
            .checked_add(cost)
            .ok_or_else(|| anyhow!("estimated Worker daily cost overflow"))?;
        entry.1 = entry
            .1
            .checked_add(1)
            .ok_or_else(|| anyhow!("priced Worker call count overflow"))?;
    }
    Ok(WorkerGovernorDailyCostProjection {
        local_day: day.local_day.clone(),
        timezone: policy.timezone.clone(),
        starts_at,
        resets_at,
        by_currency: currencies
            .into_iter()
            .map(
                |(currency, (estimated_cost_microunits, priced_call_count))| {
                    WorkerGovernorCurrencyCost {
                        currency,
                        estimated_cost_microunits: estimated_cost_microunits.to_string(),
                        priced_call_count,
                    }
                },
            )
            .collect(),
        unpriced_call_count,
    })
}

fn load_idle_projection(
    conn: &rusqlite::Connection,
    worker_id: &str,
    lane_key: &str,
) -> Result<WorkerGovernorIdleProjection> {
    let sql = format!(
        "SELECT {IDLE_COLUMNS}
         FROM hive_worker_idle_state WHERE worker_id = ?1 AND lane_key = ?2"
    );
    let value = conn
        .query_row(&sql, params![worker_id, lane_key], |row| {
            Ok(WorkerGovernorIdleProjection {
                lane_key: row.get(0)?,
                idle_streak: u32::try_from(nonnegative(row, 1)?)
                    .map_err(|_| conversion_error(1, "Worker idle streak is out of range"))?,
                not_before: row.get(2)?,
                last_material_at: row.get(3)?,
                last_outcome_run_id: row.get(4)?,
            })
        })
        .optional()?;
    Ok(value.unwrap_or_else(|| WorkerGovernorIdleProjection {
        lane_key: lane_key.to_string(),
        idle_streak: 0,
        not_before: None,
        last_material_at: None,
        last_outcome_run_id: None,
    }))
}

fn has_unresolved_provider_call(
    conn: &rusqlite::Connection,
    worker_id: &str,
    _current_run_id: &str,
    _current_provider_call_id: &str,
    now: &str,
) -> Result<bool> {
    Ok(unresolved_provider_call_count(conn, worker_id, now)? > 0)
}

fn unresolved_provider_call_count(
    conn: &rusqlite::Connection,
    worker_id: &str,
    now: &str,
) -> Result<u64> {
    Ok(unresolved_provider_call_counts(conn, worker_id, now)?.0)
}

fn unresolved_provider_call_counts(
    conn: &rusqlite::Connection,
    worker_id: &str,
    now: &str,
) -> Result<(u64, u64)> {
    let sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN {ACKNOWLEDGEABLE_PROVIDER_CALL}
                                  THEN 1 ELSE 0 END), 0)
         FROM hive_worker_provider_calls call
         JOIN hive_runs source_run ON source_run.id = call.run_id
         LEFT JOIN hive_worker_provider_call_outcomes outcome
           ON outcome.provider_call_id = call.provider_call_id
         WHERE {UNRESOLVED_PROVIDER_CALL}"
    );
    let (count, recoverable): (i64, i64) = conn
        .query_row(&sql, params![worker_id, now], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .context("counting unresolved Hive Worker provider calls")?;
    anyhow::ensure!(
        count >= 0 && recoverable >= 0 && recoverable <= count,
        "invalid unresolved Worker call counts"
    );
    Ok((count as u64, recoverable as u64))
}

fn unresolved_provider_calls_covered_by_grant(
    conn: &rusqlite::Connection,
    grant: &WorkerGovernorOverrideGrant,
    now: &str,
) -> Result<bool> {
    ensure_recovery_grant_shape(grant, &grant.worker_id, grant.owner_user_id.as_deref())?;
    let (unresolved, _) = unresolved_provider_call_counts(conn, &grant.worker_id, now)?;
    if unresolved == 0 {
        return Ok(false);
    }
    let sql = format!(
        "SELECT COUNT(*)
         FROM hive_worker_provider_calls call
         JOIN hive_runs source_run ON source_run.id = call.run_id
         LEFT JOIN hive_worker_provider_call_outcomes outcome
           ON outcome.provider_call_id = call.provider_call_id
         WHERE {UNRESOLVED_PROVIDER_CALL}
           AND {ACKNOWLEDGEABLE_PROVIDER_CALL}
           AND call.started_at < ?3"
    );
    let covered: i64 = conn.query_row(
        &sql,
        params![grant.worker_id, now, grant.created_at],
        |row| row.get(0),
    )?;
    anyhow::ensure!(covered >= 0, "negative covered Worker call count");
    Ok(covered as u64 == unresolved)
}

#[allow(clippy::too_many_arguments)]
fn evaluate_decision(
    policy: &HiveWorkerGovernorPolicy,
    daily: WorkerGovernorDailyUsage,
    idle: WorkerGovernorIdleProjection,
    origin: WorkerRunOrigin,
    reserved_tokens: u64,
    unresolved: bool,
    quiet_ends_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> WorkerGovernorDecision {
    let mut reasons = Vec::new();
    let mut next_candidates = Vec::new();
    if unresolved {
        reasons.push(WorkerGovernorGateReason::UnresolvedProviderCall);
    }
    if daily.calls_used >= daily.calls_limit {
        reasons.push(WorkerGovernorGateReason::DailyCallCapReached);
        next_candidates.push(daily.resets_at.clone());
    }
    if daily
        .tokens_used_or_reserved
        .saturating_add(reserved_tokens)
        > daily.tokens_limit
    {
        reasons.push(WorkerGovernorGateReason::DailyTokenCapReached);
        next_candidates.push(daily.resets_at.clone());
    }
    if origin.is_autonomous() {
        if let Some(quiet_ends_at) = quiet_ends_at {
            reasons.push(WorkerGovernorGateReason::QuietHours);
            next_candidates.push(canonical_timestamp(quiet_ends_at));
        }
        if idle.not_before.as_deref().is_some_and(|value| {
            parse_utc_timestamp(value).is_ok_and(|not_before| not_before > now)
        }) {
            reasons.push(WorkerGovernorGateReason::IdleBackoff);
            if let Some(not_before) = idle.not_before.clone() {
                next_candidates.push(not_before);
            }
        }
    }
    let primary_reason = reasons.first().copied();
    let disposition = if reasons.iter().any(|reason| {
        matches!(
            reason,
            WorkerGovernorGateReason::PolicyUnavailable
                | WorkerGovernorGateReason::UnresolvedProviderCall
                | WorkerGovernorGateReason::DailyCallCapReached
                | WorkerGovernorGateReason::DailyTokenCapReached
        )
    }) {
        WorkerGovernorDisposition::Deny
    } else if reasons.is_empty() {
        WorkerGovernorDisposition::Allow
    } else {
        WorkerGovernorDisposition::Defer
    };
    let next_eligible_at = if reasons.contains(&WorkerGovernorGateReason::UnresolvedProviderCall) {
        None
    } else {
        next_candidates.into_iter().max()
    };
    WorkerGovernorDecision {
        disposition,
        primary_reason,
        reasons,
        evaluated_at: canonical_timestamp(now),
        next_eligible_at,
        policy_revision: policy.revision,
        tracking_started_at: policy.tracking_started_at.clone(),
        daily,
        idle,
        override_grant_id: None,
    }
}

fn policy_unavailable_decision(
    input: &BeginWorkerProviderCall,
    started_at: &str,
) -> Result<WorkerGovernorDecision> {
    policy_unavailable_projection(
        &input.worker_id,
        input.origin,
        &input.lane_key,
        parse_utc_timestamp(started_at)?,
    )
}

fn policy_unavailable_projection(
    worker_id: &str,
    _origin: WorkerRunOrigin,
    lane_key: &str,
    now: DateTime<Utc>,
) -> Result<WorkerGovernorDecision> {
    let now_string = canonical_timestamp(now);
    let policy = HiveWorkerGovernorPolicy {
        worker_id: worker_id.to_string(),
        revision: 0,
        daily_call_limit: DEFAULT_WORKER_DAILY_CALL_LIMIT,
        daily_token_limit: DEFAULT_WORKER_DAILY_TOKEN_LIMIT,
        timezone: DEFAULT_WORKER_GOVERNOR_TIMEZONE.to_string(),
        quiet_start_minute: None,
        quiet_end_minute: None,
        quiet_gap_policy: crate::hive::DstGapPolicy::ShiftForward,
        quiet_fold_policy: crate::hive::DstFoldPolicy::First,
        idle_base_secs: DEFAULT_WORKER_IDLE_BASE_SECS,
        idle_max_secs: DEFAULT_WORKER_IDLE_MAX_SECS,
        tracking_started_at: now_string.clone(),
        created_at: now_string.clone(),
        updated_at: now_string.clone(),
    };
    let day = worker_local_day_window(&policy, now)?;
    Ok(WorkerGovernorDecision {
        disposition: WorkerGovernorDisposition::Deny,
        primary_reason: Some(WorkerGovernorGateReason::PolicyUnavailable),
        reasons: vec![WorkerGovernorGateReason::PolicyUnavailable],
        evaluated_at: now_string.clone(),
        next_eligible_at: None,
        policy_revision: 0,
        tracking_started_at: now_string,
        daily: WorkerGovernorDailyUsage {
            local_day: day.local_day,
            timezone: policy.timezone,
            starts_at: canonical_timestamp(day.starts_at),
            resets_at: canonical_timestamp(day.ends_at),
            calls_used: 0,
            calls_limit: policy.daily_call_limit,
            tokens_used_or_reserved: 0,
            tokens_limit: policy.daily_token_limit,
        },
        idle: WorkerGovernorIdleProjection {
            lane_key: lane_key.to_string(),
            idle_streak: 0,
            not_before: None,
            last_material_at: None,
            last_outcome_run_id: None,
        },
        override_grant_id: None,
    })
}

fn persist_run_decision(
    tx: &Transaction<'_>,
    input: &BeginWorkerProviderCall,
    decision: &WorkerGovernorDecision,
    override_grant_id: Option<&str>,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE hive_runs
         SET governor_gate_reason = ?6, governor_next_eligible_at = ?7,
             governor_policy_revision = ?8, governor_override_id = ?9
         WHERE id = ?1 AND status = 'running' AND lease_token = ?2
           AND lease_epoch = ?3 AND governor_origin = ?4
           AND governor_lane_key = ?5",
        params![
            input.run_id,
            input.run_lease_token,
            input.run_lease_epoch,
            input.origin.as_str(),
            input.lane_key,
            decision
                .primary_reason
                .map(WorkerGovernorGateReason::as_str),
            decision.next_eligible_at,
            decision.policy_revision,
            override_grant_id,
        ],
    )?;
    anyhow::ensure!(
        changed == 1,
        "Hive run fence changed while persisting governor decision"
    );
    Ok(())
}

fn map_run_projection(row: &Row<'_>) -> rusqlite::Result<WorkerRunGovernorProjection> {
    let origin = row
        .get::<_, Option<String>>(1)?
        .map(|value| {
            WorkerRunOrigin::parse(&value)
                .ok_or_else(|| conversion_error(1, format!("invalid run origin: {value}")))
        })
        .transpose()?;
    let gate_reason = row
        .get::<_, Option<String>>(3)?
        .map(|value| {
            WorkerGovernorGateReason::parse(&value).ok_or_else(|| {
                conversion_error(3, format!("invalid governor gate reason: {value}"))
            })
        })
        .transpose()?;
    Ok(WorkerRunGovernorProjection {
        run_id: row.get(0)?,
        origin,
        lane_key: row.get(2)?,
        gate_reason,
        next_eligible_at: row.get(4)?,
        policy_revision: optional_nonnegative(row, 5)?.map(|value| value as u64),
        override_grant_id: row.get(6)?,
    })
}

fn validate_policy_update(update: &HiveWorkerGovernorPolicyUpdate) -> Result<()> {
    anyhow::ensure!(
        (1..=MAX_WORKER_DAILY_CALL_LIMIT).contains(&update.daily_call_limit),
        "daily Worker call limit is out of range"
    );
    anyhow::ensure!(
        (1..=MAX_WORKER_DAILY_TOKEN_LIMIT).contains(&update.daily_token_limit),
        "daily Worker token limit is out of range"
    );
    anyhow::ensure!(
        !update.timezone.trim().is_empty() && update.timezone.len() <= 128,
        "Worker governor timezone is empty or too long"
    );
    parse_timezone(&update.timezone)?;
    match (update.quiet_start_minute, update.quiet_end_minute) {
        (None, None) => {}
        (Some(start), Some(end)) => {
            anyhow::ensure!(start < 1_440 && end < 1_440, "quiet minute is out of range");
            anyhow::ensure!(
                start != end,
                "quiet hours cannot cover an ambiguous full day"
            );
        }
        _ => anyhow::bail!("quiet start and end must both be present or absent"),
    }
    anyhow::ensure!(
        (1..=MAX_WORKER_IDLE_SECS).contains(&update.idle_base_secs),
        "Worker idle base is out of range"
    );
    anyhow::ensure!(
        update.idle_max_secs >= update.idle_base_secs
            && update.idle_max_secs <= MAX_WORKER_IDLE_SECS,
        "Worker idle maximum is out of range"
    );
    Ok(())
}

fn validate_begin(input: &BeginWorkerProviderCall) -> Result<()> {
    for (label, value) in [
        ("provider call id", input.provider_call_id.as_str()),
        ("worker id", input.worker_id.as_str()),
        ("session id", input.session_id.as_str()),
        ("run id", input.run_id.as_str()),
        ("run lease token", input.run_lease_token.as_str()),
        ("call kind", input.call_kind.as_str()),
    ] {
        validate_id(label, value)?;
    }
    validate_lane_key(&input.lane_key)?;
    if let Some(value) = input.workflow_goal_id.as_deref() {
        validate_id("Workflow goal id", value)?;
    }
    if let Some(value) = input.workflow_attempt_id.as_deref() {
        validate_id("Workflow attempt id", value)?;
    }
    if let Some(value) = input.override_grant_id.as_deref() {
        validate_id("override grant id", value)?;
    }
    anyhow::ensure!(
        input.origin != WorkerRunOrigin::ControllerChild,
        "ControllerChild must inherit a concrete root origin"
    );
    anyhow::ensure!(
        input.run_lease_epoch <= i64::MAX as u64,
        "Hive run lease epoch is out of range"
    );
    anyhow::ensure!(
        (1..=i64::MAX as u64).contains(&input.expected_worker_revision),
        "Hive Worker revision is out of range"
    );
    anyhow::ensure!(
        !input.expected_model_key.model_id.trim().is_empty()
            && input.expected_model_key.model_id.len() <= 512,
        "provider-call model id is empty or too long"
    );
    anyhow::ensure!(
        (1..=MAX_WORKER_DAILY_TOKEN_LIMIT).contains(&input.reserved_tokens),
        "provider-call token reservation is out of range"
    );
    if let Some(pricing) = &input.pricing {
        validate_pricing(pricing)?;
    }
    Ok(())
}

fn validate_finish(input: &FinishWorkerProviderCall) -> Result<()> {
    validate_id("provider call id", &input.provider_call_id)?;
    validate_id("worker id", &input.worker_id)?;
    validate_id("run id", &input.run_id)?;
    validate_reason("provider-call outcome", &input.outcome)?;
    if let Some(reason) = input.unknown_reason.as_deref() {
        validate_reason("provider-call unknown reason", reason)?;
    }
    if let Some(cost) = input.estimated_cost_microunits {
        anyhow::ensure!(
            cost <= i64::MAX as u64,
            "provider-call cost is out of range"
        );
    }
    Ok(())
}

fn validate_override(input: &GrantWorkerGovernorOverride) -> Result<()> {
    validate_id("override id", &input.id)?;
    validate_id("override operation id", &input.operation_id)?;
    validate_id("worker id", &input.worker_id)?;
    validate_reason("override reason", &input.reason)?;
    anyhow::ensure!(
        input.bypass_unresolved_provider_call
            || input.bypass_daily_call_cap
            || input.bypass_daily_token_cap
            || input.bypass_quiet_hours
            || input.bypass_idle_backoff,
        "Worker governor override grants no bypass"
    );
    anyhow::ensure!(
        input.expires_at > input.created_at,
        "Worker governor override expires before creation"
    );
    anyhow::ensure!(
        input.expires_at.signed_duration_since(input.created_at) <= Duration::hours(24),
        "Worker governor override may live for at most 24 hours"
    );
    Ok(())
}

fn validate_pricing(pricing: &FrozenModelPriceSnapshot) -> Result<()> {
    anyhow::ensure!(
        !pricing.catalog_source.trim().is_empty()
            && pricing.catalog_source.len() <= MAX_WORKER_GOVERNOR_ID_BYTES,
        "price catalog source is empty or too long"
    );
    if let Some(currency) = pricing.currency.as_deref() {
        anyhow::ensure!(
            !currency.trim().is_empty() && currency.len() <= MAX_WORKER_GOVERNOR_CURRENCY_BYTES,
            "price currency is empty or too long"
        );
    }
    for price in [
        pricing.input_microunits_per_million,
        pricing.output_microunits_per_million,
        pricing.cache_creation_microunits_per_million,
        pricing.cache_read_microunits_per_million,
    ]
    .into_iter()
    .flatten()
    {
        anyhow::ensure!(
            price <= i64::MAX as u64,
            "frozen model price is out of range"
        );
    }
    Ok(())
}

fn validate_override_for_begin(
    tx: &Transaction<'_>,
    input: &BeginWorkerProviderCall,
    grant: &WorkerGovernorOverrideGrant,
    started_at: &str,
) -> Result<WorkerGovernorOverrideAdmission> {
    anyhow::ensure!(
        grant.worker_id == input.worker_id && grant.owner_user_id == input.owner_user_id,
        "Worker governor override identity mismatch"
    );
    let references = tx.query_row(
        "SELECT COUNT(*) FROM hive_runs WHERE governor_override_id = ?1",
        [&grant.id],
        |row| row.get::<_, i64>(0),
    )?;
    anyhow::ensure!(
        references == 1
            && tx.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM hive_runs
                     WHERE id = ?1 AND governor_override_id = ?2
                 )",
                params![input.run_id, grant.id],
                |row| row.get::<_, bool>(0),
            )?,
        "Worker governor override is not bound to exactly this run"
    );
    let consumed_run_id = tx
        .query_row(
            "SELECT call.run_id
             FROM hive_worker_governor_override_consumptions used
             LEFT JOIN hive_worker_provider_calls call
               ON call.provider_call_id = used.provider_call_id
             WHERE used.grant_id = ?1",
            [&grant.id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    if let Some(consumed_run_id) = consumed_run_id {
        let consumed_run_id = consumed_run_id.ok_or_else(|| {
            anyhow!("Worker governor override consumption lost its provider call")
        })?;
        ensure_recovery_grant_shape(grant, &input.worker_id, input.owner_user_id.as_deref())?;
        anyhow::ensure!(
            consumed_run_id == input.run_id,
            "consumed Worker recovery grant belongs to a different run"
        );
        return Ok(WorkerGovernorOverrideAdmission::ConsumedRecoveryProvenance);
    }

    let started_at = parse_utc_timestamp(started_at)?;
    anyhow::ensure!(
        parse_utc_timestamp(&grant.created_at)? <= started_at
            && parse_utc_timestamp(&grant.expires_at)? > started_at,
        "Worker governor override is not currently valid"
    );
    Ok(WorkerGovernorOverrideAdmission::Available)
}

fn grant_bypasses(grant: &WorkerGovernorOverrideGrant, reason: WorkerGovernorGateReason) -> bool {
    match reason {
        WorkerGovernorGateReason::PolicyUnavailable => false,
        WorkerGovernorGateReason::UnresolvedProviderCall => grant.bypass_unresolved_provider_call,
        WorkerGovernorGateReason::DailyCallCapReached => grant.bypass_daily_call_cap,
        WorkerGovernorGateReason::DailyTokenCapReached => grant.bypass_daily_token_cap,
        WorkerGovernorGateReason::QuietHours => grant.bypass_quiet_hours,
        WorkerGovernorGateReason::IdleBackoff => grant.bypass_idle_backoff,
    }
}

fn idle_delay_seconds(policy: &HiveWorkerGovernorPolicy, idle_streak: u32) -> u64 {
    if idle_streak == 0 {
        return 0;
    }
    let exponent = idle_streak.saturating_sub(1).min(63);
    policy
        .idle_base_secs
        .saturating_mul(1_u64 << exponent)
        .min(policy.idle_max_secs)
}

fn validate_id(label: &str, value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.trim().is_empty() && value.len() <= MAX_WORKER_GOVERNOR_ID_BYTES,
        "{label} is empty or too long"
    );
    Ok(())
}

fn validate_lane_key(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.trim().is_empty() && value.len() <= MAX_WORKER_GOVERNOR_LANE_BYTES,
        "Worker governor lane key is empty or too long"
    );
    Ok(())
}

fn validate_reason(label: &str, value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.trim().is_empty() && value.len() <= MAX_WORKER_GOVERNOR_REASON_BYTES,
        "{label} is empty or too long"
    );
    Ok(())
}

fn nonnegative(row: &Row<'_>, index: usize) -> rusqlite::Result<i64> {
    let value = row.get::<_, i64>(index)?;
    if value < 0 {
        return Err(conversion_error(index, "negative integer"));
    }
    Ok(value)
}

fn optional_nonnegative(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<i64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            if value < 0 {
                Err(conversion_error(index, "negative integer"))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn conversion_error(index: usize, message: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(IoError::new(ErrorKind::InvalidData, message.to_string())),
    )
}
