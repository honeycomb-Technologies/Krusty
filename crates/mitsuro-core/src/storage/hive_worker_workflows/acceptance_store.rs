//! SQLite facade for explicit owner resolution of a Worker Goal acceptance.
//!
//! This path never invokes a model and never accepts caller-selected Workflow
//! identities. The acceptance run selects one immutable candidate; the
//! exact local-or-authenticated owner supplies only an accept/reject decision
//! and bounded criterion evidence.

use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::Deserialize;
use thiserror::Error;

use crate::agent::{
    WorkerGoalAttemptOutcome, WorkerGoalEffectSummary, WorkerGoalEvidence,
    WorkerGoalOutcomeCommitInput, WorkerGoalOutcomeCounters, MAX_WORKER_GOAL_EVIDENCE_ITEMS,
};
use crate::hive::canonical_timestamp;
use crate::storage::{hash_request_bytes, Database};
use crate::workflow::{
    UserGoalCriterionAcceptance, UserGoalCriterionDecision, UserWorkerGoalAcceptanceDecision,
    UserWorkerGoalAcceptanceRequest, WorkflowAcceptanceSpecV1, WorkflowAcceptanceValidationError,
};

use super::acceptance::{
    exact_result_payload, WorkerGoalAcceptanceAuthority, WorkerGoalAcceptanceCandidateRecord,
    WorkerGoalAcceptanceCandidateState, WorkerGoalAcceptanceCommitDisposition,
    WorkerGoalAcceptanceContractV1, WorkerGoalAcceptanceResolution,
    WorkerGoalAcceptanceResultRecord, WorkerGoalAcceptanceSourceSummary,
    WorkerGoalCriterionAcceptanceSpecV1, WORKER_GOAL_ACCEPTANCE_CONTRACT_VERSION,
};
use super::store::worker_goal_outcome_is_accounted_in_transaction;

const MAX_OWNER_ID_BYTES: usize = 256;

#[derive(Deserialize)]
#[serde(untagged)]
enum PersistedStepAcceptanceSpec {
    Typed(WorkflowAcceptanceSpecV1),
    Legacy(String),
}

fn acceptance_run_id(source_run_id: &str) -> String {
    format!(
        "worker-acceptance-{}",
        hash_request_bytes(format!("worker-goal-acceptance-v1:{source_run_id}"))
    )
}

fn source_outcome_sha256(
    input: &WorkerGoalOutcomeCommitInput,
) -> Result<String, WorkerGoalAcceptanceStageError> {
    let source_outcome_json = serde_json::to_string(&serde_json::json!({
        "run_id": input.run_id(),
        "outcome": input.outcome(),
        "evidence": input.evidence(),
        "effect": input.effect(),
        "counters": input.counters(),
    }))
    .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    Ok(hash_request_bytes(source_outcome_json.as_bytes()))
}

/// Crash/adoption canary for the source outcome transaction. A `Progressed`
/// outcome is not complete unless the same commit staged its exact immutable
/// acceptance candidate and dedicated run.
pub(crate) fn progressed_acceptance_is_staged_in_transaction(
    tx: &Transaction<'_>,
    input: &WorkerGoalOutcomeCommitInput,
) -> Result<bool, WorkerGoalAcceptanceStageError> {
    if input.outcome() != WorkerGoalAttemptOutcome::Progressed {
        return Ok(false);
    }
    let expected_goal_revision = input.goal_revision().checked_add(1).ok_or_else(|| {
        WorkerGoalAcceptanceStageError::Conflict("Workflow revision overflow".into())
    })?;
    let expected_run_id = acceptance_run_id(input.run_id());
    let expected_source_outcome_sha256 = source_outcome_sha256(input)?;
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM hive_worker_goal_acceptance_candidates candidate
             JOIN hive_runs acceptance_run
               ON acceptance_run.id = candidate.acceptance_run_id
             WHERE candidate.acceptance_run_id = ?1
               AND candidate.source_run_id = ?2
               AND candidate.worker_id = ?3
               AND candidate.worker_revision = ?4
               AND candidate.owner_user_id IS ?5
               AND candidate.session_id = ?6
               AND candidate.workflow_goal_id = ?7
               AND candidate.source_attempt_id = ?8
               AND candidate.plan_revision_id = ?9
               AND candidate.plan_revision_number = ?10
               AND candidate.step_id = ?11
               AND candidate.goal_revision = ?12
               AND candidate.workflow_aggregate_revision = ?12
               AND candidate.step_revision = ?13
               AND candidate.workspace_dir = ?14
               AND candidate.source_outcome_sha256 = ?15
               AND acceptance_run.kind = 'worker_workflow_acceptance'
               AND acceptance_run.worker_id = candidate.worker_id
               AND acceptance_run.workflow_goal_id = candidate.workflow_goal_id
               AND acceptance_run.workflow_attempt_id = candidate.source_attempt_id
         )",
        params![
            expected_run_id,
            input.run_id(),
            input.worker_id(),
            input.worker_revision(),
            input.owner_user_id(),
            input.session_id(),
            input.goal_id(),
            input.attempt_id(),
            input.plan_revision_id(),
            input.plan_revision_number(),
            input.step_id(),
            expected_goal_revision,
            input.step_revision(),
            input.workspace_dir().to_string_lossy(),
            expected_source_outcome_sha256,
        ],
        |row| row.get(0),
    )
    .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))
}

/// Stage the fail-closed V1 acceptance boundary inside the source outcome
/// transaction. The source attempt becomes paused but retains its exact step
/// claim; no rollover can occur before an explicit owner decision.
pub(crate) fn stage_user_review_acceptance_in_transaction(
    tx: &Transaction<'_>,
    input: &WorkerGoalOutcomeCommitInput,
    material_progress: bool,
    no_progress_streak: u32,
    now: &str,
) -> Result<WorkerGoalAcceptanceCandidateRecord, WorkerGoalAcceptanceStageError> {
    if input.outcome() != WorkerGoalAttemptOutcome::Progressed {
        return Err(WorkerGoalAcceptanceStageError::Conflict(
            "only a Progressed source outcome may stage acceptance".into(),
        ));
    }
    let next_goal_revision = input.goal_revision().checked_add(1).ok_or_else(|| {
        WorkerGoalAcceptanceStageError::Conflict("Workflow revision overflow".into())
    })?;
    let acceptance_run_id = acceptance_run_id(input.run_id());
    let source_outcome_sha256 = source_outcome_sha256(input)?;

    let run_binding: Option<(String, Option<i64>, String)> = tx
        .query_row(
            "SELECT controller_id, governor_policy_revision, status
             FROM hive_runs WHERE id = ?1 AND kind = 'worker_workflow'",
            [input.run_id()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    let Some((controller_id, governor_policy_revision, source_run_status)) = run_binding else {
        return Err(WorkerGoalAcceptanceStageError::Conflict(
            "source Worker Workflow run disappeared".into(),
        ));
    };
    if source_run_status != "running" {
        return Err(WorkerGoalAcceptanceStageError::Stale(
            "source Worker Workflow is no longer running".into(),
        ));
    }
    let workspace_mode: String = tx
        .query_row(
            "SELECT workspace_mode FROM sessions WHERE id = ?1",
            [input.session_id()],
            |row| row.get(0),
        )
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    if workspace_mode != "selected" && workspace_mode != "created" {
        return Err(WorkerGoalAcceptanceStageError::Stale(
            "Worker Goal workspace is no longer attached".into(),
        ));
    }

    let attempt_changed = tx
        .execute(
            "UPDATE workflow_execution_attempts
             SET status = 'paused', stop_reason = 'awaiting_acceptance',
                 turn_count = ?1, tool_call_count = ?2,
                 research_action_count = ?3,
                 progress_revision = progress_revision + ?4,
                 blocker_fingerprint = NULL, blocker_streak = ?5,
                 ended_at = ?6, updated_at = ?6
             WHERE id = ?7 AND goal_id = ?8 AND status = 'running'
               AND goal_revision_at_start = ?9",
            params![
                input.counters().turns,
                input.counters().tool_calls,
                input.counters().research_actions,
                i64::from(material_progress),
                no_progress_streak,
                now,
                input.attempt_id(),
                input.goal_id(),
                input.goal_revision(),
            ],
        )
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    if attempt_changed != 1 {
        return Err(WorkerGoalAcceptanceStageError::Stale(
            "source Workflow attempt changed before acceptance staging".into(),
        ));
    }
    let step_acceptance_json: Option<String> = tx
        .query_row(
            "SELECT acceptance_criteria_json FROM workflow_plan_steps
             WHERE id = ?1 AND plan_revision_id = ?2
               AND status = 'in_progress' AND claimed_attempt_id = ?3
               AND revision = ?4",
            params![
                input.step_id(),
                input.plan_revision_id(),
                input.attempt_id(),
                input.step_revision(),
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    let Some(step_acceptance_json) = step_acceptance_json else {
        return Err(WorkerGoalAcceptanceStageError::Stale(
            "source Workflow step changed before acceptance staging".into(),
        ));
    };
    let mut step_specs =
        serde_json::from_str::<Vec<PersistedStepAcceptanceSpec>>(&step_acceptance_json)
            .map_err(|error| {
                WorkerGoalAcceptanceStageError::Conflict(format!(
                    "invalid persisted step acceptance contract: {error}"
                ))
            })?
            .into_iter()
            .map(|persisted| match persisted {
                PersistedStepAcceptanceSpec::Typed(spec) => spec,
                PersistedStepAcceptanceSpec::Legacy(display) => {
                    WorkflowAcceptanceSpecV1::from_legacy_free_form(&display)
                }
            })
            .collect::<Vec<_>>();
    if step_specs.is_empty() {
        step_specs.push(WorkflowAcceptanceSpecV1::user_review());
    }
    let remaining_required_steps: u64 = tx
        .query_row(
            "SELECT COUNT(*) FROM workflow_plan_steps
             WHERE plan_revision_id = ?1 AND id <> ?2 AND required = 1
               AND status NOT IN ('completed', 'skipped')",
            params![input.plan_revision_id(), input.step_id()],
            |row| nonnegative_u64(row, 0),
        )
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    let goal_specs = if remaining_required_steps == 0 {
        let mut statement = tx
            .prepare(
                "SELECT id, description FROM workflow_goal_criteria
                 WHERE goal_id = ?1 AND required = 1
                   AND status NOT IN ('passed', 'waived')
                 ORDER BY position, id",
            )
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        let rows = statement
            .query_map([input.goal_id()], |row| {
                let criterion_id = row.get::<_, String>(0)?;
                let description = row.get::<_, String>(1)?;
                Ok(WorkerGoalCriterionAcceptanceSpecV1 {
                    criterion_id,
                    spec: WorkflowAcceptanceSpecV1::from_legacy_free_form(&description),
                })
            })
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        rows
    } else {
        Vec::new()
    };
    let acceptance_contract = WorkerGoalAcceptanceContractV1 {
        schema_version: WORKER_GOAL_ACCEPTANCE_CONTRACT_VERSION,
        step_specs,
        goal_specs,
    };
    if !acceptance_contract.validate() {
        return Err(WorkerGoalAcceptanceStageError::Conflict(
            "persisted Workflow acceptance contract is invalid or exceeds its bound".into(),
        ));
    }
    let acceptance_contract_json = serde_json::to_string(&acceptance_contract)
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    let acceptance_contract_sha256 = hash_request_bytes(acceptance_contract_json.as_bytes());
    let goal_changed = tx
        .execute(
            "UPDATE workflow_goals
             SET status = 'active', status_reason = 'awaiting_acceptance',
                 revision = ?1, updated_at = ?2
             WHERE id = ?3 AND session_id = ?4 AND status = 'active'
               AND revision = ?5",
            params![
                next_goal_revision,
                now,
                input.goal_id(),
                input.session_id(),
                input.goal_revision(),
            ],
        )
        .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    if goal_changed != 1 {
        return Err(WorkerGoalAcceptanceStageError::Stale(
            "source Workflow Goal changed before acceptance staging".into(),
        ));
    }

    let execution_context = serde_json::json!({
        "schema_version": 1,
        "mode": {
            "kind": "worker_goal_acceptance",
            "worker_id": input.worker_id(),
            "worker_revision": input.worker_revision(),
            "lane": { "kind": "direct_message" },
            "workspace_mode": workspace_mode,
            "working_dir": input.workspace_dir(),
            "project_dir": input.workspace_dir(),
            "source_run_id": input.run_id(),
            "goal_id": input.goal_id(),
            "goal_revision": next_goal_revision,
            "workflow_aggregate_revision": next_goal_revision,
            "source_attempt_id": input.attempt_id(),
            "plan_revision_id": input.plan_revision_id(),
            "plan_revision_number": input.plan_revision_number(),
            "step_id": input.step_id(),
            "step_revision": input.step_revision(),
            "acceptance_contract_sha256": acceptance_contract_sha256,
            "source_outcome_sha256": source_outcome_sha256,
            "tool_allowlist": [],
        }
    });
    let run_config = serde_json::json!({
        "worker_id": input.worker_id(),
        "acceptance_mode": "user_review",
        "source_run_id": input.run_id(),
        "workflow_goal_id": input.goal_id(),
        "workflow_attempt_id": input.attempt_id(),
        "automatic_acceptance_enabled": false,
    });
    tx.execute(
        "INSERT INTO hive_runs (
             id, controller_id, session_id, kind, objective, config_json,
             status, priority, concurrency_key, available_at, attempt_count,
             max_attempts, created_at, updated_at, worker_id,
             governor_origin, governor_lane_key, governor_policy_revision,
             execution_context_json, workflow_goal_id, workflow_attempt_id
         ) VALUES (
             ?1, ?2, ?3, 'worker_workflow_acceptance', ?4, ?5,
             'awaiting_input', 40, ?6, ?7, 0, 1, ?7, ?7, ?8,
             'workflow_acceptance', 'dm', ?9, ?10, ?11, ?12
         )",
        params![
            acceptance_run_id,
            controller_id,
            input.session_id(),
            format!("Review acceptance for Workflow step {}", input.step_id()),
            serde_json::to_string(&run_config)
                .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?,
            format!("worker:{}", input.worker_id()),
            now,
            input.worker_id(),
            governor_policy_revision,
            serde_json::to_string(&execution_context)
                .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?,
            input.goal_id(),
            input.attempt_id(),
        ],
    )
    .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;

    tx.execute(
        "INSERT INTO hive_worker_goal_acceptance_candidates (
             acceptance_run_id, source_run_id, worker_id, worker_revision,
             owner_user_id, session_id, workflow_goal_id, source_attempt_id,
             plan_revision_id, plan_revision_number, step_id, goal_revision,
             workflow_aggregate_revision, step_revision, workspace_dir,
             acceptance_contract_json, acceptance_contract_sha256,
             source_outcome_sha256, state, created_at, updated_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
             ?13, ?14, ?15, ?16, ?17, ?18, 'awaiting_user', ?19, ?19
         )",
        params![
            acceptance_run_id,
            input.run_id(),
            input.worker_id(),
            input.worker_revision(),
            input.owner_user_id(),
            input.session_id(),
            input.goal_id(),
            input.attempt_id(),
            input.plan_revision_id(),
            input.plan_revision_number(),
            input.step_id(),
            next_goal_revision,
            next_goal_revision,
            input.step_revision(),
            input.workspace_dir().to_string_lossy(),
            acceptance_contract_json,
            acceptance_contract_sha256,
            source_outcome_sha256,
            now,
        ],
    )
    .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
    tx.execute(
        "INSERT INTO workflow_events (
             session_id, goal_id, aggregate_revision, operation_id, event_type,
             actor, attempt_id, payload_json, created_at
         ) VALUES (
             ?1, ?2, ?3, ?4, 'worker_workflow_awaiting_acceptance',
             'hive_worker_runtime', ?5, ?6, ?7
         )",
        params![
            input.session_id(),
            input.goal_id(),
            next_goal_revision,
            format!("worker-goal-acceptance-staged:{}", input.run_id()),
            input.attempt_id(),
            serde_json::json!({
                "acceptance_run_id": acceptance_run_id,
                "source_run_id": input.run_id(),
                "step_id": input.step_id(),
                "authority": "user_review",
                "automatic_acceptance_enabled": false,
            })
            .to_string(),
            now,
        ],
    )
    .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;

    Ok(WorkerGoalAcceptanceCandidateRecord {
        acceptance_run_id,
        source_run_id: input.run_id().to_string(),
        worker_id: input.worker_id().to_string(),
        worker_revision: input.worker_revision(),
        owner_user_id: input.owner_user_id().map(str::to_string),
        session_id: input.session_id().to_string(),
        workflow_goal_id: input.goal_id().to_string(),
        source_attempt_id: input.attempt_id().to_string(),
        plan_revision_id: input.plan_revision_id().to_string(),
        plan_revision_number: input.plan_revision_number(),
        step_id: input.step_id().to_string(),
        goal_revision: next_goal_revision,
        workflow_aggregate_revision: next_goal_revision,
        step_revision: input.step_revision(),
        workspace_dir: input.workspace_dir().to_string_lossy().into_owned(),
        acceptance_contract,
        acceptance_contract_sha256,
        source_outcome_sha256,
        source_summary: WorkerGoalAcceptanceSourceSummary {
            outcome: input.outcome(),
            evidence: input.evidence().to_vec(),
            effect: input.effect().clone(),
            counters: input.counters(),
        },
        state: WorkerGoalAcceptanceCandidateState::AwaitingUser,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

#[derive(Debug, Error)]
pub(crate) enum WorkerGoalAcceptanceStageError {
    #[error("Worker Goal acceptance source is stale: {0}")]
    Stale(String),
    #[error("Worker Goal acceptance staging conflicts with durable state: {0}")]
    Conflict(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerGoalAcceptanceLifecycle {
    GoalCancelled,
    WorkerArchived,
}

impl WorkerGoalAcceptanceLifecycle {
    const fn reason(self) -> &'static str {
        match self {
            Self::GoalCancelled => "workflow_goal_cancelled",
            Self::WorkerArchived => "worker_archived",
        }
    }
}

pub(crate) fn pending_worker_goal_acceptance_exists_in_transaction(
    tx: &Transaction<'_>,
    goal_id: &str,
) -> Result<bool, WorkerGoalAcceptanceStageError> {
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM hive_worker_goal_acceptance_candidates
             WHERE workflow_goal_id = ?1
               AND state IN ('awaiting_user', 'needs_user', 'verifying')
         )",
        [goal_id],
        |row| row.get(0),
    )
    .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))
}

/// Invalidate every pending acceptance selected by the exact Goal (and,
/// optionally, Worker) fence. The caller owns the coupled attempt/step/Goal
/// lifecycle mutation in the same transaction.
pub(crate) fn terminalize_pending_worker_goal_acceptances_in_transaction(
    tx: &Transaction<'_>,
    goal_id: Option<&str>,
    worker_id: Option<&str>,
    lifecycle: WorkerGoalAcceptanceLifecycle,
    now: &str,
) -> Result<Vec<WorkerGoalAcceptanceCandidateRecord>, WorkerGoalAcceptanceStageError> {
    if goal_id.is_none() && worker_id.is_none() {
        return Err(WorkerGoalAcceptanceStageError::Conflict(
            "acceptance lifecycle invalidation requires a Goal or Worker fence".into(),
        ));
    }
    let candidate_ids = {
        let mut statement = tx
            .prepare(
                "SELECT acceptance_run_id
                 FROM hive_worker_goal_acceptance_candidates
                 WHERE (?1 IS NULL OR workflow_goal_id = ?1)
                   AND (?2 IS NULL OR worker_id = ?2)
                   AND state IN ('awaiting_user', 'needs_user', 'verifying')
                 ORDER BY created_at, acceptance_run_id",
            )
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        let rows = statement
            .query_map(params![goal_id, worker_id], |row| row.get::<_, String>(0))
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        rows
    };
    let mut terminalized = Vec::with_capacity(candidate_ids.len());
    for acceptance_run_id in candidate_ids {
        let candidate = load_candidate(tx, &acceptance_run_id)
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?
            .ok_or_else(|| {
                WorkerGoalAcceptanceStageError::Conflict(
                    "pending acceptance candidate disappeared during lifecycle invalidation".into(),
                )
            })?;
        if load_result(tx, &acceptance_run_id)
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?
            .is_some()
        {
            return Err(WorkerGoalAcceptanceStageError::Conflict(
                "pending acceptance candidate already has an immutable result".into(),
            ));
        }
        let result = WorkerGoalAcceptanceResultRecord {
            acceptance_run_id: candidate.acceptance_run_id.clone(),
            source_run_id: candidate.source_run_id.clone(),
            authority: WorkerGoalAcceptanceAuthority::Lifecycle,
            decision: UserWorkerGoalAcceptanceDecision::Reject,
            reason: lifecycle.reason().into(),
            criteria: Vec::new(),
            receipts: Vec::new(),
            provider_call_ids: Vec::new(),
            resulting_goal_revision: None,
            resulting_goal_status: None,
            resulting_step_status: None,
            committed_at: now.into(),
        };
        insert_result(tx, &result)
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        let candidate_changed = tx
            .execute(
                "UPDATE hive_worker_goal_acceptance_candidates
                 SET state = 'stale', updated_at = ?2, resolved_at = ?2
                 WHERE acceptance_run_id = ?1
                   AND state IN ('awaiting_user', 'needs_user', 'verifying')",
                params![candidate.acceptance_run_id, now],
            )
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        if candidate_changed != 1 {
            return Err(WorkerGoalAcceptanceStageError::Stale(
                "acceptance candidate changed during lifecycle invalidation".into(),
            ));
        }
        let run_changed = tx
            .execute(
                "UPDATE hive_runs
                 SET status = 'cancelled', outcome_json = ?2,
                     finished_at = ?3, updated_at = ?3,
                     last_stop_reason = ?4, last_error = NULL
                 WHERE id = ?1 AND kind = 'worker_workflow_acceptance'
                   AND status = 'awaiting_input'",
                params![
                    candidate.acceptance_run_id,
                    serde_json::to_string(&exact_result_payload(&result)).map_err(|error| {
                        WorkerGoalAcceptanceStageError::Conflict(error.to_string())
                    })?,
                    now,
                    lifecycle.reason(),
                ],
            )
            .map_err(|error| WorkerGoalAcceptanceStageError::Conflict(error.to_string()))?;
        if run_changed != 1 {
            return Err(WorkerGoalAcceptanceStageError::Stale(
                "acceptance run changed during lifecycle invalidation".into(),
            ));
        }
        terminalized.push(candidate);
    }
    Ok(terminalized)
}

#[derive(Debug, Clone)]
pub struct SqliteWorkerGoalAcceptanceStore {
    database_path: PathBuf,
}

impl SqliteWorkerGoalAcceptanceStore {
    pub fn new(database_path: impl AsRef<Path>) -> Self {
        Self {
            database_path: database_path.as_ref().to_path_buf(),
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn candidate(
        &self,
        acceptance_run_id: &str,
        authenticated_owner_user_id: Option<&str>,
    ) -> Result<Option<WorkerGoalAcceptanceCandidateRecord>, WorkerGoalAcceptanceStoreError> {
        validate_owner(authenticated_owner_user_id)?;
        let database = Database::new(&self.database_path)
            .map_err(|error| database_error("opening acceptance database", error))?;
        let candidate = load_candidate(database.conn(), acceptance_run_id)?;
        // Filtering rather than returning an authorization error hides the
        // existence and ownership of another tenant's candidate.
        Ok(candidate.filter(|candidate| {
            candidate_is_visible_to_owner(
                candidate.owner_user_id.as_deref(),
                authenticated_owner_user_id,
            )
        }))
    }

    /// Atomically apply an explicit owner decision to one exact pending
    /// candidate. `None` is the canonical local single-tenant identity, while
    /// `Some` is an exact authenticated multi-tenant identity. A repeated
    /// byte-equivalent decision adopts the existing result; any different
    /// second decision is a conflict.
    pub fn resolve_user(
        &self,
        authenticated_owner_user_id: Option<&str>,
        request: &UserWorkerGoalAcceptanceRequest,
    ) -> Result<WorkerGoalAcceptanceResolution, WorkerGoalAcceptanceStoreError> {
        validate_owner(authenticated_owner_user_id)?;
        request.validate()?;
        let database = Database::new(&self.database_path)
            .map_err(|error| database_error("opening acceptance database", error))?;
        let tx = Transaction::new_unchecked(database.conn(), TransactionBehavior::Immediate)
            .map_err(|error| database_error("acquiring acceptance writer", error))?;

        let candidate = load_candidate(&tx, &request.acceptance_run_id)?.ok_or_else(|| {
            WorkerGoalAcceptanceStoreError::NotFound(request.acceptance_run_id.clone())
        })?;
        if !candidate_is_visible_to_owner(
            candidate.owner_user_id.as_deref(),
            authenticated_owner_user_id,
        ) {
            // Treat an ownership mismatch exactly like an unknown run so the
            // API cannot be used to enumerate another tenant's candidates.
            return Err(WorkerGoalAcceptanceStoreError::NotFound(
                request.acceptance_run_id.clone(),
            ));
        }

        if let Some(existing) = load_result(&tx, &request.acceptance_run_id)? {
            if request.expected_goal_revision != candidate.goal_revision
                || !exact_user_result_matches_request(&existing, request)
            {
                return Err(WorkerGoalAcceptanceStoreError::Conflict(
                    "acceptance run already has a different immutable result".into(),
                ));
            }
            let resolution = resolution_from_result(
                &candidate,
                &existing,
                WorkerGoalAcceptanceCommitDisposition::AdoptedExact,
            )?;
            tx.commit()
                .map_err(|error| commit_uncertain("adopting acceptance result", error))?;
            return Ok(resolution);
        }

        validate_pending_candidate(&tx, &candidate, request.expected_goal_revision)?;
        validate_requested_criteria(&tx, &candidate, request)?;

        let now = canonical_timestamp(Utc::now());
        let next_goal_revision = candidate.goal_revision.checked_add(1).ok_or_else(|| {
            WorkerGoalAcceptanceStoreError::Conflict("Workflow revision overflow".into())
        })?;
        let (goal_status, step_status) = match request.decision {
            UserWorkerGoalAcceptanceDecision::Accept => {
                apply_user_acceptance(&tx, &candidate, request, next_goal_revision, &now)?
            }
            UserWorkerGoalAcceptanceDecision::Reject => {
                apply_user_rejection(&tx, &candidate, request, next_goal_revision, &now)?
            }
        };

        let result = WorkerGoalAcceptanceResultRecord {
            acceptance_run_id: candidate.acceptance_run_id.clone(),
            source_run_id: candidate.source_run_id.clone(),
            authority: WorkerGoalAcceptanceAuthority::User,
            decision: request.decision,
            reason: request.reason.clone(),
            criteria: request.criteria.clone(),
            receipts: Vec::new(),
            provider_call_ids: Vec::new(),
            resulting_goal_revision: Some(next_goal_revision),
            resulting_goal_status: Some(goal_status.clone()),
            resulting_step_status: Some(step_status.clone()),
            committed_at: now.clone(),
        };
        insert_result(&tx, &result)?;
        let resolution = resolution_from_result(
            &candidate,
            &result,
            WorkerGoalAcceptanceCommitDisposition::Inserted,
        )?;

        let candidate_state = match request.decision {
            UserWorkerGoalAcceptanceDecision::Accept => "accepted",
            UserWorkerGoalAcceptanceDecision::Reject => "rejected",
        };
        let candidate_changed = tx
            .execute(
                "UPDATE hive_worker_goal_acceptance_candidates
                 SET state = ?2, updated_at = ?3, resolved_at = ?3
                 WHERE acceptance_run_id = ?1
                   AND state IN ('awaiting_user', 'needs_user')",
                params![candidate.acceptance_run_id, candidate_state, now],
            )
            .map_err(|error| database_error("resolving acceptance candidate", error))?;
        if candidate_changed != 1 {
            return Err(stale(
                "acceptance candidate changed during owner resolution",
            ));
        }

        let run_outcome = exact_result_payload(&result);
        let run_changed = tx
            .execute(
                "UPDATE hive_runs
                 SET status = 'succeeded', outcome_json = ?2,
                     finished_at = ?3, updated_at = ?3,
                     last_stop_reason = 'owner_acceptance_resolved',
                     last_error = NULL
                 WHERE id = ?1 AND kind = 'worker_workflow_acceptance'
                   AND status = 'awaiting_input'",
                params![
                    candidate.acceptance_run_id,
                    serde_json::to_string(&run_outcome).map_err(|error| database_error(
                        "encoding acceptance run outcome",
                        error
                    ))?,
                    now,
                ],
            )
            .map_err(|error| database_error("terminalizing acceptance run", error))?;
        if run_changed != 1 {
            return Err(stale("acceptance run changed during owner resolution"));
        }

        tx.execute(
            "INSERT INTO workflow_events (
                 session_id, goal_id, aggregate_revision, operation_id,
                 event_type, actor, attempt_id, payload_json, created_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, 'worker_goal_acceptance_resolved',
                 'user', ?5, ?6, ?7
             )",
            params![
                candidate.session_id,
                candidate.workflow_goal_id,
                next_goal_revision,
                format!("worker-goal-acceptance:{}", candidate.acceptance_run_id),
                candidate.source_attempt_id,
                serde_json::json!({
                    "acceptance_run_id": candidate.acceptance_run_id,
                    "source_run_id": candidate.source_run_id,
                    "decision": request.decision,
                    "goal_status": goal_status,
                    "step_status": step_status,
                    "authority": "user",
                })
                .to_string(),
                now,
            ],
        )
        .map_err(|error| database_error("recording acceptance Workflow event", error))?;

        tx.commit()
            .map_err(|error| commit_uncertain("committing owner acceptance", error))?;
        Ok(resolution)
    }
}

#[derive(Debug, Error)]
pub enum WorkerGoalAcceptanceStoreError {
    #[error(transparent)]
    Validation(#[from] WorkflowAcceptanceValidationError),
    #[error("Worker Goal acceptance candidate '{0}' was not found")]
    NotFound(String),
    #[error("authenticated user does not own this Worker Goal acceptance")]
    Forbidden,
    #[error("Worker Goal acceptance is stale: {0}")]
    Stale(String),
    #[error("Worker Goal acceptance conflicts with durable state: {0}")]
    Conflict(String),
    #[error("Worker Goal acceptance storage failed: {0}")]
    Database(String),
    #[error("Worker Goal acceptance may have committed but cannot be proven: {0}")]
    CommitUncertain(String),
}

impl WorkerGoalAcceptanceStoreError {
    pub const fn is_proven_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }
}

fn apply_user_acceptance(
    tx: &Transaction<'_>,
    candidate: &WorkerGoalAcceptanceCandidateRecord,
    request: &UserWorkerGoalAcceptanceRequest,
    next_goal_revision: u64,
    now: &str,
) -> Result<(String, String), WorkerGoalAcceptanceStoreError> {
    for criterion in &request.criteria {
        let changed = tx
            .execute(
                "UPDATE workflow_goal_criteria
                 SET status = ?1, evidence_json = ?2, verifier = ?3,
                     verified_at = ?4
                 WHERE id = ?5 AND goal_id = ?6",
                params![
                    criterion_decision_str(criterion),
                    serde_json::to_string(&criterion.evidence).map_err(|error| database_error(
                        "encoding owner criterion evidence",
                        error
                    ))?,
                    format!("user:{}", candidate.acceptance_run_id),
                    now,
                    criterion.criterion_id,
                    candidate.workflow_goal_id,
                ],
            )
            .map_err(|error| database_error("applying owner criterion decision", error))?;
        if changed != 1 {
            return Err(stale("Goal criterion changed during owner acceptance"));
        }
    }

    let step_changed = tx
        .execute(
            "UPDATE workflow_plan_steps
             SET status = 'completed', claimed_attempt_id = NULL,
                 revision = revision + 1, outcome = ?1, evidence_json = ?2,
                 completed_at = ?3
             WHERE id = ?4 AND plan_revision_id = ?5
               AND status = 'in_progress' AND claimed_attempt_id = ?6
               AND revision = ?7",
            params![
                request.reason,
                serde_json::to_string(&vec![format!(
                    "Explicit owner acceptance {}",
                    candidate.acceptance_run_id
                )])
                .map_err(|error| database_error("encoding owner step evidence", error))?,
                now,
                candidate.step_id,
                candidate.plan_revision_id,
                candidate.source_attempt_id,
                candidate.step_revision,
            ],
        )
        .map_err(|error| database_error("completing owner-accepted step", error))?;
    if step_changed != 1 {
        return Err(stale("Workflow step changed during owner acceptance"));
    }

    let attempt_changed = tx
        .execute(
            "UPDATE workflow_execution_attempts
             SET status = 'succeeded', stop_reason = 'owner_acceptance',
                 progress_revision = progress_revision + 1,
                 ended_at = COALESCE(ended_at, ?1), updated_at = ?1
             WHERE id = ?2 AND goal_id = ?3 AND status = 'paused'
               AND stop_reason = 'awaiting_acceptance'",
            params![now, candidate.source_attempt_id, candidate.workflow_goal_id],
        )
        .map_err(|error| database_error("finalizing owner-accepted attempt", error))?;
    if attempt_changed != 1 {
        return Err(stale("Workflow attempt changed during owner acceptance"));
    }

    let incomplete_required: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM workflow_plan_steps
             WHERE plan_revision_id = ?1 AND required = 1
               AND status NOT IN ('completed', 'skipped')",
            [candidate.plan_revision_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| database_error("counting incomplete Workflow steps", error))?;
    if incomplete_required == 0 {
        tx.execute(
            "UPDATE workflow_plan_revisions
             SET status = 'completed', completed_at = ?1
             WHERE id = ?2 AND status = 'active'",
            params![now, candidate.plan_revision_id],
        )
        .map_err(|error| database_error("completing accepted plan revision", error))?;
    }
    let unmet_required: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM workflow_goal_criteria
             WHERE goal_id = ?1 AND required = 1
               AND status NOT IN ('passed', 'waived')",
            [candidate.workflow_goal_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| database_error("counting unmet Goal criteria", error))?;

    let (goal_status, status_reason, completed_at) = if incomplete_required > 0 {
        ("active", None, None)
    } else if unmet_required > 0 {
        ("paused", Some("awaiting_user_acceptance"), None)
    } else {
        ("completed", Some("verified_by_owner"), Some(now))
    };
    update_goal(
        tx,
        candidate,
        next_goal_revision,
        goal_status,
        status_reason,
        completed_at,
        now,
    )?;
    Ok((goal_status.to_string(), "completed".to_string()))
}

fn apply_user_rejection(
    tx: &Transaction<'_>,
    candidate: &WorkerGoalAcceptanceCandidateRecord,
    request: &UserWorkerGoalAcceptanceRequest,
    next_goal_revision: u64,
    now: &str,
) -> Result<(String, String), WorkerGoalAcceptanceStoreError> {
    let step_changed = tx
        .execute(
            "UPDATE workflow_plan_steps
             SET status = 'pending', claimed_attempt_id = NULL,
                 revision = revision + 1, outcome = ?1, evidence_json = '[]'
             WHERE id = ?2 AND plan_revision_id = ?3
               AND status = 'in_progress' AND claimed_attempt_id = ?4
               AND revision = ?5",
            params![
                request.reason,
                candidate.step_id,
                candidate.plan_revision_id,
                candidate.source_attempt_id,
                candidate.step_revision,
            ],
        )
        .map_err(|error| database_error("releasing owner-rejected step", error))?;
    if step_changed != 1 {
        return Err(stale("Workflow step changed during owner rejection"));
    }
    let attempt_changed = tx
        .execute(
            "UPDATE workflow_execution_attempts
             SET status = 'failed', stop_reason = 'owner_acceptance_rejected',
                 ended_at = COALESCE(ended_at, ?1), updated_at = ?1
             WHERE id = ?2 AND goal_id = ?3 AND status = 'paused'
               AND stop_reason = 'awaiting_acceptance'",
            params![now, candidate.source_attempt_id, candidate.workflow_goal_id],
        )
        .map_err(|error| database_error("finalizing owner-rejected attempt", error))?;
    if attempt_changed != 1 {
        return Err(stale("Workflow attempt changed during owner rejection"));
    }
    update_goal(
        tx,
        candidate,
        next_goal_revision,
        "active",
        Some("acceptance_rejected"),
        None,
        now,
    )?;
    Ok(("active".to_string(), "pending".to_string()))
}

#[allow(clippy::too_many_arguments)]
fn update_goal(
    tx: &Transaction<'_>,
    candidate: &WorkerGoalAcceptanceCandidateRecord,
    next_revision: u64,
    status: &str,
    status_reason: Option<&str>,
    completed_at: Option<&str>,
    now: &str,
) -> Result<(), WorkerGoalAcceptanceStoreError> {
    let changed = tx
        .execute(
            "UPDATE workflow_goals
             SET status = ?1, status_reason = ?2, revision = ?3,
                 completed_at = COALESCE(?4, completed_at), updated_at = ?5
             WHERE id = ?6 AND session_id = ?7 AND revision = ?8
               AND status IN ('active', 'paused')",
            params![
                status,
                status_reason,
                next_revision,
                completed_at,
                now,
                candidate.workflow_goal_id,
                candidate.session_id,
                candidate.goal_revision,
            ],
        )
        .map_err(|error| database_error("advancing accepted Workflow Goal", error))?;
    if changed != 1 {
        return Err(stale("Workflow Goal changed during owner acceptance"));
    }
    Ok(())
}

fn validate_pending_candidate(
    tx: &Transaction<'_>,
    candidate: &WorkerGoalAcceptanceCandidateRecord,
    expected_goal_revision: u64,
) -> Result<(), WorkerGoalAcceptanceStoreError> {
    if candidate.goal_revision != expected_goal_revision
        || candidate.workflow_aggregate_revision != expected_goal_revision
        || !matches!(
            candidate.state,
            WorkerGoalAcceptanceCandidateState::AwaitingUser
                | WorkerGoalAcceptanceCandidateState::NeedsUser
        )
    {
        return Err(stale("acceptance candidate revision or state changed"));
    }

    let acceptance_run: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = tx
        .query_row(
            "SELECT kind, status, worker_id, workflow_goal_id, workflow_attempt_id
             FROM hive_runs WHERE id = ?1",
            [candidate.acceptance_run_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| database_error("loading acceptance run", error))?;
    if acceptance_run.as_ref().is_none_or(|run| {
        run.0 != "worker_workflow_acceptance"
            || run.1 != "awaiting_input"
            || run.2.as_deref() != Some(candidate.worker_id.as_str())
            || run.3.as_deref() != Some(candidate.workflow_goal_id.as_str())
            || run.4.as_deref() != Some(candidate.source_attempt_id.as_str())
    }) {
        return Err(stale("acceptance run is no longer awaiting owner input"));
    }

    let source_run_ok: bool = tx
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_runs
                 WHERE id = ?1 AND kind = 'worker_workflow'
                   AND status = 'succeeded' AND worker_id = ?2
                   AND session_id = ?3 AND workflow_goal_id = ?4
                   AND workflow_attempt_id = ?5
             )",
            params![
                candidate.source_run_id,
                candidate.worker_id,
                candidate.session_id,
                candidate.workflow_goal_id,
                candidate.source_attempt_id,
            ],
            |row| row.get(0),
        )
        .map_err(|error| database_error("validating source Worker Workflow run", error))?;
    if !source_run_ok
        || !worker_goal_outcome_is_accounted_in_transaction(tx, &candidate.source_run_id)
            .map_err(|error| database_error("validating source provider accounting", error))?
    {
        return Err(stale("source Worker Workflow is not fully accounted"));
    }
    let source_payload: Option<(String, String, String, String)> = tx
        .query_row(
            "SELECT outcome, evidence_json, effect_json, counters_json
             FROM hive_worker_goal_outcomes WHERE run_id = ?1",
            [candidate.source_run_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| database_error("loading frozen source outcome", error))?;
    let Some((outcome, evidence_json, effect_json, counters_json)) = source_payload else {
        return Err(stale("source Worker Workflow outcome disappeared"));
    };
    let canonical_source = serde_json::to_string(&serde_json::json!({
        "run_id": candidate.source_run_id,
        "outcome": outcome,
        "evidence": serde_json::from_str::<serde_json::Value>(&evidence_json)
            .map_err(|error| database_error("decoding frozen source evidence", error))?,
        "effect": serde_json::from_str::<serde_json::Value>(&effect_json)
            .map_err(|error| database_error("decoding frozen source effect", error))?,
        "counters": serde_json::from_str::<serde_json::Value>(&counters_json)
            .map_err(|error| database_error("decoding frozen source counters", error))?,
    }))
    .map_err(|error| database_error("encoding frozen source outcome", error))?;
    if hash_request_bytes(canonical_source.as_bytes()) != candidate.source_outcome_sha256 {
        return Err(stale("source Worker Workflow outcome digest changed"));
    }

    let exact: bool = tx
        .query_row(
            "SELECT EXISTS(
                 SELECT 1
                 FROM hive_workers worker
                 JOIN hive_controllers controller ON controller.worker_id = worker.id
                 JOIN sessions session ON session.id = worker.dm_session_id
                 JOIN workflow_goals goal ON goal.id = ?4
                 JOIN workflow_plan_revisions plan ON plan.id = ?6
                 JOIN workflow_plan_steps step ON step.id = ?7
                 JOIN workflow_execution_attempts attempt ON attempt.id = ?5
                 WHERE worker.id = ?1 AND worker.user_id IS ?2
                   AND worker.revision = ?3 AND worker.status = 'active'
                   AND controller.session_id = ?8 AND controller.status = 'active'
                   AND session.id = ?8 AND session.user_id IS ?2
                   AND session.session_type = 'hive'
                   AND session.working_dir = ?9 AND session.project_dir = ?9
                   AND goal.session_id = ?8 AND goal.revision = ?10
                   AND goal.status IN ('active', 'paused')
                   AND plan.goal_id = goal.id AND plan.status = 'active'
                   AND plan.revision_number = ?11
                   AND step.plan_revision_id = plan.id
                   AND step.status = 'in_progress'
                   AND step.claimed_attempt_id = attempt.id
                   AND step.revision = ?12
                   AND attempt.goal_id = goal.id
                   AND attempt.plan_revision_id = plan.id
                   AND attempt.step_id = step.id
                   AND attempt.status = 'paused'
                   AND attempt.stop_reason = 'awaiting_acceptance'
             )",
            params![
                candidate.worker_id,
                candidate.owner_user_id,
                candidate.worker_revision,
                candidate.workflow_goal_id,
                candidate.source_attempt_id,
                candidate.plan_revision_id,
                candidate.step_id,
                candidate.session_id,
                candidate.workspace_dir,
                candidate.goal_revision,
                candidate.plan_revision_number,
                candidate.step_revision,
            ],
            |row| row.get(0),
        )
        .map_err(|error| database_error("validating exact acceptance authority", error))?;
    if !exact {
        return Err(stale(
            "Worker, owner, workspace, Goal, plan, step, or attempt changed",
        ));
    }
    Ok(())
}

fn validate_requested_criteria(
    tx: &Transaction<'_>,
    candidate: &WorkerGoalAcceptanceCandidateRecord,
    request: &UserWorkerGoalAcceptanceRequest,
) -> Result<(), WorkerGoalAcceptanceStoreError> {
    let remaining_required_steps: u64 = tx
        .query_row(
            "SELECT COUNT(*)
             FROM workflow_plan_steps
             WHERE plan_revision_id = ?1 AND id <> ?2 AND required = 1
               AND status NOT IN ('completed', 'skipped')",
            params![candidate.plan_revision_id, candidate.step_id],
            |row| nonnegative_u64(row, 0),
        )
        .map_err(|error| database_error("loading remaining required Workflow steps", error))?;
    let unmet_required_criterion_ids = {
        let mut statement = tx
            .prepare(
                "SELECT id FROM workflow_goal_criteria
                 WHERE goal_id = ?1 AND required = 1
                   AND status NOT IN ('passed', 'waived')
                 ORDER BY position, id",
            )
            .map_err(|error| database_error("loading unmet Goal criteria", error))?;
        let rows = statement
            .query_map([candidate.workflow_goal_id.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| database_error("querying unmet Goal criteria", error))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| database_error("reading unmet Goal criteria", error))?;
        rows
    };
    let frozen_goal_criterion_ids = candidate
        .acceptance_contract
        .goal_specs
        .iter()
        .map(|item| item.criterion_id.clone())
        .collect::<Vec<_>>();
    if (remaining_required_steps > 0 && !frozen_goal_criterion_ids.is_empty())
        || (remaining_required_steps == 0
            && frozen_goal_criterion_ids != unmet_required_criterion_ids)
    {
        return Err(stale(
            "required step or Goal criterion state changed after acceptance staging",
        ));
    }
    validate_requested_criteria_policy(
        request.decision,
        remaining_required_steps,
        &frozen_goal_criterion_ids,
        &request.criteria,
    )
}

fn validate_requested_criteria_policy(
    decision: UserWorkerGoalAcceptanceDecision,
    remaining_required_steps: u64,
    unmet_required_criterion_ids: &[String],
    requested: &[UserGoalCriterionAcceptance],
) -> Result<(), WorkerGoalAcceptanceStoreError> {
    if decision == UserWorkerGoalAcceptanceDecision::Reject {
        return if requested.is_empty() {
            Ok(())
        } else {
            Err(WorkerGoalAcceptanceStoreError::Conflict(
                "a rejected step cannot mutate Goal criteria".into(),
            ))
        };
    }
    if remaining_required_steps > 0 {
        return if requested.is_empty() {
            Ok(())
        } else {
            Err(WorkerGoalAcceptanceStoreError::Conflict(
                "a nonfinal step acceptance cannot mutate Goal criteria".into(),
            ))
        };
    }

    let mut unmet = unmet_required_criterion_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if requested.len() != unmet.len() {
        return Err(WorkerGoalAcceptanceStoreError::Conflict(
            "final step acceptance must decide every unmet required Goal criterion exactly once"
                .into(),
        ));
    }
    for criterion in requested {
        if !matches!(
            criterion.decision,
            UserGoalCriterionDecision::Passed | UserGoalCriterionDecision::Waived
        ) {
            return Err(WorkerGoalAcceptanceStoreError::Conflict(
                "final step acceptance may only pass or waive required Goal criteria".into(),
            ));
        }
        if !unmet.remove(criterion.criterion_id.as_str()) {
            return Err(WorkerGoalAcceptanceStoreError::Conflict(
                "final step acceptance contains an extra or already-resolved Goal criterion".into(),
            ));
        }
    }
    if !unmet.is_empty() {
        return Err(WorkerGoalAcceptanceStoreError::Conflict(
            "final step acceptance omitted an unmet required Goal criterion".into(),
        ));
    }
    Ok(())
}

fn exact_user_result_matches_request(
    existing: &WorkerGoalAcceptanceResultRecord,
    request: &UserWorkerGoalAcceptanceRequest,
) -> bool {
    existing.authority == WorkerGoalAcceptanceAuthority::User
        && existing.decision == request.decision
        && existing.reason == request.reason
        && existing.criteria == request.criteria
        && existing.receipts.is_empty()
        && existing.provider_call_ids.is_empty()
}

fn insert_result(
    tx: &Transaction<'_>,
    result: &WorkerGoalAcceptanceResultRecord,
) -> Result<(), WorkerGoalAcceptanceStoreError> {
    let changed = tx
        .execute(
            "INSERT INTO hive_worker_goal_acceptance_results (
                 acceptance_run_id, source_run_id, authority, decision,
                 reason, criteria_json, receipts_json, provider_call_ids_json,
                 resulting_goal_revision, resulting_goal_status,
                 resulting_step_status, committed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                result.acceptance_run_id,
                result.source_run_id,
                authority_str(result.authority),
                decision_str(result.decision),
                result.reason,
                serde_json::to_string(&result.criteria)
                    .map_err(|error| database_error("encoding owner criterion decisions", error))?,
                serde_json::to_string(&result.receipts)
                    .map_err(|error| database_error("encoding acceptance receipts", error))?,
                serde_json::to_string(&result.provider_call_ids)
                    .map_err(|error| database_error("encoding acceptance provider calls", error))?,
                result.resulting_goal_revision,
                result.resulting_goal_status,
                result.resulting_step_status,
                result.committed_at,
            ],
        )
        .map_err(|error| database_error("inserting immutable acceptance result", error))?;
    if changed != 1 {
        return Err(WorkerGoalAcceptanceStoreError::Conflict(
            "acceptance result insert did not write one row".into(),
        ));
    }
    Ok(())
}

fn load_candidate(
    connection: &rusqlite::Connection,
    acceptance_run_id: &str,
) -> Result<Option<WorkerGoalAcceptanceCandidateRecord>, WorkerGoalAcceptanceStoreError> {
    connection
        .query_row(
            "SELECT candidate.acceptance_run_id, candidate.source_run_id,
                    candidate.worker_id, candidate.worker_revision,
                    candidate.owner_user_id, candidate.session_id,
                    candidate.workflow_goal_id, candidate.source_attempt_id,
                    candidate.plan_revision_id, candidate.plan_revision_number,
                    candidate.step_id, candidate.goal_revision,
                    candidate.workflow_aggregate_revision,
                    candidate.step_revision, candidate.workspace_dir,
                    candidate.acceptance_contract_json,
                    candidate.acceptance_contract_sha256,
                    candidate.source_outcome_sha256, candidate.state,
                    candidate.created_at,
                    candidate.updated_at, source.outcome, source.evidence_json,
                    source.effect_json, source.counters_json
             FROM hive_worker_goal_acceptance_candidates candidate
             JOIN hive_worker_goal_outcomes source
               ON source.run_id = candidate.source_run_id
             WHERE candidate.acceptance_run_id = ?1",
            [acceptance_run_id],
            map_candidate,
        )
        .optional()
        .map_err(|error| database_error("loading acceptance candidate", error))
}

fn map_candidate(row: &Row<'_>) -> rusqlite::Result<WorkerGoalAcceptanceCandidateRecord> {
    let contract_json: String = row.get(15)?;
    let acceptance_contract =
        serde_json::from_str::<WorkerGoalAcceptanceContractV1>(&contract_json)
            .map_err(|error| conversion_error(15, error.to_string()))?;
    if !acceptance_contract.validate() {
        return Err(conversion_error(15, "invalid acceptance contract"));
    }
    let acceptance_contract_sha256: String = row.get(16)?;
    if hash_request_bytes(contract_json.as_bytes()) != acceptance_contract_sha256 {
        return Err(conversion_error(16, "acceptance contract digest mismatch"));
    }
    let source_outcome_sha256: String = row.get(17)?;
    if source_outcome_sha256.len() != 64
        || !source_outcome_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(conversion_error(17, "invalid source outcome digest"));
    }
    let state: String = row.get(18)?;
    let source_run_id: String = row.get(1)?;
    let source_summary = decode_source_summary(row, &source_run_id, &source_outcome_sha256)?;
    Ok(WorkerGoalAcceptanceCandidateRecord {
        acceptance_run_id: row.get(0)?,
        source_run_id,
        worker_id: row.get(2)?,
        worker_revision: nonnegative_u64(row, 3)?,
        owner_user_id: row.get(4)?,
        session_id: row.get(5)?,
        workflow_goal_id: row.get(6)?,
        source_attempt_id: row.get(7)?,
        plan_revision_id: row.get(8)?,
        plan_revision_number: nonnegative_u64(row, 9)?,
        step_id: row.get(10)?,
        goal_revision: nonnegative_u64(row, 11)?,
        workflow_aggregate_revision: nonnegative_u64(row, 12)?,
        step_revision: nonnegative_u64(row, 13)?,
        workspace_dir: row.get(14)?,
        acceptance_contract,
        acceptance_contract_sha256,
        source_outcome_sha256,
        source_summary,
        state: parse_candidate_state(&state)
            .ok_or_else(|| conversion_error(18, "invalid acceptance candidate state"))?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn decode_source_summary(
    row: &Row<'_>,
    source_run_id: &str,
    expected_sha256: &str,
) -> rusqlite::Result<WorkerGoalAcceptanceSourceSummary> {
    let outcome_text: String = row.get(21)?;
    let evidence_json: String = row.get(22)?;
    let effect_json: String = row.get(23)?;
    let counters_json: String = row.get(24)?;
    let evidence_value = serde_json::from_str::<serde_json::Value>(&evidence_json)
        .map_err(|error| conversion_error(22, error.to_string()))?;
    let effect_value = serde_json::from_str::<serde_json::Value>(&effect_json)
        .map_err(|error| conversion_error(23, error.to_string()))?;
    let counters_value = serde_json::from_str::<serde_json::Value>(&counters_json)
        .map_err(|error| conversion_error(24, error.to_string()))?;
    let canonical_source = serde_json::to_string(&serde_json::json!({
        "run_id": source_run_id,
        "outcome": &outcome_text,
        "evidence": evidence_value,
        "effect": effect_value,
        "counters": counters_value,
    }))
    .map_err(|error| conversion_error(21, error.to_string()))?;
    if hash_request_bytes(canonical_source.as_bytes()) != expected_sha256 {
        return Err(conversion_error(21, "source outcome digest mismatch"));
    }

    let outcome =
        serde_json::from_value::<WorkerGoalAttemptOutcome>(serde_json::Value::String(outcome_text))
            .map_err(|error| conversion_error(21, error.to_string()))?;
    if outcome != WorkerGoalAttemptOutcome::Progressed {
        return Err(conversion_error(
            21,
            "acceptance source outcome is not Progressed",
        ));
    }
    let evidence = serde_json::from_str::<Vec<WorkerGoalEvidence>>(&evidence_json)
        .map_err(|error| conversion_error(22, error.to_string()))?;
    if evidence.len() > MAX_WORKER_GOAL_EVIDENCE_ITEMS
        || evidence
            .iter()
            .any(|item| WorkerGoalEvidence::new(item.kind(), item.summary().to_string()).is_err())
    {
        return Err(conversion_error(22, "invalid bounded source evidence"));
    }
    let effect = serde_json::from_str::<WorkerGoalEffectSummary>(&effect_json)
        .map_err(|error| conversion_error(23, error.to_string()))?;
    if WorkerGoalEffectSummary::new(effect.summary().to_string(), effect.workspace_mutated())
        .is_err()
    {
        return Err(conversion_error(23, "invalid bounded source effect"));
    }
    let counters = serde_json::from_str::<WorkerGoalOutcomeCounters>(&counters_json)
        .map_err(|error| conversion_error(24, error.to_string()))?;
    if counters
        .successful_tool_calls
        .saturating_add(counters.failed_tool_calls)
        != counters.tool_calls
        || counters.research_actions > counters.tool_calls
    {
        return Err(conversion_error(24, "invalid source outcome counters"));
    }
    Ok(WorkerGoalAcceptanceSourceSummary {
        outcome,
        evidence,
        effect,
        counters,
    })
}

fn load_result(
    connection: &rusqlite::Connection,
    acceptance_run_id: &str,
) -> Result<Option<WorkerGoalAcceptanceResultRecord>, WorkerGoalAcceptanceStoreError> {
    connection
        .query_row(
            "SELECT acceptance_run_id, source_run_id, authority, decision,
                    reason, criteria_json, receipts_json,
                    provider_call_ids_json, resulting_goal_revision,
                    resulting_goal_status, resulting_step_status, committed_at
             FROM hive_worker_goal_acceptance_results
             WHERE acceptance_run_id = ?1",
            [acceptance_run_id],
            |row| {
                let authority: String = row.get(2)?;
                let decision: String = row.get(3)?;
                let criteria: String = row.get(5)?;
                let receipts: String = row.get(6)?;
                let provider_calls: String = row.get(7)?;
                Ok(WorkerGoalAcceptanceResultRecord {
                    acceptance_run_id: row.get(0)?,
                    source_run_id: row.get(1)?,
                    authority: parse_authority(&authority)
                        .ok_or_else(|| conversion_error(2, "invalid acceptance authority"))?,
                    decision: parse_decision(&decision)
                        .ok_or_else(|| conversion_error(3, "invalid acceptance decision"))?,
                    reason: row.get(4)?,
                    criteria: serde_json::from_str(&criteria)
                        .map_err(|error| conversion_error(5, error.to_string()))?,
                    receipts: serde_json::from_str(&receipts)
                        .map_err(|error| conversion_error(6, error.to_string()))?,
                    provider_call_ids: serde_json::from_str(&provider_calls)
                        .map_err(|error| conversion_error(7, error.to_string()))?,
                    resulting_goal_revision: optional_nonnegative_u64(row, 8)?,
                    resulting_goal_status: row.get(9)?,
                    resulting_step_status: row.get(10)?,
                    committed_at: row.get(11)?,
                })
            },
        )
        .optional()
        .map_err(|error| database_error("loading acceptance result", error))
}

fn resolution_from_result(
    candidate: &WorkerGoalAcceptanceCandidateRecord,
    result: &WorkerGoalAcceptanceResultRecord,
    disposition: WorkerGoalAcceptanceCommitDisposition,
) -> Result<WorkerGoalAcceptanceResolution, WorkerGoalAcceptanceStoreError> {
    if result.authority != WorkerGoalAcceptanceAuthority::User
        || result.acceptance_run_id != candidate.acceptance_run_id
        || result.source_run_id != candidate.source_run_id
    {
        return Err(WorkerGoalAcceptanceStoreError::Conflict(
            "immutable owner acceptance result has an invalid response binding".into(),
        ));
    }
    let goal_revision = result.resulting_goal_revision.ok_or_else(|| {
        WorkerGoalAcceptanceStoreError::Conflict(
            "immutable owner acceptance result has no frozen Goal revision".into(),
        )
    })?;
    let goal_status = result.resulting_goal_status.clone().ok_or_else(|| {
        WorkerGoalAcceptanceStoreError::Conflict(
            "immutable owner acceptance result has no frozen Goal status".into(),
        )
    })?;
    let step_status = result.resulting_step_status.clone().ok_or_else(|| {
        WorkerGoalAcceptanceStoreError::Conflict(
            "immutable owner acceptance result has no frozen step status".into(),
        )
    })?;
    Ok(WorkerGoalAcceptanceResolution {
        disposition,
        acceptance_run_id: candidate.acceptance_run_id.clone(),
        source_run_id: candidate.source_run_id.clone(),
        workflow_goal_id: candidate.workflow_goal_id.clone(),
        source_attempt_id: candidate.source_attempt_id.clone(),
        step_id: candidate.step_id.clone(),
        decision: result.decision,
        goal_revision,
        goal_status,
        step_status,
    })
}

fn validate_owner(owner_user_id: Option<&str>) -> Result<(), WorkerGoalAcceptanceStoreError> {
    let Some(owner_user_id) = owner_user_id else {
        // `None` is the canonical local single-tenant actor identity. It must
        // remain distinct from every authenticated multi-tenant identity.
        return Ok(());
    };
    if owner_user_id.trim().is_empty()
        || owner_user_id.trim() != owner_user_id
        || owner_user_id.len() > MAX_OWNER_ID_BYTES
        || owner_user_id.chars().any(char::is_control)
    {
        return Err(WorkerGoalAcceptanceStoreError::Forbidden);
    }
    Ok(())
}

fn candidate_is_visible_to_owner(candidate_owner: Option<&str>, actor_owner: Option<&str>) -> bool {
    candidate_owner == actor_owner
}

fn criterion_decision_str(criterion: &UserGoalCriterionAcceptance) -> &'static str {
    match criterion.decision {
        UserGoalCriterionDecision::Passed => "passed",
        UserGoalCriterionDecision::Failed => "failed",
        UserGoalCriterionDecision::Waived => "waived",
    }
}

fn authority_str(authority: WorkerGoalAcceptanceAuthority) -> &'static str {
    match authority {
        WorkerGoalAcceptanceAuthority::User => "user",
        WorkerGoalAcceptanceAuthority::Lifecycle => "lifecycle",
        WorkerGoalAcceptanceAuthority::Structural => "structural",
        WorkerGoalAcceptanceAuthority::StructuralAndSemantic => "structural_and_semantic",
    }
}

fn parse_authority(value: &str) -> Option<WorkerGoalAcceptanceAuthority> {
    match value {
        "user" => Some(WorkerGoalAcceptanceAuthority::User),
        "lifecycle" => Some(WorkerGoalAcceptanceAuthority::Lifecycle),
        "structural" => Some(WorkerGoalAcceptanceAuthority::Structural),
        "structural_and_semantic" => Some(WorkerGoalAcceptanceAuthority::StructuralAndSemantic),
        _ => None,
    }
}

fn decision_str(decision: UserWorkerGoalAcceptanceDecision) -> &'static str {
    match decision {
        UserWorkerGoalAcceptanceDecision::Accept => "accept",
        UserWorkerGoalAcceptanceDecision::Reject => "reject",
    }
}

fn parse_decision(value: &str) -> Option<UserWorkerGoalAcceptanceDecision> {
    match value {
        "accept" => Some(UserWorkerGoalAcceptanceDecision::Accept),
        "reject" => Some(UserWorkerGoalAcceptanceDecision::Reject),
        _ => None,
    }
}

fn parse_candidate_state(value: &str) -> Option<WorkerGoalAcceptanceCandidateState> {
    match value {
        "awaiting_user" => Some(WorkerGoalAcceptanceCandidateState::AwaitingUser),
        "verifying" => Some(WorkerGoalAcceptanceCandidateState::Verifying),
        "needs_user" => Some(WorkerGoalAcceptanceCandidateState::NeedsUser),
        "accepted" => Some(WorkerGoalAcceptanceCandidateState::Accepted),
        "rejected" => Some(WorkerGoalAcceptanceCandidateState::Rejected),
        "stale" => Some(WorkerGoalAcceptanceCandidateState::Stale),
        _ => None,
    }
}

fn nonnegative_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|error| conversion_error(index, error.to_string()))
}

fn optional_nonnegative_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|error| conversion_error(index, error.to_string()))
        })
        .transpose()
}

fn conversion_error(index: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.into(),
        )),
    )
}

fn stale(reason: impl Into<String>) -> WorkerGoalAcceptanceStoreError {
    WorkerGoalAcceptanceStoreError::Stale(reason.into())
}

fn database_error(context: &str, error: impl std::fmt::Display) -> WorkerGoalAcceptanceStoreError {
    WorkerGoalAcceptanceStoreError::Database(format!("{context}: {error}"))
}

fn commit_uncertain(
    context: &str,
    error: impl std::fmt::Display,
) -> WorkerGoalAcceptanceStoreError {
    WorkerGoalAcceptanceStoreError::CommitUncertain(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criterion(id: &str, decision: UserGoalCriterionDecision) -> UserGoalCriterionAcceptance {
        UserGoalCriterionAcceptance {
            criterion_id: id.into(),
            decision,
            evidence: vec![format!("owner evidence for {id}")],
        }
    }

    fn request(criteria: Vec<UserGoalCriterionAcceptance>) -> UserWorkerGoalAcceptanceRequest {
        UserWorkerGoalAcceptanceRequest {
            acceptance_run_id: "acceptance-run".into(),
            expected_goal_revision: 4,
            decision: UserWorkerGoalAcceptanceDecision::Accept,
            reason: "I reviewed the exact result".into(),
            criteria,
        }
    }

    #[test]
    fn nonfinal_acceptance_rejects_goal_criterion_mutations() {
        let requested = vec![criterion("criterion-a", UserGoalCriterionDecision::Passed)];
        assert!(matches!(
            validate_requested_criteria_policy(
                UserWorkerGoalAcceptanceDecision::Accept,
                1,
                &["criterion-a".into()],
                &requested,
            ),
            Err(WorkerGoalAcceptanceStoreError::Conflict(_))
        ));
        assert!(validate_requested_criteria_policy(
            UserWorkerGoalAcceptanceDecision::Accept,
            1,
            &["criterion-a".into()],
            &[],
        )
        .is_ok());
    }

    #[test]
    fn final_acceptance_requires_the_exact_unmet_required_criterion_set() {
        let expected = vec!["criterion-a".into(), "criterion-b".into()];
        let missing = vec![criterion("criterion-a", UserGoalCriterionDecision::Passed)];
        let extra = vec![
            criterion("criterion-a", UserGoalCriterionDecision::Passed),
            criterion("criterion-b", UserGoalCriterionDecision::Waived),
            criterion("criterion-c", UserGoalCriterionDecision::Passed),
        ];
        let failed = vec![
            criterion("criterion-a", UserGoalCriterionDecision::Passed),
            criterion("criterion-b", UserGoalCriterionDecision::Failed),
        ];
        for requested in [&missing, &extra, &failed] {
            assert!(matches!(
                validate_requested_criteria_policy(
                    UserWorkerGoalAcceptanceDecision::Accept,
                    0,
                    &expected,
                    requested,
                ),
                Err(WorkerGoalAcceptanceStoreError::Conflict(_))
            ));
        }
        let exact = vec![
            criterion("criterion-b", UserGoalCriterionDecision::Waived),
            criterion("criterion-a", UserGoalCriterionDecision::Passed),
        ];
        assert!(validate_requested_criteria_policy(
            UserWorkerGoalAcceptanceDecision::Accept,
            0,
            &expected,
            &exact,
        )
        .is_ok());
    }

    #[test]
    fn replay_adopts_only_the_exact_immutable_user_result() {
        let exact_request = request(vec![criterion(
            "criterion-a",
            UserGoalCriterionDecision::Passed,
        )]);
        let result = WorkerGoalAcceptanceResultRecord {
            acceptance_run_id: exact_request.acceptance_run_id.clone(),
            source_run_id: "source-run".into(),
            authority: WorkerGoalAcceptanceAuthority::User,
            decision: exact_request.decision,
            reason: exact_request.reason.clone(),
            criteria: exact_request.criteria.clone(),
            receipts: Vec::new(),
            provider_call_ids: Vec::new(),
            resulting_goal_revision: Some(3),
            resulting_goal_status: Some("active".into()),
            resulting_step_status: Some("completed".into()),
            committed_at: "2026-08-25T00:00:00.000Z".into(),
        };
        assert!(exact_user_result_matches_request(&result, &exact_request));

        let mut different = exact_request;
        different.reason = "a different second decision".into();
        assert!(!exact_user_result_matches_request(&result, &different));
    }

    #[test]
    fn local_owner_identity_is_exactly_null() {
        assert!(validate_owner(None).is_ok());
        assert!(candidate_is_visible_to_owner(None, None));
        assert!(!candidate_is_visible_to_owner(None, Some("owner-a")));
        assert!(!candidate_is_visible_to_owner(Some("owner-a"), None));
    }

    #[test]
    fn another_authenticated_owner_cannot_see_the_candidate() {
        assert!(validate_owner(Some("owner-a")).is_ok());
        assert!(validate_owner(Some("owner-b")).is_ok());
        assert!(candidate_is_visible_to_owner(
            Some("owner-a"),
            Some("owner-a")
        ));
        assert!(!candidate_is_visible_to_owner(
            Some("owner-a"),
            Some("owner-b")
        ));
    }

    #[test]
    fn malformed_authenticated_owner_is_rejected_but_local_is_not() {
        assert!(validate_owner(None).is_ok());
        for malformed in [Some(""), Some(" owner"), Some("owner\n")] {
            assert!(matches!(
                validate_owner(malformed),
                Err(WorkerGoalAcceptanceStoreError::Forbidden)
            ));
        }
    }
}
