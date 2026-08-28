use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde_json::Value;

use crate::agent::{
    WorkerGoalAttemptOutcome, WorkerGoalEvidenceKind, WorkerGoalOutcomeCommit,
    WorkerGoalOutcomeCommitDisposition, WorkerGoalOutcomeCommitError, WorkerGoalOutcomeCommitInput,
    WorkerGoalOutcomeCommitter,
};
use crate::hive::canonical_timestamp;
use crate::storage::{hash_request_bytes, DaemonFence, Database};

use super::{
    progressed_acceptance_is_staged_in_transaction, stage_user_review_acceptance_in_transaction,
    WorkerGoalAcceptanceStageError, WorkerGoalOutcomeRecord,
};

const MAX_ERROR_BYTES: usize = 2_048;

/// SQLite implementation of the trusted Worker Goal outcome boundary.
///
/// One instance is frozen to one scheduler generation.  The transaction
/// independently revalidates the daemon generation, run lease, Worker,
/// Workflow aggregate, plan/step claim, workspace and every provider Started
/// row before it writes the append-only result.
#[derive(Debug, Clone)]
pub struct SqliteWorkerGoalOutcomeStore {
    database_path: PathBuf,
    daemon_fence: DaemonFence,
}

impl SqliteWorkerGoalOutcomeStore {
    pub fn new(database_path: impl AsRef<Path>, daemon_fence: DaemonFence) -> Self {
        Self {
            database_path: database_path.as_ref().to_path_buf(),
            daemon_fence,
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn daemon_fence(&self) -> &DaemonFence {
        &self.daemon_fence
    }
}

impl WorkerGoalOutcomeCommitter for SqliteWorkerGoalOutcomeStore {
    fn commit_outcome(
        &self,
        input: &WorkerGoalOutcomeCommitInput,
    ) -> Result<WorkerGoalOutcomeCommit, WorkerGoalOutcomeCommitError> {
        let database = Database::new(&self.database_path)
            .map_err(|error| conflict(format!("opening Worker Goal database: {error:#}")))?;
        let tx = Transaction::new_unchecked(database.conn(), TransactionBehavior::Immediate)
            .map_err(|error| conflict(format!("acquiring Worker Goal writer: {error}")))?;

        let durable = commit_outcome_in_transaction(&tx, &self.daemon_fence, input)?;
        tx.commit().map_err(|error| {
            WorkerGoalOutcomeCommitError::CommitUncertain(bounded(format!(
                "SQLite commit failed after Worker Goal writes: {error}"
            )))
        })?;
        Ok(durable)
    }
}

fn commit_outcome_in_transaction(
    tx: &Transaction<'_>,
    daemon_fence: &DaemonFence,
    input: &WorkerGoalOutcomeCommitInput,
) -> Result<WorkerGoalOutcomeCommit, WorkerGoalOutcomeCommitError> {
    let provider_call_ids_json = serde_json::to_string(input.provider_call_ids())
        .map_err(|error| conflict(format!("encoding provider identities: {error}")))?;
    let evidence_json = serde_json::to_string(input.evidence())
        .map_err(|error| conflict(format!("encoding Worker Goal evidence: {error}")))?;
    let effect_json = serde_json::to_string(input.effect())
        .map_err(|error| conflict(format!("encoding Worker Goal effect: {error}")))?;
    let counters_json = serde_json::to_string(&input.counters())
        .map_err(|error| conflict(format!("encoding Worker Goal counters: {error}")))?;
    let workspace_dir = input.workspace_dir().to_string_lossy().into_owned();

    if let Some(existing) = committed_worker_goal_outcome_in_transaction(tx, input.run_id())
        .map_err(|error| conflict(format!("loading durable Worker Goal outcome: {error}")))?
    {
        let exact = existing.worker_id == input.worker_id()
            && existing.owner_user_id.as_deref() == input.owner_user_id()
            && existing.session_id == input.session_id()
            && existing.workflow_goal_id == input.goal_id()
            && existing.workflow_attempt_id == input.attempt_id()
            && existing.plan_revision_id == input.plan_revision_id()
            && existing.step_id == input.step_id()
            && existing.workspace_dir == workspace_dir
            && existing.provider_call_ids == input.provider_call_ids()
            && existing.outcome == input.outcome()
            && existing.evidence
                == serde_json::from_str::<Value>(&evidence_json).unwrap_or_default()
            && existing.effect == serde_json::from_str::<Value>(&effect_json).unwrap_or_default()
            && existing.counters
                == serde_json::from_str::<Value>(&counters_json).unwrap_or_default();
        if exact {
            if input.outcome() == WorkerGoalAttemptOutcome::Progressed
                && !progressed_acceptance_is_staged_in_transaction(tx, input)
                    .map_err(|error| conflict(error.to_string()))?
            {
                return Err(conflict(
                    "Progressed outcome exists without its atomic acceptance candidate",
                ));
            }
            return Ok(WorkerGoalOutcomeCommit {
                disposition: WorkerGoalOutcomeCommitDisposition::AdoptedExact,
            });
        }
        return Err(conflict(
            "run already has a different immutable Worker Goal outcome",
        ));
    }

    let now = canonical_timestamp(Utc::now());
    let daemon_current = tx
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_daemon_leases
                 WHERE lease_name = ?1 AND owner_id = ?2 AND fencing_token = ?3
                   AND expires_at > ?4
             )",
            params![
                daemon_fence.lease_name,
                daemon_fence.owner_id,
                daemon_fence.fencing_token,
                now,
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| conflict(format!("validating daemon generation: {error}")))?;
    if !daemon_current || input.run_lease_epoch() != daemon_fence.fencing_token {
        return Err(stale("daemon generation is no longer current"));
    }

    let binding = load_binding(tx, input.run_id())?
        .ok_or_else(|| conflict("Worker Workflow run does not exist"))?;
    validate_binding(tx, daemon_fence, input, &binding, &now)?;
    validate_provider_calls(tx, input, &binding)?;

    // `Succeeded` is reserved for a future structural acceptance authority.
    // The current runner can supply concrete tool evidence, but it cannot
    // prove that arbitrary user-authored step/Goal criteria are satisfied.
    // Persisting such a claim would turn model/tool prose into Workflow
    // authority, so reject it before any write.
    if input.outcome() == WorkerGoalAttemptOutcome::Succeeded {
        return Err(conflict(
            "Worker Goal step completion requires a structural verifier authority",
        ));
    }
    let material_progress = input.effect().workspace_mutated()
        || input.evidence().iter().any(|item| {
            matches!(
                item.kind(),
                WorkerGoalEvidenceKind::WorkspaceMutation | WorkerGoalEvidenceKind::Verification
            )
        });
    let no_progress_fingerprint = (!material_progress).then(|| {
        hash_request_bytes(
            serde_json::json!({
                "outcome": outcome_str(input.outcome()),
                "evidence": input.evidence(),
                "effect": input.effect(),
            })
            .to_string(),
        )
    });
    let previous: Option<(Option<String>, u32)> = tx
        .query_row(
            "SELECT no_progress_fingerprint, no_progress_streak
             FROM hive_worker_goal_outcomes
             WHERE workflow_goal_id = ?1 AND step_id = ?2
             ORDER BY committed_at DESC, run_id DESC LIMIT 1",
            params![input.goal_id(), input.step_id()],
            |row| Ok((row.get(0)?, nonnegative_u32(row, 1)?)),
        )
        .optional()
        .map_err(|error| conflict(format!("loading no-progress history: {error}")))?;
    let no_progress_streak = match (&no_progress_fingerprint, previous) {
        (Some(current), Some((Some(previous), streak))) if current == &previous => {
            streak.saturating_add(1).min(3)
        }
        (Some(_), _) => 1,
        (None, _) => 0,
    };

    let outcome_inserted = tx
        .execute(
            "INSERT INTO hive_worker_goal_outcomes (
             run_id, worker_id, owner_user_id, session_id, workflow_goal_id,
             workflow_attempt_id, plan_revision_id, step_id, workspace_dir,
             provider_call_ids_json, outcome, evidence_json, effect_json,
             counters_json, no_progress_fingerprint, no_progress_streak,
             committed_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
             ?14, ?15, ?16, ?17
         )",
            params![
                input.run_id(),
                input.worker_id(),
                input.owner_user_id(),
                input.session_id(),
                input.goal_id(),
                input.attempt_id(),
                input.plan_revision_id(),
                input.step_id(),
                workspace_dir,
                provider_call_ids_json,
                outcome_str(input.outcome()),
                evidence_json,
                effect_json,
                counters_json,
                no_progress_fingerprint,
                no_progress_streak,
                now,
            ],
        )
        .map_err(|error| conflict(format!("inserting immutable Worker Goal outcome: {error}")))?;
    if outcome_inserted != 1 {
        return Err(conflict(
            "Worker Goal outcome insert did not persist one row",
        ));
    }

    if input.outcome() == WorkerGoalAttemptOutcome::Progressed {
        stage_user_review_acceptance_in_transaction(
            tx,
            input,
            material_progress,
            no_progress_streak,
            &now,
        )
        .map_err(|error| match error {
            WorkerGoalAcceptanceStageError::Stale(message) => stale(message),
            WorkerGoalAcceptanceStageError::Conflict(message) => conflict(message),
        })?;
    } else {
        apply_trusted_workflow_outcome(tx, input, no_progress_streak, material_progress, &now)?;
    }

    Ok(WorkerGoalOutcomeCommit {
        disposition: WorkerGoalOutcomeCommitDisposition::Inserted,
    })
}

#[derive(Debug)]
struct GoalRunBinding {
    worker_id: String,
    owner_user_id: Option<String>,
    session_id: String,
    kind: String,
    status: String,
    lease_owner: Option<String>,
    lease_token: Option<String>,
    lease_epoch: Option<u64>,
    lease_expires_at: Option<String>,
    attempt_count: u32,
    workflow_goal_id: Option<String>,
    workflow_attempt_id: Option<String>,
    governor_origin: String,
    governor_lane_key: String,
    execution_context: Value,
    config: Value,
    worker_status: String,
    worker_revision: u64,
    worker_model: Option<String>,
    worker_model_key: Option<Value>,
    worker_model_catalog_revision: Option<String>,
    worker_permission_mode: String,
    worker_dm_session_id: Option<String>,
    controller_worker_id: Option<String>,
    controller_owner_user_id: Option<String>,
    controller_session_id: String,
    controller_status: String,
    session_owner_user_id: Option<String>,
    session_type: String,
    workspace_mode: String,
    working_dir: Option<String>,
    project_dir: Option<String>,
    goal_status: String,
    goal_revision: u64,
    plan_revision_id: String,
    plan_revision_number: u64,
    plan_status: String,
    step_id: String,
    step_revision: u64,
    step_status: String,
    step_claimed_attempt_id: Option<String>,
    attempt_status: String,
    attempt_goal_revision: u64,
    attempt_max_turns: u32,
    attempt_max_tool_calls: u32,
    attempt_max_research_actions: u32,
}

fn load_binding(
    tx: &Transaction<'_>,
    run_id: &str,
) -> Result<Option<GoalRunBinding>, WorkerGoalOutcomeCommitError> {
    tx.query_row(
        "SELECT run.worker_id, worker.user_id, run.session_id, run.kind,
                run.status, run.lease_owner, run.lease_token, run.lease_epoch,
                run.lease_expires_at, run.attempt_count, run.workflow_goal_id,
                run.workflow_attempt_id, run.governor_origin,
                run.governor_lane_key, run.execution_context_json,
                run.config_json, worker.status, worker.revision, worker.model,
                worker.model_key_json, worker.model_catalog_revision,
                worker.permission_mode, worker.dm_session_id,
                controller.worker_id, controller.user_id, controller.session_id,
                controller.status, session.user_id, session.session_type,
                session.workspace_mode, session.working_dir, session.project_dir,
                goal.status, goal.revision, plan.id, plan.revision_number,
                plan.status, step.id, step.revision, step.status,
                step.claimed_attempt_id, attempt.status,
                attempt.goal_revision_at_start, attempt.max_turns,
                attempt.max_tool_calls, attempt.max_research_actions
         FROM hive_runs run
         JOIN hive_workers worker ON worker.id = run.worker_id
         JOIN hive_controllers controller ON controller.id = run.controller_id
         JOIN sessions session ON session.id = run.session_id
         JOIN workflow_goals goal ON goal.id = run.workflow_goal_id
         JOIN workflow_execution_attempts attempt
           ON attempt.id = run.workflow_attempt_id
         JOIN workflow_plan_revisions plan ON plan.id = attempt.plan_revision_id
         JOIN workflow_plan_steps step ON step.id = attempt.step_id
         WHERE run.id = ?1",
        [run_id],
        map_binding,
    )
    .optional()
    .map_err(|error| conflict(format!("loading exact Worker Workflow binding: {error}")))
}

fn map_binding(row: &Row<'_>) -> rusqlite::Result<GoalRunBinding> {
    let execution_context_json: String = row.get(14)?;
    let config_json: String = row.get(15)?;
    let worker_model_key_json: Option<String> = row.get(19)?;
    Ok(GoalRunBinding {
        worker_id: row.get(0)?,
        owner_user_id: row.get(1)?,
        session_id: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        lease_owner: row.get(5)?,
        lease_token: row.get(6)?,
        lease_epoch: optional_nonnegative_u64(row, 7)?,
        lease_expires_at: row.get(8)?,
        attempt_count: nonnegative_u32(row, 9)?,
        workflow_goal_id: row.get(10)?,
        workflow_attempt_id: row.get(11)?,
        governor_origin: row.get(12)?,
        governor_lane_key: row.get(13)?,
        execution_context: serde_json::from_str(&execution_context_json)
            .map_err(|error| conversion_error(14, error.to_string()))?,
        config: serde_json::from_str(&config_json)
            .map_err(|error| conversion_error(15, error.to_string()))?,
        worker_status: row.get(16)?,
        worker_revision: nonnegative_u64(row, 17)?,
        worker_model: row.get(18)?,
        worker_model_key: worker_model_key_json
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|error| conversion_error(19, error.to_string()))
            })
            .transpose()?,
        worker_model_catalog_revision: row.get(20)?,
        worker_permission_mode: row.get(21)?,
        worker_dm_session_id: row.get(22)?,
        controller_worker_id: row.get(23)?,
        controller_owner_user_id: row.get(24)?,
        controller_session_id: row.get(25)?,
        controller_status: row.get(26)?,
        session_owner_user_id: row.get(27)?,
        session_type: row.get(28)?,
        workspace_mode: row.get(29)?,
        working_dir: row.get(30)?,
        project_dir: row.get(31)?,
        goal_status: row.get(32)?,
        goal_revision: nonnegative_u64(row, 33)?,
        plan_revision_id: row.get(34)?,
        plan_revision_number: nonnegative_u64(row, 35)?,
        plan_status: row.get(36)?,
        step_id: row.get(37)?,
        step_revision: nonnegative_u64(row, 38)?,
        step_status: row.get(39)?,
        step_claimed_attempt_id: row.get(40)?,
        attempt_status: row.get(41)?,
        attempt_goal_revision: nonnegative_u64(row, 42)?,
        attempt_max_turns: nonnegative_u32(row, 43)?,
        attempt_max_tool_calls: nonnegative_u32(row, 44)?,
        attempt_max_research_actions: nonnegative_u32(row, 45)?,
    })
}

fn validate_binding(
    tx: &Transaction<'_>,
    daemon_fence: &DaemonFence,
    input: &WorkerGoalOutcomeCommitInput,
    binding: &GoalRunBinding,
    now: &str,
) -> Result<(), WorkerGoalOutcomeCommitError> {
    if binding.kind != "worker_workflow"
        || binding.worker_id != input.worker_id()
        || binding.owner_user_id.as_deref() != input.owner_user_id()
        || binding.session_id != input.session_id()
        || binding.workflow_goal_id.as_deref() != Some(input.goal_id())
        || binding.workflow_attempt_id.as_deref() != Some(input.attempt_id())
    {
        return Err(conflict(
            "outcome authority differs from the run's immutable Workflow linkage",
        ));
    }
    if binding.status != "running"
        || binding.lease_owner.as_deref() != Some(daemon_fence.owner_id.as_str())
        || binding.lease_token.as_deref() != Some(input.run_lease_token())
        || binding.lease_epoch != Some(input.run_lease_epoch())
        || binding
            .lease_expires_at
            .as_deref()
            .is_none_or(|value| value <= now)
    {
        return Err(stale("Worker Workflow run lease is no longer current"));
    }
    if binding.worker_status != "active"
        || binding.worker_revision != input.worker_revision()
        || binding.worker_dm_session_id.as_deref() != Some(input.session_id())
        || binding.controller_worker_id.as_deref() != Some(input.worker_id())
        || binding.controller_owner_user_id != binding.owner_user_id
        || binding.controller_session_id != input.session_id()
        || binding.controller_status != "active"
        || binding.session_owner_user_id != binding.owner_user_id
        || binding.session_type != "hive"
    {
        return Err(stale(
            "Worker, controller, owner, or session lifecycle changed",
        ));
    }
    if binding.goal_status != "active"
        || binding.goal_revision != input.goal_revision()
        || binding.goal_revision != input.workflow_aggregate_revision()
        || binding.attempt_status != "running"
        || binding.attempt_goal_revision != input.goal_revision()
        || binding.plan_revision_id != input.plan_revision_id()
        || binding.plan_revision_number != input.plan_revision_number()
        || binding.plan_status != "active"
        || binding.step_id != input.step_id()
        || binding.step_revision != input.step_revision()
        || binding.step_status != "in_progress"
        || binding.step_claimed_attempt_id.as_deref() != Some(input.attempt_id())
    {
        return Err(stale(
            "Goal, attempt, plan, or step revision changed before outcome commit",
        ));
    }
    if binding.workspace_mode != "selected" && binding.workspace_mode != "created" {
        return Err(stale("Worker Goal no longer has an attached workspace"));
    }
    let expected_workspace = input.workspace_dir().to_string_lossy();
    if binding.working_dir.as_deref() != Some(expected_workspace.as_ref())
        || binding.project_dir.as_deref() != Some(expected_workspace.as_ref())
    {
        return Err(stale("Worker Goal workspace binding changed"));
    }
    if binding.governor_lane_key != "dm"
        || binding.governor_origin != input.run_origin().as_str()
        || !matches!(
            binding.governor_origin.as_str(),
            "user_workflow_activation" | "workflow_rollover"
        )
    {
        return Err(conflict("Worker Goal governor origin or lane is invalid"));
    }
    if binding.config.get("model").and_then(Value::as_str) != binding.worker_model.as_deref()
        || binding.config.get("model_key") != binding.worker_model_key.as_ref()
        || binding
            .config
            .get("model_catalog_revision")
            .and_then(Value::as_str)
            != binding.worker_model_catalog_revision.as_deref()
        || binding
            .config
            .get("permission_mode")
            .and_then(Value::as_str)
            != Some(binding.worker_permission_mode.as_str())
    {
        return Err(stale("Worker Goal model or permission binding changed"));
    }
    validate_context(input, binding)?;
    if input.counters().turns > binding.attempt_max_turns
        || input.counters().tool_calls > binding.attempt_max_tool_calls
        || input.counters().research_actions > binding.attempt_max_research_actions
    {
        return Err(conflict(
            "Worker Goal outcome exceeds its frozen attempt budget",
        ));
    }
    let attempt_open = tx
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM hive_run_attempts
                 WHERE run_id = ?1 AND attempt_no = ?2 AND executor_id = ?3
                   AND lease_token = ?4 AND lease_epoch = ?5
                   AND finished_at IS NULL
             )",
            params![
                input.run_id(),
                binding.attempt_count,
                daemon_fence.owner_id,
                input.run_lease_token(),
                input.run_lease_epoch(),
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| conflict(format!("validating open run attempt: {error}")))?;
    if !attempt_open {
        return Err(stale("Worker Workflow run attempt is no longer open"));
    }
    Ok(())
}

fn validate_context(
    input: &WorkerGoalOutcomeCommitInput,
    binding: &GoalRunBinding,
) -> Result<(), WorkerGoalOutcomeCommitError> {
    let mode = binding
        .execution_context
        .get("mode")
        .ok_or_else(|| conflict("Worker Goal execution context has no mode"))?;
    let expected_workspace = input.workspace_dir().to_string_lossy();
    let exact = binding
        .execution_context
        .get("schema_version")
        .and_then(Value::as_u64)
        == Some(1)
        && mode.get("kind").and_then(Value::as_str) == Some("worker_goal")
        && mode.get("worker_id").and_then(Value::as_str) == Some(input.worker_id())
        && mode.get("worker_revision").and_then(Value::as_u64) == Some(input.worker_revision())
        && mode.pointer("/lane/kind").and_then(Value::as_str) == Some("direct_message")
        && mode.get("working_dir").and_then(Value::as_str) == Some(expected_workspace.as_ref())
        && mode.get("project_dir").and_then(Value::as_str) == Some(expected_workspace.as_ref())
        && mode.get("goal_id").and_then(Value::as_str) == Some(input.goal_id())
        && mode.get("goal_revision").and_then(Value::as_u64) == Some(input.goal_revision())
        && mode
            .get("workflow_aggregate_revision")
            .and_then(Value::as_u64)
            == Some(input.workflow_aggregate_revision())
        && mode.get("attempt_id").and_then(Value::as_str) == Some(input.attempt_id())
        && mode.get("plan_revision_id").and_then(Value::as_str) == Some(input.plan_revision_id())
        && mode.get("plan_revision_number").and_then(Value::as_u64)
            == Some(input.plan_revision_number())
        && mode.get("step_id").and_then(Value::as_str) == Some(input.step_id())
        && mode.get("step_revision").and_then(Value::as_u64) == Some(input.step_revision());
    if !exact {
        return Err(conflict(
            "Worker Goal outcome differs from its frozen execution context",
        ));
    }
    Ok(())
}

fn validate_provider_calls(
    tx: &Transaction<'_>,
    input: &WorkerGoalOutcomeCommitInput,
    binding: &GoalRunBinding,
) -> Result<(), WorkerGoalOutcomeCommitError> {
    let mut statement = tx
        .prepare(
            "SELECT call.provider_call_id
             FROM hive_worker_provider_calls call
             WHERE call.run_id = ?1 AND call.call_kind = 'agent_turn'
             ORDER BY call.started_at, call.provider_call_id",
        )
        .map_err(|error| conflict(format!("preparing provider-call validation: {error}")))?;
    let mut persisted = statement
        .query_map([input.run_id()], |row| row.get::<_, String>(0))
        .map_err(|error| conflict(format!("loading provider-call identities: {error}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| conflict(format!("decoding provider-call identities: {error}")))?;
    let mut supplied = input.provider_call_ids().to_vec();
    persisted.sort();
    supplied.sort();
    if persisted != supplied {
        return Err(conflict(
            "Worker Goal outcome omits or invents an AgentTurn provider call",
        ));
    }

    let incompatible_or_unresolved_auxiliary: bool = tx
        .query_row(
            "SELECT EXISTS(
                 SELECT 1
                 FROM hive_worker_provider_calls call
                 LEFT JOIN hive_worker_provider_call_outcomes terminal
                   ON terminal.provider_call_id = call.provider_call_id
                 WHERE call.run_id = ?1
                   AND call.call_kind <> 'agent_turn'
                   AND (
                       call.worker_id <> ?2
                       OR call.worker_revision <> ?3
                       OR call.owner_user_id IS NOT ?4
                       OR call.session_id <> ?5
                       OR call.run_lease_token <> ?6
                       OR call.run_lease_epoch <> ?7
                       OR call.workflow_goal_id IS NOT ?8
                       OR call.workflow_attempt_id IS NOT ?9
                       OR call.origin <> ?10
                       OR call.lane_key <> 'dm'
                       OR terminal.state IS NOT 'completed'
                       OR terminal.remote_acceptance IS NOT 'acknowledged'
                   )
             )",
            params![
                input.run_id(),
                input.worker_id(),
                input.worker_revision(),
                input.owner_user_id(),
                input.session_id(),
                input.run_lease_token(),
                input.run_lease_epoch(),
                input.goal_id(),
                input.attempt_id(),
                binding.governor_origin,
            ],
            |row| row.get(0),
        )
        .map_err(|error| conflict(format!("validating auxiliary provider calls: {error}")))?;
    if incompatible_or_unresolved_auxiliary {
        return Err(conflict(
            "Worker Goal run has an unresolved or incompatible auxiliary provider call",
        ));
    }

    for (index, provider_call_id) in input.provider_call_ids().iter().enumerate() {
        let row = tx
            .query_row(
                "SELECT call.worker_id, call.worker_revision, call.owner_user_id,
                        call.session_id, call.run_id, call.run_lease_token,
                        call.run_lease_epoch, call.workflow_goal_id,
                        call.workflow_attempt_id, call.origin, call.lane_key,
                        call.model_id, call.model_key_json,
                        call.model_catalog_revision, call.permission_mode,
                        call.call_kind, outcome.state, outcome.outcome,
                        outcome.remote_acceptance
                 FROM hive_worker_provider_calls call
                 LEFT JOIN hive_worker_provider_call_outcomes outcome
                   ON outcome.provider_call_id = call.provider_call_id
                 WHERE call.provider_call_id = ?1",
                [provider_call_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        nonnegative_u64(row, 1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        nonnegative_u64(row, 6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, String>(14)?,
                        row.get::<_, String>(15)?,
                        row.get::<_, Option<String>>(16)?,
                        row.get::<_, Option<String>>(17)?,
                        row.get::<_, Option<String>>(18)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| conflict(format!("loading exact provider call: {error}")))?
            .ok_or_else(|| conflict("Worker Goal provider Started row does not exist"))?;
        let exact = row.0 == input.worker_id()
            && row.1 == input.worker_revision()
            && row.2.as_deref() == input.owner_user_id()
            && row.3 == input.session_id()
            && row.4 == input.run_id()
            && row.5 == input.run_lease_token()
            && row.6 == input.run_lease_epoch()
            && row.7.as_deref() == Some(input.goal_id())
            && row.8.as_deref() == Some(input.attempt_id())
            && row.9 == binding.governor_origin
            && row.10 == "dm"
            && binding.worker_model.as_deref() == Some(row.11.as_str())
            && binding.worker_model_key.as_ref()
                == serde_json::from_str::<Value>(&row.12).ok().as_ref()
            && binding.worker_model_catalog_revision == row.13
            && binding.worker_permission_mode == row.14
            && row.15 == "agent_turn";
        if !exact {
            return Err(conflict(
                "provider Started provenance differs from the Worker Goal run",
            ));
        }
        match (row.16.as_deref(), row.17.as_deref(), row.18.as_deref()) {
            (None, None, None) if index + 1 == input.provider_call_ids().len() => {}
            (Some("completed"), Some("completed"), Some("acknowledged")) => {}
            _ => {
                return Err(conflict(
                    "Worker Goal provider call has an unresolved or incompatible outcome",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn apply_trusted_workflow_outcome(
    tx: &Transaction<'_>,
    input: &WorkerGoalOutcomeCommitInput,
    no_progress_streak: u32,
    material_progress: bool,
    now: &str,
) -> Result<(), WorkerGoalOutcomeCommitError> {
    let evidence = input
        .evidence()
        .iter()
        .map(|item| item.summary().to_string())
        .collect::<Vec<_>>();
    let evidence_json = serde_json::to_string(&evidence)
        .map_err(|error| conflict(format!("encoding step evidence: {error}")))?;
    let (attempt_status, attempt_reason) = match input.outcome() {
        WorkerGoalAttemptOutcome::Succeeded => {
            return Err(conflict(
                "Worker Goal step completion requires a structural verifier authority",
            ));
        }
        WorkerGoalAttemptOutcome::Cancelled => ("cancelled", "bounded_attempt_cancelled"),
        WorkerGoalAttemptOutcome::Failed => ("failed", "bounded_attempt_failed"),
        WorkerGoalAttemptOutcome::BudgetExhausted => ("paused", "attempt_budget_exhausted"),
        WorkerGoalAttemptOutcome::NeedsAttention => ("paused", "needs_attention"),
        WorkerGoalAttemptOutcome::Blocked => ("paused", "bounded_attempt_blocked"),
        WorkerGoalAttemptOutcome::Progressed => {
            return Err(conflict(
                "Progressed Worker Goal outcomes require atomic acceptance staging",
            ));
        }
    };
    let changed = tx
        .execute(
            "UPDATE workflow_execution_attempts
             SET status = ?1, stop_reason = ?2, turn_count = ?3,
                 tool_call_count = ?4, research_action_count = ?5,
                 progress_revision = progress_revision + ?6,
                 blocker_fingerprint = ?7, blocker_streak = ?8,
                 ended_at = ?9, updated_at = ?9
             WHERE id = ?10 AND goal_id = ?11 AND status = 'running'
               AND goal_revision_at_start = ?12",
            params![
                attempt_status,
                attempt_reason,
                input.counters().turns,
                input.counters().tool_calls,
                input.counters().research_actions,
                i64::from(material_progress),
                if no_progress_streak > 0 {
                    Some(hash_request_bytes(
                        serde_json::json!({
                            "outcome": outcome_str(input.outcome()),
                            "effect": input.effect(),
                            "evidence": input.evidence(),
                        })
                        .to_string(),
                    ))
                } else {
                    None
                },
                no_progress_streak,
                now,
                input.attempt_id(),
                input.goal_id(),
                input.goal_revision(),
            ],
        )
        .map_err(|error| conflict(format!("finalizing Workflow attempt: {error}")))?;
    if changed != 1 {
        return Err(stale("Workflow attempt changed during outcome commit"));
    }

    let step_status = if no_progress_streak >= 3 {
        "blocked"
    } else {
        "pending"
    };
    let step_changed = tx
        .execute(
            "UPDATE workflow_plan_steps
         SET status = ?1, claimed_attempt_id = NULL, revision = revision + 1,
             outcome = CASE WHEN ?1 = 'blocked' THEN ?2 ELSE outcome END,
             evidence_json = CASE WHEN ?1 = 'blocked' THEN ?3 ELSE evidence_json END
         WHERE id = ?4 AND status = 'in_progress'
           AND claimed_attempt_id = ?5 AND revision = ?6",
            params![
                step_status,
                input.effect().summary(),
                evidence_json,
                input.step_id(),
                input.attempt_id(),
                input.step_revision(),
            ],
        )
        .map_err(|error| conflict(format!("releasing bounded Workflow step: {error}")))?;
    if step_changed != 1 {
        return Err(stale("Workflow step changed during bounded outcome commit"));
    }

    let next_goal_status = if no_progress_streak >= 3 {
        "blocked"
    } else if matches!(
        input.outcome(),
        WorkerGoalAttemptOutcome::NeedsAttention | WorkerGoalAttemptOutcome::BudgetExhausted
    ) {
        "paused"
    } else {
        "active"
    };
    let status_reason = if no_progress_streak >= 3 {
        Some("repeated_no_progress")
    } else {
        match input.outcome() {
            WorkerGoalAttemptOutcome::NeedsAttention => Some("needs_attention"),
            WorkerGoalAttemptOutcome::BudgetExhausted => Some("attempt_budget_exhausted"),
            _ => None,
        }
    };
    let next_revision = input
        .goal_revision()
        .checked_add(1)
        .ok_or_else(|| conflict("Workflow aggregate revision overflow during outcome commit"))?;
    let changed = tx
        .execute(
            "UPDATE workflow_goals
             SET status = ?1, status_reason = ?2, revision = ?3, updated_at = ?4
             WHERE id = ?5 AND session_id = ?6 AND status = 'active'
               AND revision = ?7",
            params![
                next_goal_status,
                status_reason,
                next_revision,
                now,
                input.goal_id(),
                input.session_id(),
                input.goal_revision(),
            ],
        )
        .map_err(|error| conflict(format!("advancing Workflow aggregate: {error}")))?;
    if changed != 1 {
        return Err(stale("Workflow aggregate changed during outcome commit"));
    }
    tx.execute(
        "INSERT INTO workflow_events (
             session_id, goal_id, aggregate_revision, operation_id, event_type,
             actor, attempt_id, payload_json, created_at
         ) VALUES (
             ?1, ?2, ?3, ?4, 'worker_workflow_attempt_committed',
             'hive_worker_runtime', ?5, ?6, ?7
         )",
        params![
            input.session_id(),
            input.goal_id(),
            next_revision,
            format!("worker-goal-outcome:{}", input.run_id()),
            input.attempt_id(),
            serde_json::json!({
                "run_id": input.run_id(),
                "outcome": outcome_str(input.outcome()),
                "material_progress": material_progress,
                "no_progress_streak": no_progress_streak,
                "goal_status": next_goal_status,
            })
            .to_string(),
            now,
        ],
    )
    .map_err(|error| conflict(format!("recording Workflow outcome event: {error}")))?;
    Ok(())
}

pub(crate) fn committed_worker_goal_outcome_in_transaction(
    tx: &Transaction<'_>,
    run_id: &str,
) -> rusqlite::Result<Option<WorkerGoalOutcomeRecord>> {
    tx.query_row(
        "SELECT run_id, worker_id, owner_user_id, session_id,
                workflow_goal_id, workflow_attempt_id, plan_revision_id,
                step_id, workspace_dir, provider_call_ids_json, outcome,
                evidence_json, effect_json, counters_json,
                no_progress_fingerprint, no_progress_streak, committed_at
         FROM hive_worker_goal_outcomes WHERE run_id = ?1",
        [run_id],
        |row| {
            let provider_call_ids_json: String = row.get(9)?;
            let evidence_json: String = row.get(11)?;
            let effect_json: String = row.get(12)?;
            let counters_json: String = row.get(13)?;
            let outcome: String = row.get(10)?;
            Ok(WorkerGoalOutcomeRecord {
                run_id: row.get(0)?,
                worker_id: row.get(1)?,
                owner_user_id: row.get(2)?,
                session_id: row.get(3)?,
                workflow_goal_id: row.get(4)?,
                workflow_attempt_id: row.get(5)?,
                plan_revision_id: row.get(6)?,
                step_id: row.get(7)?,
                workspace_dir: row.get(8)?,
                provider_call_ids: serde_json::from_str(&provider_call_ids_json)
                    .map_err(|error| conversion_error(9, error.to_string()))?,
                outcome: parse_outcome(&outcome)
                    .ok_or_else(|| conversion_error(10, "invalid Worker Goal outcome"))?,
                evidence: serde_json::from_str(&evidence_json)
                    .map_err(|error| conversion_error(11, error.to_string()))?,
                effect: serde_json::from_str(&effect_json)
                    .map_err(|error| conversion_error(12, error.to_string()))?,
                counters: serde_json::from_str(&counters_json)
                    .map_err(|error| conversion_error(13, error.to_string()))?,
                no_progress_fingerprint: row.get(14)?,
                no_progress_streak: nonnegative_u32(row, 15)?,
                committed_at: row.get(16)?,
            })
        },
    )
    .optional()
}

/// A committed typed result becomes terminal run authority only after every
/// exact provider Started row named by that immutable result has an
/// acknowledged terminal outcome.  This is intentionally separate from
/// outcome commit because the held final provider permit is completed after
/// the SQLite result transaction commits.
pub(crate) fn worker_goal_outcome_is_accounted_in_transaction(
    tx: &Transaction<'_>,
    run_id: &str,
) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM hive_worker_goal_outcomes outcome
             JOIN hive_runs run ON run.id = outcome.run_id
             WHERE outcome.run_id = ?1
               AND run.kind = 'worker_workflow'
               AND run.worker_id = outcome.worker_id
               AND run.session_id = outcome.session_id
               AND run.workflow_goal_id = outcome.workflow_goal_id
               AND run.workflow_attempt_id = outcome.workflow_attempt_id
               AND (
                   outcome.outcome <> 'progressed'
                   OR EXISTS (
                       SELECT 1
                       FROM hive_worker_goal_acceptance_candidates candidate
                       JOIN hive_runs acceptance_run
                         ON acceptance_run.id = candidate.acceptance_run_id
                       WHERE candidate.source_run_id = outcome.run_id
                         AND candidate.worker_id = outcome.worker_id
                         AND candidate.owner_user_id IS outcome.owner_user_id
                         AND candidate.session_id = outcome.session_id
                         AND candidate.workflow_goal_id = outcome.workflow_goal_id
                         AND candidate.source_attempt_id = outcome.workflow_attempt_id
                         AND candidate.plan_revision_id = outcome.plan_revision_id
                         AND candidate.step_id = outcome.step_id
                         AND candidate.workspace_dir = outcome.workspace_dir
                         AND acceptance_run.kind = 'worker_workflow_acceptance'
                   )
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM json_each(outcome.provider_call_ids_json) call_id
                   LEFT JOIN hive_worker_provider_calls call
                     ON call.provider_call_id = call_id.value
                   LEFT JOIN hive_worker_provider_call_outcomes terminal
                     ON terminal.provider_call_id = call.provider_call_id
                   WHERE call.provider_call_id IS NULL
                      OR call.run_id <> run.id
                      OR call.workflow_goal_id IS NOT run.workflow_goal_id
                      OR call.workflow_attempt_id IS NOT run.workflow_attempt_id
                      OR terminal.state IS NOT 'completed'
                      OR terminal.outcome IS NOT 'completed'
                      OR terminal.remote_acceptance IS NOT 'acknowledged'
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM hive_worker_provider_calls call
                   LEFT JOIN hive_worker_provider_call_outcomes terminal
                     ON terminal.provider_call_id = call.provider_call_id
                   WHERE call.run_id = run.id
                     AND (
                         call.worker_id <> run.worker_id
                         OR call.session_id <> run.session_id
                         OR call.workflow_goal_id IS NOT run.workflow_goal_id
                         OR call.workflow_attempt_id IS NOT run.workflow_attempt_id
                         OR terminal.state IS NOT 'completed'
                         OR terminal.remote_acceptance IS NOT 'acknowledged'
                         OR (
                             call.call_kind = 'agent_turn'
                             AND NOT EXISTS (
                                 SELECT 1
                                 FROM json_each(outcome.provider_call_ids_json) listed
                                 WHERE listed.value = call.provider_call_id
                             )
                         )
                     )
               )
         )",
        [run_id],
        |row| row.get(0),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerWorkflowProviderRecovery {
    CanonicalOutcomeAdopted,
    SafeBeforeProviderBoundary,
    ProviderBoundaryWithoutOutcome,
    CommittedOutcomeUnaccounted,
    NotWorkerWorkflow,
}

/// Reconcile the provider crash window for one exact Worker Workflow claim.
/// A committed outcome proves that the held final AgentTurn response was
/// received, so that one Started row may be terminalized as acknowledged after
/// a crash. No other unresolved call is inferred: auxiliary or earlier calls
/// become Unknown and force explicit recovery.
pub fn reconcile_worker_workflow_provider_boundary_in_transaction(
    tx: &Transaction<'_>,
    run_id: &str,
    lease_token: &str,
    lease_epoch: u64,
    now: &str,
) -> rusqlite::Result<WorkerWorkflowProviderRecovery> {
    let run: Option<(String, String, String, String, String)> = tx
        .query_row(
            "SELECT kind, worker_id, session_id, workflow_goal_id,
                    workflow_attempt_id
             FROM hive_runs WHERE id = ?1",
            [run_id],
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
        .optional()?;
    let Some((kind, worker_id, session_id, goal_id, attempt_id)) = run else {
        return Ok(WorkerWorkflowProviderRecovery::NotWorkerWorkflow);
    };
    if kind != "worker_workflow" {
        return Ok(WorkerWorkflowProviderRecovery::NotWorkerWorkflow);
    }

    let call_count: u64 = tx.query_row(
        "SELECT COUNT(*) FROM hive_worker_provider_calls WHERE run_id = ?1",
        [run_id],
        |row| nonnegative_u64(row, 0),
    )?;
    let committed = committed_worker_goal_outcome_in_transaction(tx, run_id)?;
    let committed_has_required_acceptance = committed
        .as_ref()
        .map(|outcome| worker_goal_outcome_has_required_acceptance_in_transaction(tx, outcome))
        .transpose()?
        .unwrap_or(false);
    if call_count == 0 {
        return Ok(if committed.is_none() {
            WorkerWorkflowProviderRecovery::SafeBeforeProviderBoundary
        } else {
            WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted
        });
    }

    let invalid_binding: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM hive_worker_provider_calls call
             WHERE call.run_id = ?1
               AND (
                   call.worker_id <> ?2
                   OR call.session_id <> ?3
                   OR call.workflow_goal_id IS NOT ?4
                   OR call.workflow_attempt_id IS NOT ?5
                   OR call.run_lease_token <> ?6
                   OR call.run_lease_epoch <> ?7
               )
         )",
        params![
            run_id,
            worker_id,
            session_id,
            goal_id,
            attempt_id,
            lease_token,
            lease_epoch,
        ],
        |row| row.get(0),
    )?;

    if committed.is_none() || invalid_binding || !committed_has_required_acceptance {
        tx.execute(
            "INSERT OR IGNORE INTO hive_worker_provider_call_outcomes (
                 provider_call_id, state, outcome, remote_acceptance,
                 unknown_reason, finished_at
             )
             SELECT call.provider_call_id, 'unknown',
                    'worker_workflow_interrupted', 'possibly_sent',
                    'Worker Workflow crossed a provider boundary without an exact committed outcome',
                    ?2
             FROM hive_worker_provider_calls call
             WHERE call.run_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_provider_call_outcomes terminal
                   WHERE terminal.provider_call_id = call.provider_call_id
               )",
            params![run_id, now],
        )?;
        return Ok(if committed.is_none() {
            WorkerWorkflowProviderRecovery::ProviderBoundaryWithoutOutcome
        } else {
            WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted
        });
    }

    let committed = committed.expect("checked committed Worker Goal outcome");
    let mut started_agent_turns = {
        let mut statement = tx.prepare(
            "SELECT provider_call_id FROM hive_worker_provider_calls
             WHERE run_id = ?1 AND call_kind = 'agent_turn'
             ORDER BY provider_call_id",
        )?;
        let rows = statement
            .query_map([run_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut committed_agent_turns = committed.provider_call_ids.clone();
    started_agent_turns.sort();
    committed_agent_turns.sort();
    if started_agent_turns != committed_agent_turns {
        return Ok(WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted);
    }

    let incompatible_terminal: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM hive_worker_provider_calls call
             JOIN hive_worker_provider_call_outcomes terminal
               ON terminal.provider_call_id = call.provider_call_id
             WHERE call.run_id = ?1
               AND (terminal.state IS NOT 'completed'
                    OR (call.call_kind = 'agent_turn'
                        AND terminal.outcome IS NOT 'completed')
                    OR terminal.remote_acceptance IS NOT 'acknowledged')
         )",
        [run_id],
        |row| row.get(0),
    )?;
    if incompatible_terminal {
        return Ok(WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted);
    }

    let unresolved = {
        let mut statement = tx.prepare(
            "SELECT call.provider_call_id, call.call_kind
             FROM hive_worker_provider_calls call
             LEFT JOIN hive_worker_provider_call_outcomes terminal
               ON terminal.provider_call_id = call.provider_call_id
             WHERE call.run_id = ?1 AND terminal.provider_call_id IS NULL
             ORDER BY call.started_at, call.provider_call_id",
        )?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    if unresolved.len() == 1
        && unresolved[0].0
            == committed
                .provider_call_ids
                .last()
                .cloned()
                .unwrap_or_default()
        && unresolved[0].1 == "agent_turn"
    {
        tx.execute(
            "INSERT INTO hive_worker_provider_call_outcomes (
                 provider_call_id, state, outcome, remote_acceptance,
                 usage_json, usage_total_tokens, estimated_cost_microunits,
                 unknown_reason, finished_at
             ) VALUES (?1, 'completed', 'completed', 'acknowledged',
                       NULL, NULL, NULL, NULL, ?2)",
            params![unresolved[0].0, now],
        )?;
    } else if !unresolved.is_empty() {
        tx.execute(
            "INSERT OR IGNORE INTO hive_worker_provider_call_outcomes (
                 provider_call_id, state, outcome, remote_acceptance,
                 unknown_reason, finished_at
             )
             SELECT call.provider_call_id, 'unknown',
                    'worker_workflow_interrupted', 'possibly_sent',
                    'unresolved Worker Workflow provider call is not the committed final AgentTurn',
                    ?2
             FROM hive_worker_provider_calls call
             WHERE call.run_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM hive_worker_provider_call_outcomes terminal
                   WHERE terminal.provider_call_id = call.provider_call_id
               )",
            params![run_id, now],
        )?;
        return Ok(WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted);
    }

    if worker_goal_outcome_is_accounted_in_transaction(tx, run_id)? {
        Ok(WorkerWorkflowProviderRecovery::CanonicalOutcomeAdopted)
    } else {
        Ok(WorkerWorkflowProviderRecovery::CommittedOutcomeUnaccounted)
    }
}

fn worker_goal_outcome_has_required_acceptance_in_transaction(
    tx: &Transaction<'_>,
    outcome: &WorkerGoalOutcomeRecord,
) -> rusqlite::Result<bool> {
    if outcome.outcome != WorkerGoalAttemptOutcome::Progressed {
        return Ok(true);
    }
    tx.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM hive_worker_goal_acceptance_candidates candidate
             JOIN hive_runs acceptance_run
               ON acceptance_run.id = candidate.acceptance_run_id
             WHERE candidate.source_run_id = ?1
               AND candidate.worker_id = ?2
               AND candidate.owner_user_id IS ?3
               AND candidate.session_id = ?4
               AND candidate.workflow_goal_id = ?5
               AND candidate.source_attempt_id = ?6
               AND candidate.plan_revision_id = ?7
               AND candidate.step_id = ?8
               AND candidate.workspace_dir = ?9
               AND acceptance_run.kind = 'worker_workflow_acceptance'
         )",
        params![
            outcome.run_id,
            outcome.worker_id,
            outcome.owner_user_id,
            outcome.session_id,
            outcome.workflow_goal_id,
            outcome.workflow_attempt_id,
            outcome.plan_revision_id,
            outcome.step_id,
            outcome.workspace_dir,
        ],
        |row| row.get(0),
    )
}

/// Couple an uncertain expired Worker Workflow run to its canonical Workflow
/// attempt.  This never creates a rollover: the user must explicitly resolve
/// the recovery-required run before more workspace effects can occur.
pub(crate) fn pause_worker_workflow_after_uncertain_run_in_transaction(
    tx: &Transaction<'_>,
    run_id: &str,
    now: &str,
) -> rusqlite::Result<bool> {
    let binding: Option<(String, String, String, u64)> = tx
        .query_row(
            "SELECT run.workflow_attempt_id, run.workflow_goal_id,
                    goal.session_id, goal.revision
             FROM hive_runs run
             JOIN workflow_goals goal ON goal.id = run.workflow_goal_id
             WHERE run.id = ?1 AND run.kind = 'worker_workflow'",
            [run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    nonnegative_u64(row, 3)?,
                ))
            },
        )
        .optional()?;
    let Some((attempt_id, goal_id, session_id, revision)) = binding else {
        return Ok(false);
    };
    let attempt_changed = tx.execute(
        "UPDATE workflow_execution_attempts
         SET status = 'paused', stop_reason = 'side_effects_uncertain',
             ended_at = ?2, updated_at = ?2
         WHERE id = ?1 AND status = 'running'",
        params![attempt_id, now],
    )?;
    if attempt_changed == 0 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE workflow_plan_steps
         SET status = 'blocked', claimed_attempt_id = NULL, revision = revision + 1
         WHERE claimed_attempt_id = ?1 AND status = 'in_progress'",
        [attempt_id.as_str()],
    )?;
    let next_revision = revision.saturating_add(1);
    tx.execute(
        "UPDATE workflow_goals
         SET status = 'paused', status_reason = 'side_effects_uncertain',
             revision = ?1, updated_at = ?2
         WHERE id = ?3 AND revision = ?4",
        params![next_revision, now, goal_id, revision],
    )?;
    tx.execute(
        "INSERT INTO workflow_events (
             session_id, goal_id, aggregate_revision, operation_id, event_type,
             actor, attempt_id, payload_json, created_at
         ) VALUES (
             ?1, ?2, ?3, ?4, 'worker_workflow_recovery_required',
             'hive_runtime', ?5, ?6, ?7
         )",
        params![
            session_id,
            goal_id,
            next_revision,
            format!("worker-goal-recovery:{run_id}"),
            attempt_id,
            serde_json::json!({
                "run_id": run_id,
                "stop_reason": "side_effects_uncertain",
            })
            .to_string(),
            now,
        ],
    )?;
    Ok(true)
}

fn outcome_str(outcome: WorkerGoalAttemptOutcome) -> &'static str {
    match outcome {
        WorkerGoalAttemptOutcome::Succeeded => "succeeded",
        WorkerGoalAttemptOutcome::Progressed => "progressed",
        WorkerGoalAttemptOutcome::Blocked => "blocked",
        WorkerGoalAttemptOutcome::Failed => "failed",
        WorkerGoalAttemptOutcome::Cancelled => "cancelled",
        WorkerGoalAttemptOutcome::BudgetExhausted => "budget_exhausted",
        WorkerGoalAttemptOutcome::NeedsAttention => "needs_attention",
    }
}

fn parse_outcome(value: &str) -> Option<WorkerGoalAttemptOutcome> {
    match value {
        "succeeded" => Some(WorkerGoalAttemptOutcome::Succeeded),
        "progressed" => Some(WorkerGoalAttemptOutcome::Progressed),
        "blocked" => Some(WorkerGoalAttemptOutcome::Blocked),
        "failed" => Some(WorkerGoalAttemptOutcome::Failed),
        "cancelled" => Some(WorkerGoalAttemptOutcome::Cancelled),
        "budget_exhausted" => Some(WorkerGoalAttemptOutcome::BudgetExhausted),
        "needs_attention" => Some(WorkerGoalAttemptOutcome::NeedsAttention),
        _ => None,
    }
}

fn nonnegative_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| conversion_error(index, "negative integer"))
}

fn nonnegative_u32(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| conversion_error(index, "integer exceeds u32"))
}

fn optional_nonnegative_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| u64::try_from(value).map_err(|_| conversion_error(index, "negative integer")))
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

fn stale(reason: impl Into<String>) -> WorkerGoalOutcomeCommitError {
    WorkerGoalOutcomeCommitError::StaleRejected(bounded(reason.into()))
}

fn conflict(reason: impl Into<String>) -> WorkerGoalOutcomeCommitError {
    WorkerGoalOutcomeCommitError::ConflictOrCorrupt(bounded(reason.into()))
}

fn bounded(mut value: String) -> String {
    if value.len() <= MAX_ERROR_BYTES {
        return value;
    }
    let mut boundary = MAX_ERROR_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}
