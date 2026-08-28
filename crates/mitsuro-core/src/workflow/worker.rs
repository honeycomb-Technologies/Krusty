use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hive::canonical_timestamp;
use crate::storage::{
    pending_worker_goal_acceptance_exists_in_transaction,
    terminalize_pending_worker_goal_acceptances_in_transaction,
    worker_goal_outcome_is_accounted_in_transaction, DaemonFence, HiveRunExecutionContextV1,
    WorkerGoalAcceptanceLifecycle, WorkerGoalAcceptanceStageError, WorkerRunOrigin, WorkspaceMode,
};

use super::{
    WorkflowError, WorkflowManager, DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS,
    DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS, DEFAULT_GOAL_ATTEMPT_MAX_TURNS,
    DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS,
};

const WORKER_GOAL_TOOLS: [&str; 8] = [
    "apply_patch",
    "bash",
    "edit",
    "glob",
    "grep",
    "list",
    "multiedit",
    "read",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerWorkflowActivationSource {
    UserActivation,
    WorkflowRollover,
}

impl WorkerWorkflowActivationSource {
    fn origin(self) -> WorkerRunOrigin {
        match self {
            Self::UserActivation => WorkerRunOrigin::UserWorkflowActivation,
            Self::WorkflowRollover => WorkerRunOrigin::WorkflowRollover,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerWorkflowActivationRequest {
    pub worker_id: String,
    pub expected_worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub goal_id: String,
    pub expected_goal_revision: u64,
    pub operation_id: String,
    pub source: WorkerWorkflowActivationSource,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerWorkflowActivationDisposition {
    Created,
    Existing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerWorkflowActivation {
    pub disposition: WorkerWorkflowActivationDisposition,
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub controller_id: String,
    pub run_id: String,
    pub run_status: String,
    pub workflow_goal_id: String,
    pub workflow_attempt_id: String,
    pub workflow_attempt_status: String,
    pub goal_status: String,
    pub goal_revision: u64,
    /// Canonically equal to `goal_revision` in Workflow schema v1.  It remains
    /// explicit so a future aggregate/event revision can evolve safely.
    pub workflow_aggregate_revision: u64,
    pub plan_revision_id: String,
    pub plan_revision_number: u64,
    pub step_id: String,
    pub step_revision: u64,
    pub workspace_dir: String,
    pub worker_revision: u64,
    pub governor_origin: WorkerRunOrigin,
    pub execution_context: Value,
}

#[derive(Debug, Clone)]
pub struct WorkerWorkflowLifecycleRequest {
    pub worker_id: String,
    pub expected_worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub goal_id: String,
    pub expected_goal_revision: u64,
    pub operation_id: String,
    pub reason: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerWorkflowLifecycleResult {
    pub changed: bool,
    pub worker_id: String,
    pub worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub workflow_goal_id: String,
    pub goal_revision: u64,
    pub goal_status: String,
    pub affected_run_ids: Vec<String>,
    pub affected_attempt_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerWorkflowReconciliation {
    pub run_id: String,
    pub workflow_goal_id: String,
    pub workflow_attempt_id: String,
    pub run_status: String,
    pub goal_status: String,
    pub tokens_used: u64,
    pub recovery_required: bool,
}

impl WorkflowManager {
    /// Activate or resume one Worker-owned Goal and atomically create its next
    /// bounded attempt/run.  The Worker's private DM session is the Goal's
    /// ownership boundary.  A neutral session returns `WorkspaceRequired`;
    /// the daemon's cwd is never consulted.
    pub fn activate_or_resume_worker_workflow(
        &self,
        request: WorkerWorkflowActivationRequest,
    ) -> Result<WorkerWorkflowActivation, WorkflowError> {
        validate_activation_request(&request)?;
        let db = self
            .db
            .lock()
            .map_err(|error| WorkflowError::Database(error.to_string()))?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let result = activate_or_resume_worker_workflow_in_transaction(&tx, &request)?;
        tx.commit()?;
        Ok(result)
    }

    /// After a run reaches a durable terminal state, create at most one fresh
    /// rollover for an active Goal with remaining dependency-ready work.
    /// Nothing is spliced into the previous attempt.
    pub fn finalize_worker_workflow_attempt(
        &self,
        daemon_fence: &DaemonFence,
        worker_id: &str,
        owner_user_id: Option<&str>,
        run_id: &str,
        operation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<WorkerWorkflowActivation>, WorkflowError> {
        let db = self
            .db
            .lock()
            .map_err(|error| WorkflowError::Database(error.to_string()))?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let result = finalize_worker_workflow_attempt_in_transaction(
            &tx,
            daemon_fence,
            worker_id,
            owner_user_id,
            run_id,
            operation_id,
            now,
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn pause_worker_workflow(
        &self,
        request: WorkerWorkflowLifecycleRequest,
    ) -> Result<WorkerWorkflowLifecycleResult, WorkflowError> {
        let db = self
            .db
            .lock()
            .map_err(|error| WorkflowError::Database(error.to_string()))?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let result = pause_worker_workflow_in_transaction(&tx, &request)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn cancel_worker_workflow(
        &self,
        request: WorkerWorkflowLifecycleRequest,
    ) -> Result<WorkerWorkflowLifecycleResult, WorkflowError> {
        let db = self
            .db
            .lock()
            .map_err(|error| WorkflowError::Database(error.to_string()))?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let result = cancel_worker_workflow_in_transaction(&tx, &request)?;
        tx.commit()?;
        Ok(result)
    }

    /// Reconcile provider usage and the coupled recovery state for one exact
    /// run.  Usage is recomputed from the append-only provider ledger, never
    /// from the conversation session token counter.
    pub fn reconcile_worker_workflow_run(
        &self,
        daemon_fence: &DaemonFence,
        run_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<WorkerWorkflowReconciliation>, WorkflowError> {
        let db = self
            .db
            .lock()
            .map_err(|error| WorkflowError::Database(error.to_string()))?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let result = reconcile_worker_workflow_run_in_transaction(&tx, daemon_fence, run_id, now)?;
        tx.commit()?;
        Ok(result)
    }

    /// Materialize a bounded batch of crash-safe continuations whose source
    /// run committed successfully before the runtime could call `finalize`.
    pub fn materialize_due_worker_workflow_rollovers(
        &self,
        daemon_fence: &DaemonFence,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<WorkerWorkflowActivation>, WorkflowError> {
        let db = self
            .db
            .lock()
            .map_err(|error| WorkflowError::Database(error.to_string()))?;
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate)?;
        let result = materialize_due_worker_workflow_rollovers_in_transaction(
            &tx,
            daemon_fence,
            limit,
            now,
        )?;
        tx.commit()?;
        Ok(result)
    }
}

pub fn reconcile_worker_workflow_run_in_transaction(
    tx: &Transaction<'_>,
    daemon_fence: &DaemonFence,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<WorkerWorkflowReconciliation>, WorkflowError> {
    let timestamp = canonical_timestamp(now);
    ensure_current_daemon_fence(tx, daemon_fence, &timestamp)?;
    let row: Option<(String, String, String, String, u64, Option<u64>)> = tx
        .query_row(
            "SELECT run.workflow_goal_id, run.workflow_attempt_id,
                    run.status, goal.status, goal.revision, goal.token_budget
             FROM hive_runs run
             JOIN workflow_goals goal ON goal.id = run.workflow_goal_id
             WHERE run.id = ?1 AND run.kind = 'worker_workflow'",
            [run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    nonnegative_u64(row, 4)?,
                    optional_nonnegative_u64(row, 5)?,
                ))
            },
        )
        .optional()?;
    let Some((goal_id, attempt_id, run_status, mut goal_status, revision, token_budget)) = row
    else {
        return Ok(None);
    };
    let tokens_used: u64 = tx.query_row(
        "SELECT COALESCE(SUM(
             CASE
                 WHEN terminal.usage_total_tokens IS NOT NULL
                     THEN terminal.usage_total_tokens
                 ELSE call.reserved_tokens
             END
         ), 0)
         FROM hive_worker_provider_calls call
         LEFT JOIN hive_worker_provider_call_outcomes terminal
           ON terminal.provider_call_id = call.provider_call_id
         WHERE call.workflow_goal_id = ?1",
        [goal_id.as_str()],
        |row| nonnegative_u64(row, 0),
    )?;
    tx.execute(
        "UPDATE workflow_goals SET tokens_used = ?2, updated_at = ?3
         WHERE id = ?1",
        params![goal_id, tokens_used, timestamp],
    )?;
    if goal_status == "active" && token_budget.is_some_and(|limit| tokens_used >= limit) {
        let next_revision = revision
            .checked_add(1)
            .ok_or_else(|| WorkflowError::Validation("workflow revision overflow".to_string()))?;
        tx.execute(
            "UPDATE workflow_goals
             SET status = 'paused', status_reason = 'token_budget_exhausted',
                 revision = ?2, updated_at = ?3
             WHERE id = ?1 AND revision = ?4",
            params![goal_id, next_revision, timestamp, revision],
        )?;
        goal_status = "paused".to_string();
    }
    let recovery_required = run_status == "recovery_required";
    if recovery_required {
        tx.execute(
            "UPDATE workflow_execution_attempts
             SET status = 'paused', stop_reason = 'side_effects_uncertain',
                 ended_at = COALESCE(ended_at, ?2), updated_at = ?2
             WHERE id = ?1 AND status = 'running'",
            params![attempt_id, timestamp],
        )?;
    }
    Ok(Some(WorkerWorkflowReconciliation {
        run_id: run_id.to_string(),
        workflow_goal_id: goal_id,
        workflow_attempt_id: attempt_id,
        run_status,
        goal_status,
        tokens_used,
        recovery_required,
    }))
}

pub fn activate_or_resume_worker_workflow_in_transaction(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowActivationRequest,
) -> Result<WorkerWorkflowActivation, WorkflowError> {
    validate_activation_request(request)?;
    if let Some(receipt) = load_activation_receipt(tx, request)? {
        return Ok(receipt);
    }
    if let Some(existing) = load_existing_nonterminal(tx, request)? {
        store_activation_receipt(tx, request, &existing)?;
        return Ok(existing);
    }

    let worker: Option<(
        Option<String>,
        u64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        u64,
    )> = tx
        .query_row(
            "SELECT worker.user_id, worker.revision, worker.status,
                    worker.dm_session_id, worker.model, worker.model_key_json,
                    worker.model_catalog_revision, worker.permission_mode,
                    controller.id, controller.status, session.working_dir,
                    session.project_dir, session.workspace_mode, policy.revision
             FROM hive_workers worker
             JOIN hive_controllers controller ON controller.worker_id = worker.id
             JOIN sessions session ON session.id = worker.dm_session_id
             JOIN hive_worker_governor_policies policy ON policy.worker_id = worker.id
             WHERE worker.id = ?1
               AND controller.session_id = session.id
               AND controller.user_id IS worker.user_id
               AND session.user_id IS worker.user_id
               AND session.session_type = 'hive'",
            [request.worker_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    nonnegative_u64(row, 1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    nonnegative_u64(row, 13)?,
                ))
            },
        )
        .optional()?;
    let Some((
        worker_owner,
        worker_revision,
        worker_status,
        session_id,
        model,
        model_key_json,
        model_catalog_revision,
        permission_mode,
        controller_id,
        controller_status,
        working_dir,
        project_dir,
        workspace_mode,
        policy_revision,
    )) = worker
    else {
        return Err(WorkflowError::NotFound(format!(
            "Hive Worker {}",
            request.worker_id
        )));
    };
    if worker_owner != request.owner_user_id {
        return Err(WorkflowError::Conflict(
            "Worker Workflow owner mismatch".to_string(),
        ));
    }
    if worker_revision != request.expected_worker_revision {
        return Err(WorkflowError::Conflict(format!(
            "expected Worker revision {}, current revision is {worker_revision}",
            request.expected_worker_revision
        )));
    }
    if worker_status != "active" || controller_status != "active" {
        return Err(WorkflowError::InvalidTransition(
            "Worker and controller must be active".to_string(),
        ));
    }
    let session_id = session_id
        .ok_or_else(|| WorkflowError::Conflict("Worker has no private DM session".to_string()))?;
    let introduction_ready: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM hive_worker_introductions
             WHERE worker_id = ?1 AND status IN ('confirmed', 'skipped')
         )",
        [request.worker_id.as_str()],
        |row| row.get(0),
    )?;
    if !introduction_ready {
        return Err(WorkflowError::InvalidTransition(
            "Worker Introduction must be confirmed or skipped before Goal activation".to_string(),
        ));
    }
    if workspace_mode != "selected" && workspace_mode != "created" {
        return Err(WorkflowError::WorkspaceRequired(
            "the Worker DM session is workspace-neutral".to_string(),
        ));
    }
    let workspace_dir = working_dir
        .ok_or_else(|| WorkflowError::WorkspaceRequired("working_dir is missing".to_string()))?;
    if !valid_absolute_path(&workspace_dir)
        || project_dir.as_deref() != Some(workspace_dir.as_str())
    {
        return Err(WorkflowError::WorkspaceRequired(
            "working_dir and project_dir must be the same absolute path".to_string(),
        ));
    }
    let model = model.ok_or_else(|| {
        WorkflowError::Validation("Worker Goal requires an exact model".to_string())
    })?;
    let model_key_json = model_key_json.ok_or_else(|| {
        WorkflowError::Validation("Worker Goal requires an exact provider/model key".to_string())
    })?;
    let model_key: Value = serde_json::from_str(&model_key_json)?;

    let goal: Option<(String, String, u64, bool, Option<u64>)> = tx
        .query_row(
            "SELECT session_id, status, revision, needs_definition, token_budget
             FROM workflow_goals WHERE id = ?1",
            [request.goal_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    nonnegative_u64(row, 2)?,
                    row.get(3)?,
                    optional_nonnegative_u64(row, 4)?,
                ))
            },
        )
        .optional()?;
    let Some((goal_session_id, goal_status, goal_revision, needs_definition, token_budget)) = goal
    else {
        return Err(WorkflowError::NotFound(format!("goal {}", request.goal_id)));
    };
    if goal_session_id != session_id {
        return Err(WorkflowError::Conflict(
            "Goal is not owned by this Worker's private DM session".to_string(),
        ));
    }
    if goal_revision != request.expected_goal_revision {
        return Err(WorkflowError::Conflict(format!(
            "expected workflow revision {}, current revision is {goal_revision}",
            request.expected_goal_revision
        )));
    }
    if needs_definition
        || !matches!(
            goal_status.as_str(),
            "draft" | "active" | "paused" | "blocked"
        )
    {
        return Err(WorkflowError::InvalidTransition(format!(
            "cannot activate {goal_status} Worker Goal"
        )));
    }
    if pending_worker_goal_acceptance_exists_in_transaction(tx, &request.goal_id)
        .map_err(map_acceptance_lifecycle_error)?
    {
        return Err(WorkflowError::InvalidTransition(
            "Worker Goal is awaiting an explicit acceptance decision; activating another attempt would escape the frozen candidate"
                .to_string(),
        ));
    }
    let authoritative_tokens_used: u64 = tx.query_row(
        "SELECT COALESCE(SUM(
             CASE
                 WHEN terminal.usage_total_tokens IS NOT NULL
                     THEN terminal.usage_total_tokens
                 ELSE call.reserved_tokens
             END
         ), 0)
         FROM hive_worker_provider_calls call
         LEFT JOIN hive_worker_provider_call_outcomes terminal
           ON terminal.provider_call_id = call.provider_call_id
         WHERE call.workflow_goal_id = ?1",
        [request.goal_id.as_str()],
        |row| nonnegative_u64(row, 0),
    )?;
    if token_budget.is_some_and(|limit| authoritative_tokens_used >= limit) {
        return Err(WorkflowError::InvalidTransition(
            "Worker Goal token budget is exhausted".to_string(),
        ));
    }
    let criteria_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM workflow_goal_criteria WHERE goal_id = ?1",
        [request.goal_id.as_str()],
        |row| row.get(0),
    )?;
    if criteria_count == 0 {
        return Err(WorkflowError::Validation(
            "Worker Goal requires verification criteria".to_string(),
        ));
    }

    let plan: Option<(String, u64)> = tx
        .query_row(
            "SELECT id, revision_number FROM workflow_plan_revisions
             WHERE goal_id = ?1 AND status = 'active'",
            [request.goal_id.as_str()],
            |row| Ok((row.get(0)?, nonnegative_u64(row, 1)?)),
        )
        .optional()?;
    let Some((plan_revision_id, plan_revision_number)) = plan else {
        return Err(WorkflowError::InvalidTransition(
            "Worker Goal requires exactly one active plan revision".to_string(),
        ));
    };
    let step: Option<(String, u64)> = tx
        .query_row(
            "SELECT step.id, step.revision
             FROM workflow_plan_steps step
             WHERE step.plan_revision_id = ?1
               AND step.status IN ('pending', 'blocked')
               AND NOT EXISTS (
                   SELECT 1
                   FROM workflow_step_dependencies dependency
                   JOIN workflow_plan_steps prerequisite
                     ON prerequisite.id = dependency.depends_on_step_id
                   WHERE dependency.step_id = step.id
                     AND prerequisite.plan_revision_id = step.plan_revision_id
                     AND prerequisite.status NOT IN ('completed', 'skipped')
               )
             ORDER BY step.position, step.id LIMIT 1",
            [plan_revision_id.as_str()],
            |row| Ok((row.get(0)?, nonnegative_u64(row, 1)?)),
        )
        .optional()?;
    let Some((step_id, previous_step_revision)) = step else {
        return Err(WorkflowError::InvalidTransition(
            "Worker Goal has no dependency-ready step".to_string(),
        ));
    };

    let next_goal_revision = goal_revision
        .checked_add(1)
        .ok_or_else(|| WorkflowError::Validation("workflow revision overflow".to_string()))?;
    let next_step_revision = previous_step_revision
        .checked_add(1)
        .ok_or_else(|| WorkflowError::Validation("workflow step revision overflow".to_string()))?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let timestamp = canonical_timestamp(request.now);
    let origin = request.source.origin();

    let goal_changed = tx.execute(
        "UPDATE workflow_goals
         SET status = 'active', status_reason = NULL,
             revision = ?1, activated_at = COALESCE(activated_at, ?2),
             tokens_used = ?6, updated_at = ?2
         WHERE id = ?3 AND session_id = ?4 AND revision = ?5
           AND status IN ('draft', 'active', 'paused', 'blocked')",
        params![
            next_goal_revision,
            timestamp,
            request.goal_id,
            session_id,
            goal_revision,
            authoritative_tokens_used,
        ],
    )?;
    if goal_changed != 1 {
        return Err(WorkflowError::Conflict(
            "Worker Goal changed during activation".to_string(),
        ));
    }
    tx.execute(
        "INSERT INTO workflow_execution_attempts (
             id, goal_id, plan_revision_id, step_id, status, permission_mode,
             goal_revision_at_start, max_turns, max_tool_calls,
             max_wall_time_secs, max_research_actions, started_at, updated_at
         ) VALUES (
             ?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11
         )",
        params![
            attempt_id,
            request.goal_id,
            plan_revision_id,
            step_id,
            permission_mode,
            next_goal_revision,
            DEFAULT_GOAL_ATTEMPT_MAX_TURNS,
            DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS,
            DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS,
            DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS,
            timestamp,
        ],
    )?;
    let step_changed = tx.execute(
        "UPDATE workflow_plan_steps
         SET status = 'in_progress', claimed_attempt_id = ?1,
             revision = ?2, started_at = COALESCE(started_at, ?3)
         WHERE id = ?4 AND plan_revision_id = ?5
           AND status IN ('pending', 'blocked') AND revision = ?6",
        params![
            attempt_id,
            next_step_revision,
            timestamp,
            step_id,
            plan_revision_id,
            previous_step_revision,
        ],
    )?;
    if step_changed != 1 {
        return Err(WorkflowError::Conflict(
            "dependency-ready Workflow step changed during activation".to_string(),
        ));
    }

    let typed_workspace_mode = match workspace_mode.as_str() {
        "selected" => WorkspaceMode::Selected,
        "created" => WorkspaceMode::Created,
        _ => {
            return Err(WorkflowError::WorkspaceRequired(
                "the Worker DM session is workspace-neutral".to_string(),
            ));
        }
    };
    let typed_execution_context = HiveRunExecutionContextV1::worker_goal(
        request.worker_id.clone(),
        worker_revision,
        typed_workspace_mode,
        workspace_dir.clone(),
        workspace_dir.clone(),
        request.goal_id.clone(),
        next_goal_revision,
        next_goal_revision,
        attempt_id.clone(),
        plan_revision_id.clone(),
        plan_revision_number,
        step_id.clone(),
        next_step_revision,
        WORKER_GOAL_TOOLS
            .iter()
            .map(|tool| (*tool).to_string())
            .collect(),
    )
    .map_err(|error| WorkflowError::Validation(error.to_string()))?;
    typed_execution_context
        .validate()
        .map_err(|error| WorkflowError::Validation(error.to_string()))?;
    let execution_context = serde_json::to_value(&typed_execution_context)?;
    let config = serde_json::json!({
        "worker_id": request.worker_id,
        "model": model,
        "model_key": model_key,
        "model_catalog_revision": model_catalog_revision,
        "permission_mode": permission_mode,
        "workflow_goal_id": request.goal_id,
        "workflow_attempt_id": attempt_id,
        "operation_id": request.operation_id,
        "working_dir": workspace_dir,
        "project_dir": workspace_dir,
        "max_turns": DEFAULT_GOAL_ATTEMPT_MAX_TURNS,
        "max_tool_calls": DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS,
        "max_wall_time_secs": DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS,
        "max_research_actions": DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS,
    });
    let objective: String = tx.query_row(
        "SELECT objective FROM workflow_goals WHERE id = ?1",
        [request.goal_id.as_str()],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO hive_runs (
             id, controller_id, session_id, kind, objective, config_json,
             status, priority, concurrency_key, available_at, attempt_count,
             max_attempts, created_at, updated_at, worker_id,
             governor_origin, governor_lane_key, governor_policy_revision,
             execution_context_json, workflow_goal_id, workflow_attempt_id
         ) VALUES (
             ?1, ?2, ?3, 'worker_workflow', ?4, ?5, 'queued', ?6, ?7,
             ?8, 0, 1, ?8, ?8, ?9, ?10, 'dm', ?11, ?12, ?13, ?14
         )",
        params![
            run_id,
            controller_id,
            session_id,
            objective,
            serde_json::to_string(&config)?,
            if request.source == WorkerWorkflowActivationSource::UserActivation {
                50
            } else {
                0
            },
            format!("worker:{}", request.worker_id),
            timestamp,
            request.worker_id,
            origin.as_str(),
            policy_revision,
            serde_json::to_string(&execution_context)?,
            request.goal_id,
            attempt_id,
        ],
    )?;
    tx.execute(
        "INSERT INTO workflow_events (
             session_id, goal_id, aggregate_revision, operation_id, event_type,
             actor, attempt_id, payload_json, created_at
         ) VALUES (
             ?1, ?2, ?3, ?4, 'worker_workflow_attempt_started',
             ?5, ?6, ?7, ?8
         )",
        params![
            session_id,
            request.goal_id,
            next_goal_revision,
            request.operation_id,
            if request.source == WorkerWorkflowActivationSource::UserActivation {
                "user"
            } else {
                "hive_runtime"
            },
            attempt_id,
            serde_json::json!({
                "run_id": run_id,
                "worker_id": request.worker_id,
                "source": request.source,
                "step_id": step_id,
            })
            .to_string(),
            timestamp,
        ],
    )?;

    let activation = WorkerWorkflowActivation {
        disposition: WorkerWorkflowActivationDisposition::Created,
        worker_id: request.worker_id.clone(),
        owner_user_id: request.owner_user_id.clone(),
        session_id,
        controller_id,
        run_id,
        run_status: "queued".to_string(),
        workflow_goal_id: request.goal_id.clone(),
        workflow_attempt_id: attempt_id,
        workflow_attempt_status: "running".to_string(),
        goal_status: "active".to_string(),
        goal_revision: next_goal_revision,
        workflow_aggregate_revision: next_goal_revision,
        plan_revision_id,
        plan_revision_number,
        step_id,
        step_revision: next_step_revision,
        workspace_dir,
        worker_revision,
        governor_origin: origin,
        execution_context,
    };
    store_activation_receipt(tx, request, &activation)?;
    Ok(activation)
}

pub fn finalize_worker_workflow_attempt_in_transaction(
    tx: &Transaction<'_>,
    daemon_fence: &DaemonFence,
    worker_id: &str,
    owner_user_id: Option<&str>,
    run_id: &str,
    operation_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<WorkerWorkflowActivation>, WorkflowError> {
    let timestamp = canonical_timestamp(now);
    ensure_current_daemon_fence(tx, daemon_fence, &timestamp)?;
    let next: Option<(String, u64, String, String, u64)> = tx
        .query_row(
            "SELECT run.workflow_goal_id, goal.revision, goal.status,
                    plan.status, worker.revision
             FROM hive_runs run
             JOIN workflow_goals goal ON goal.id = run.workflow_goal_id
             JOIN workflow_execution_attempts attempt
               ON attempt.id = run.workflow_attempt_id
             JOIN workflow_plan_revisions plan
               ON plan.id = attempt.plan_revision_id
             JOIN hive_workers worker ON worker.id = run.worker_id
             WHERE run.id = ?1 AND run.kind = 'worker_workflow'
               AND run.status = 'succeeded'
               AND run.worker_id = ?2
               AND worker.user_id IS ?3
               AND run.workflow_goal_id IS NOT NULL
               AND run.workflow_attempt_id IS NOT NULL
               AND EXISTS (
                   SELECT 1 FROM hive_worker_goal_outcomes outcome
                   WHERE outcome.run_id = run.id
                     AND outcome.workflow_goal_id = run.workflow_goal_id
                     AND outcome.workflow_attempt_id = run.workflow_attempt_id
               )",
            params![run_id, worker_id, owner_user_id],
            |row| {
                Ok((
                    row.get(0)?,
                    nonnegative_u64(row, 1)?,
                    row.get(2)?,
                    row.get(3)?,
                    nonnegative_u64(row, 4)?,
                ))
            },
        )
        .optional()?;
    let Some((goal_id, revision, goal_status, plan_status, worker_revision)) = next else {
        return Ok(None);
    };
    if !worker_goal_outcome_is_accounted_in_transaction(tx, run_id)? {
        return Err(WorkflowError::Conflict(
            "source Worker Workflow outcome is not fully provider-accounted".to_string(),
        ));
    }
    if pending_worker_goal_acceptance_exists_in_transaction(tx, &goal_id)
        .map_err(map_acceptance_lifecycle_error)?
    {
        return Ok(None);
    }
    if goal_status != "active" || plan_status != "active" {
        return Ok(None);
    }
    let has_ready_step: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1
             FROM workflow_plan_steps step
             WHERE step.plan_revision_id = (
                 SELECT id FROM workflow_plan_revisions
                 WHERE goal_id = ?1 AND status = 'active'
             )
               AND step.status IN ('pending', 'blocked')
               AND NOT EXISTS (
                   SELECT 1
                   FROM workflow_step_dependencies dependency
                   JOIN workflow_plan_steps prerequisite
                     ON prerequisite.id = dependency.depends_on_step_id
                   WHERE dependency.step_id = step.id
                     AND prerequisite.plan_revision_id = step.plan_revision_id
                     AND prerequisite.status NOT IN ('completed', 'skipped')
               )
         )",
        [goal_id.as_str()],
        |row| row.get(0),
    )?;
    if !has_ready_step {
        return Ok(None);
    }
    activate_or_resume_worker_workflow_in_transaction(
        tx,
        &WorkerWorkflowActivationRequest {
            worker_id: worker_id.to_string(),
            expected_worker_revision: worker_revision,
            owner_user_id: owner_user_id.map(str::to_string),
            goal_id,
            expected_goal_revision: revision,
            operation_id: operation_id.to_string(),
            source: WorkerWorkflowActivationSource::WorkflowRollover,
            now,
        },
    )
    .map(Some)
}

pub fn materialize_due_worker_workflow_rollovers_in_transaction(
    tx: &Transaction<'_>,
    daemon_fence: &DaemonFence,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<WorkerWorkflowActivation>, WorkflowError> {
    if limit == 0 || limit > 100 {
        return Err(WorkflowError::Validation(
            "Worker Workflow rollover sweep limit must be between 1 and 100".to_string(),
        ));
    }
    let timestamp = canonical_timestamp(now);
    ensure_current_daemon_fence(tx, daemon_fence, &timestamp)?;
    let sources = {
        let mut statement = tx.prepare(
            "SELECT source.id, source.worker_id, worker.user_id
             FROM hive_runs source
             JOIN hive_workers worker ON worker.id = source.worker_id
             JOIN sessions session ON session.id = source.session_id
             JOIN hive_controllers controller
               ON controller.id = source.controller_id
             JOIN workflow_goals goal ON goal.id = source.workflow_goal_id
             JOIN workflow_plan_revisions plan
               ON plan.goal_id = goal.id AND plan.status = 'active'
             WHERE source.kind = 'worker_workflow'
               AND source.status = 'succeeded'
               AND worker.status = 'active'
               AND worker.dm_session_id = session.id
               AND worker.user_id IS session.user_id
               AND session.session_type = 'hive'
               AND controller.worker_id = worker.id
               AND controller.session_id = session.id
               AND controller.user_id IS worker.user_id
               AND controller.status = 'active'
               AND goal.status = 'active'
               AND EXISTS (
                   SELECT 1 FROM hive_worker_goal_outcomes outcome
                   WHERE outcome.run_id = source.id
                     AND outcome.worker_id = source.worker_id
                     AND outcome.session_id = source.session_id
                     AND outcome.workflow_goal_id = source.workflow_goal_id
                     AND outcome.workflow_attempt_id = source.workflow_attempt_id
                     AND NOT EXISTS (
                         SELECT 1
                         FROM json_each(outcome.provider_call_ids_json) listed
                         LEFT JOIN hive_worker_provider_calls call
                           ON call.provider_call_id = listed.value
                         LEFT JOIN hive_worker_provider_call_outcomes terminal
                           ON terminal.provider_call_id = call.provider_call_id
                         WHERE call.provider_call_id IS NULL
                            OR call.run_id <> source.id
                            OR call.worker_id <> source.worker_id
                            OR call.session_id <> source.session_id
                            OR call.workflow_goal_id IS NOT source.workflow_goal_id
                            OR call.workflow_attempt_id IS NOT source.workflow_attempt_id
                            OR call.call_kind <> 'agent_turn'
                            OR terminal.state IS NOT 'completed'
                            OR terminal.outcome IS NOT 'completed'
                            OR terminal.remote_acceptance IS NOT 'acknowledged'
                     )
                     AND NOT EXISTS (
                         SELECT 1
                         FROM hive_worker_provider_calls call
                         LEFT JOIN hive_worker_provider_call_outcomes terminal
                           ON terminal.provider_call_id = call.provider_call_id
                         WHERE call.run_id = source.id
                           AND (
                               call.worker_id <> source.worker_id
                               OR call.session_id <> source.session_id
                               OR call.workflow_goal_id IS NOT source.workflow_goal_id
                               OR call.workflow_attempt_id
                                   IS NOT source.workflow_attempt_id
                               OR terminal.state IS NOT 'completed'
                               OR terminal.remote_acceptance IS NOT 'acknowledged'
                               OR (
                                   call.call_kind = 'agent_turn'
                                   AND NOT EXISTS (
                                       SELECT 1
                                       FROM json_each(
                                           outcome.provider_call_ids_json
                                       ) named
                                       WHERE named.value = call.provider_call_id
                                   )
                               )
                           )
                   )
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM hive_worker_goal_acceptance_candidates candidate
                   WHERE candidate.source_run_id = source.id
                     AND candidate.state IN (
                         'awaiting_user', 'needs_user', 'verifying'
                     )
               )
               AND NOT EXISTS (
                   SELECT 1 FROM workflow_idempotency receipt
                   WHERE receipt.operation_id =
                       'worker-workflow-rollover:' || source.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM hive_runs current
                   WHERE current.workflow_goal_id = source.workflow_goal_id
                     AND current.kind = 'worker_workflow'
                     AND current.status IN (
                         'queued', 'leased', 'running', 'sleeping',
                         'awaiting_input', 'retry_wait', 'recovery_required'
                     )
               )
               AND source.id = (
                   SELECT latest.id FROM hive_runs latest
                   WHERE latest.workflow_goal_id = source.workflow_goal_id
                     AND latest.kind = 'worker_workflow'
                     AND latest.status = 'succeeded'
                   ORDER BY latest.finished_at DESC, latest.updated_at DESC,
                            latest.id DESC
                   LIMIT 1
               )
               AND EXISTS (
                   SELECT 1 FROM workflow_plan_steps step
                   WHERE step.plan_revision_id = plan.id
                     AND step.status IN ('pending', 'blocked')
                     AND NOT EXISTS (
                         SELECT 1
                         FROM workflow_step_dependencies dependency
                         JOIN workflow_plan_steps prerequisite
                           ON prerequisite.id = dependency.depends_on_step_id
                         WHERE dependency.step_id = step.id
                           AND prerequisite.plan_revision_id = step.plan_revision_id
                           AND prerequisite.status NOT IN ('completed', 'skipped')
                     )
               )
             ORDER BY source.finished_at, source.id
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map(
                [i64::try_from(limit).map_err(|_| {
                    WorkflowError::Validation("rollover sweep limit overflow".to_string())
                })?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut activations = Vec::with_capacity(sources.len());
    for (run_id, worker_id, owner_user_id) in sources {
        let Some(reconciliation) =
            reconcile_worker_workflow_run_in_transaction(tx, daemon_fence, &run_id, now)?
        else {
            continue;
        };
        if reconciliation.run_status != "succeeded"
            || reconciliation.goal_status != "active"
            || reconciliation.recovery_required
        {
            continue;
        }
        if let Some(activation) = finalize_worker_workflow_attempt_in_transaction(
            tx,
            daemon_fence,
            &worker_id,
            owner_user_id.as_deref(),
            &run_id,
            &format!("worker-workflow-rollover:{run_id}"),
            now,
        )? {
            activations.push(activation);
        }
    }
    Ok(activations)
}

fn ensure_current_daemon_fence(
    tx: &Transaction<'_>,
    daemon_fence: &DaemonFence,
    now: &str,
) -> Result<(), WorkflowError> {
    let current: bool = tx.query_row(
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
        |row| row.get(0),
    )?;
    if !current {
        return Err(WorkflowError::Conflict(
            "Hive daemon generation is stale".to_string(),
        ));
    }
    Ok(())
}

fn load_existing_nonterminal(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowActivationRequest,
) -> Result<Option<WorkerWorkflowActivation>, WorkflowError> {
    let existing = tx
        .query_row(
            "SELECT run.worker_id, worker.user_id, run.session_id,
                run.controller_id, run.id, run.status, run.workflow_goal_id,
                run.workflow_attempt_id, attempt.status, goal.status,
                goal.revision, plan.id,
                plan.revision_number, step.id, step.revision,
                session.working_dir, worker.revision, run.governor_origin,
                run.execution_context_json
         FROM hive_runs run
         JOIN hive_workers worker ON worker.id = run.worker_id
         JOIN sessions session ON session.id = run.session_id
         JOIN workflow_goals goal ON goal.id = run.workflow_goal_id
         JOIN workflow_execution_attempts attempt
           ON attempt.id = run.workflow_attempt_id
         JOIN workflow_plan_revisions plan ON plan.id = attempt.plan_revision_id
         JOIN workflow_plan_steps step ON step.id = attempt.step_id
         WHERE run.kind = 'worker_workflow'
           AND run.workflow_goal_id = ?1
           AND run.status IN (
               'queued', 'leased', 'running', 'sleeping', 'awaiting_input',
               'retry_wait', 'recovery_required'
           )",
            [request.goal_id.as_str()],
            |row| {
                let worker_id: String = row.get(0)?;
                let owner_user_id: Option<String> = row.get(1)?;
                let origin: String = row.get(17)?;
                let execution_context_json: String = row.get(18)?;
                let goal_revision = nonnegative_u64(row, 10)?;
                let worker_revision = nonnegative_u64(row, 16)?;
                Ok(WorkerWorkflowActivation {
                    disposition: WorkerWorkflowActivationDisposition::Existing,
                    worker_id,
                    owner_user_id,
                    session_id: row.get(2)?,
                    controller_id: row.get(3)?,
                    run_id: row.get(4)?,
                    run_status: row.get(5)?,
                    workflow_goal_id: row.get(6)?,
                    workflow_attempt_id: row.get(7)?,
                    workflow_attempt_status: row.get(8)?,
                    goal_status: row.get(9)?,
                    goal_revision,
                    workflow_aggregate_revision: goal_revision,
                    plan_revision_id: row.get(11)?,
                    plan_revision_number: nonnegative_u64(row, 12)?,
                    step_id: row.get(13)?,
                    step_revision: nonnegative_u64(row, 14)?,
                    workspace_dir: row.get(15)?,
                    worker_revision,
                    governor_origin: WorkerRunOrigin::parse(&origin)
                        .ok_or(rusqlite::Error::InvalidQuery)?,
                    execution_context: serde_json::from_str(&execution_context_json)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            },
        )
        .optional()?;
    if let Some(existing) = existing.as_ref() {
        if existing.worker_id != request.worker_id
            || existing.owner_user_id != request.owner_user_id
            || existing.worker_revision != request.expected_worker_revision
            || existing.goal_revision != request.expected_goal_revision
        {
            return Err(WorkflowError::Conflict(
                "existing Worker Workflow run differs from the activation fence".to_string(),
            ));
        }
    }
    Ok(existing)
}

/// Worker archive irreversibly invalidates pending acceptance and cancels its
/// exact Goal/plan/step/attempt in the same archive transaction.
pub fn archive_worker_goal_acceptances_in_transaction(
    tx: &Transaction<'_>,
    worker_id: &str,
    now: &str,
) -> Result<Vec<String>, WorkflowError> {
    transition_pending_worker_goal_acceptances_for_worker_lifecycle(
        tx,
        worker_id,
        WorkerGoalAcceptanceLifecycle::WorkerArchived,
        now,
    )
}

fn transition_pending_worker_goal_acceptances_for_worker_lifecycle(
    tx: &Transaction<'_>,
    worker_id: &str,
    lifecycle: WorkerGoalAcceptanceLifecycle,
    now: &str,
) -> Result<Vec<String>, WorkflowError> {
    let candidates = terminalize_pending_worker_goal_acceptances_in_transaction(
        tx,
        None,
        Some(worker_id),
        lifecycle,
        now,
    )
    .map_err(map_acceptance_lifecycle_error)?;
    let mut acceptance_run_ids = Vec::with_capacity(candidates.len());
    let mut transitioned_goals = std::collections::HashSet::with_capacity(candidates.len());
    for candidate in candidates {
        if !transitioned_goals.insert(candidate.workflow_goal_id.clone()) {
            return Err(WorkflowError::Conflict(
                "one Worker Goal has multiple pending acceptance candidates".to_string(),
            ));
        }
        let attempt_changed = tx.execute(
            "UPDATE workflow_execution_attempts
             SET status = 'cancelled', stop_reason = ?2,
                 ended_at = COALESCE(ended_at, ?3), updated_at = ?3
             WHERE id = ?1 AND goal_id = ?4 AND status = 'paused'
               AND stop_reason = 'awaiting_acceptance'",
            params![
                candidate.source_attempt_id,
                lifecycle_reason(lifecycle),
                now,
                candidate.workflow_goal_id,
            ],
        )?;
        if attempt_changed != 1 {
            return Err(WorkflowError::Conflict(
                "pending acceptance attempt changed during Worker lifecycle transition".to_string(),
            ));
        }
        let step_status = if lifecycle == WorkerGoalAcceptanceLifecycle::WorkerArchived {
            "cancelled"
        } else {
            "pending"
        };
        let step_changed = tx.execute(
            "UPDATE workflow_plan_steps
             SET status = ?2, claimed_attempt_id = NULL,
                 revision = revision + 1,
                 outcome = ?3,
                 evidence_json = CASE WHEN ?2 = 'cancelled' THEN '[]' ELSE evidence_json END
             WHERE id = ?1 AND plan_revision_id = ?4
               AND status = 'in_progress' AND claimed_attempt_id = ?5
               AND revision = ?6",
            params![
                candidate.step_id,
                step_status,
                lifecycle_reason(lifecycle),
                candidate.plan_revision_id,
                candidate.source_attempt_id,
                candidate.step_revision,
            ],
        )?;
        if step_changed != 1 {
            return Err(WorkflowError::Conflict(
                "pending acceptance step changed during Worker lifecycle transition".to_string(),
            ));
        }
        if lifecycle == WorkerGoalAcceptanceLifecycle::WorkerArchived {
            tx.execute(
                "UPDATE workflow_plan_revisions
                 SET status = 'cancelled'
                 WHERE id = ?1 AND goal_id = ?2
                   AND status IN ('proposed', 'approved', 'active')",
                params![candidate.plan_revision_id, candidate.workflow_goal_id],
            )?;
        }
        let (session_id, current_revision, current_status): (String, u64, String) = tx.query_row(
            "SELECT session_id, revision, status FROM workflow_goals WHERE id = ?1",
            [candidate.workflow_goal_id.as_str()],
            |row| Ok((row.get(0)?, nonnegative_u64(row, 1)?, row.get(2)?)),
        )?;
        if !matches!(current_status.as_str(), "active" | "paused" | "blocked") {
            return Err(WorkflowError::Conflict(
                "pending acceptance Goal is no longer lifecycle-transitionable".to_string(),
            ));
        }
        let next_revision = current_revision
            .checked_add(1)
            .ok_or_else(|| WorkflowError::Validation("workflow revision overflow".to_string()))?;
        let goal_status = if lifecycle == WorkerGoalAcceptanceLifecycle::WorkerArchived {
            "cancelled"
        } else {
            "paused"
        };
        let goal_changed = tx.execute(
            "UPDATE workflow_goals
             SET status = ?2, status_reason = ?3, revision = ?4,
                 cancelled_at = CASE WHEN ?2 = 'cancelled' THEN ?5 ELSE cancelled_at END,
                 updated_at = ?5
             WHERE id = ?1 AND revision = ?6
               AND status IN ('active', 'paused', 'blocked')",
            params![
                candidate.workflow_goal_id,
                goal_status,
                lifecycle_reason(lifecycle),
                next_revision,
                now,
                current_revision,
            ],
        )?;
        if goal_changed != 1 {
            return Err(WorkflowError::Conflict(
                "pending acceptance Goal changed during Worker lifecycle transition".to_string(),
            ));
        }
        tx.execute(
            "INSERT INTO workflow_events (
                 session_id, goal_id, aggregate_revision, operation_id,
                 event_type, actor, attempt_id, payload_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'hive_runtime', ?6, ?7, ?8)",
            params![
                session_id,
                candidate.workflow_goal_id,
                next_revision,
                format!(
                    "worker-goal-acceptance-lifecycle:{}:{}",
                    lifecycle_reason(lifecycle),
                    candidate.acceptance_run_id
                ),
                if lifecycle == WorkerGoalAcceptanceLifecycle::WorkerArchived {
                    "worker_goal_acceptance_archived"
                } else {
                    "worker_goal_acceptance_stale_worker_paused"
                },
                candidate.source_attempt_id,
                serde_json::json!({
                    "acceptance_run_id": candidate.acceptance_run_id,
                    "source_run_id": candidate.source_run_id,
                    "worker_id": candidate.worker_id,
                    "step_id": candidate.step_id,
                    "reason": lifecycle_reason(lifecycle),
                })
                .to_string(),
                now,
            ],
        )?;
        acceptance_run_ids.push(candidate.acceptance_run_id);
    }
    Ok(acceptance_run_ids)
}

const fn lifecycle_reason(lifecycle: WorkerGoalAcceptanceLifecycle) -> &'static str {
    match lifecycle {
        WorkerGoalAcceptanceLifecycle::GoalCancelled => "workflow_goal_cancelled",
        WorkerGoalAcceptanceLifecycle::WorkerArchived => "worker_archived",
    }
}

fn map_acceptance_lifecycle_error(error: WorkerGoalAcceptanceStageError) -> WorkflowError {
    match error {
        WorkerGoalAcceptanceStageError::Stale(message)
        | WorkerGoalAcceptanceStageError::Conflict(message) => WorkflowError::Conflict(message),
    }
}

pub fn pause_worker_workflow_in_transaction(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowLifecycleRequest,
) -> Result<WorkerWorkflowLifecycleResult, WorkflowError> {
    transition_worker_workflow_in_transaction(tx, request, "paused", false)
}

pub fn cancel_worker_workflow_in_transaction(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowLifecycleRequest,
) -> Result<WorkerWorkflowLifecycleResult, WorkflowError> {
    transition_worker_workflow_in_transaction(tx, request, "cancelled", true)
}

fn transition_worker_workflow_in_transaction(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowLifecycleRequest,
    target: &str,
    cancel_plan: bool,
) -> Result<WorkerWorkflowLifecycleResult, WorkflowError> {
    if request.operation_id.trim().is_empty() || request.reason.trim().is_empty() {
        return Err(WorkflowError::Validation(
            "lifecycle operation and reason are required".to_string(),
        ));
    }
    if let Some(receipt) = load_lifecycle_receipt(tx, request, target)? {
        return Ok(receipt);
    }
    let binding: Option<(String, Option<String>, u64, String, u64, String)> = tx
        .query_row(
            "SELECT worker.id, worker.user_id, worker.revision,
                    goal.session_id, goal.revision,
                    goal.status
             FROM workflow_goals goal
             JOIN hive_workers worker ON worker.dm_session_id = goal.session_id
             WHERE goal.id = ?1 AND worker.id = ?2",
            params![request.goal_id, request.worker_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    nonnegative_u64(row, 2)?,
                    row.get(3)?,
                    nonnegative_u64(row, 4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((_worker_id, owner_user_id, worker_revision, session_id, revision, current_status)) =
        binding
    else {
        return Err(WorkflowError::NotFound(format!(
            "Worker Goal {}",
            request.goal_id
        )));
    };
    if owner_user_id != request.owner_user_id
        || worker_revision != request.expected_worker_revision
        || revision != request.expected_goal_revision
    {
        return Err(WorkflowError::Conflict(
            "Worker Goal lifecycle fence changed".to_string(),
        ));
    }
    if current_status == target {
        let result = WorkerWorkflowLifecycleResult {
            changed: false,
            worker_id: request.worker_id.clone(),
            worker_revision,
            owner_user_id,
            session_id,
            workflow_goal_id: request.goal_id.clone(),
            goal_revision: revision,
            goal_status: current_status,
            affected_run_ids: Vec::new(),
            affected_attempt_ids: Vec::new(),
        };
        store_lifecycle_receipt(tx, request, target, &result)?;
        return Ok(result);
    }
    if !matches!(
        current_status.as_str(),
        "draft" | "active" | "paused" | "blocked"
    ) {
        return Err(WorkflowError::InvalidTransition(format!(
            "cannot {target} {current_status} Worker Goal"
        )));
    }
    let timestamp = canonical_timestamp(request.now);
    let pending_acceptance =
        pending_worker_goal_acceptance_exists_in_transaction(tx, &request.goal_id)
            .map_err(map_acceptance_lifecycle_error)?;
    if target == "paused" && pending_acceptance {
        return Err(WorkflowError::InvalidTransition(
            "Worker Goal is already stopped awaiting an explicit acceptance decision; pause would invalidate its frozen revision"
                .to_string(),
        ));
    }
    let terminalized_acceptances = if target == "cancelled" && pending_acceptance {
        terminalize_pending_worker_goal_acceptances_in_transaction(
            tx,
            Some(&request.goal_id),
            Some(&request.worker_id),
            WorkerGoalAcceptanceLifecycle::GoalCancelled,
            &timestamp,
        )
        .map_err(map_acceptance_lifecycle_error)?
    } else {
        Vec::new()
    };
    let mut affected = {
        let mut statement = tx.prepare(
            "SELECT id, workflow_attempt_id FROM hive_runs
             WHERE workflow_goal_id = ?1 AND kind = 'worker_workflow'
               AND status IN (
                   'queued', 'leased', 'running', 'sleeping', 'awaiting_input',
                   'retry_wait', 'recovery_required'
               ) ORDER BY created_at, id",
        )?;
        let rows = statement
            .query_map([request.goal_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    affected.extend(terminalized_acceptances.iter().map(|candidate| {
        (
            candidate.acceptance_run_id.clone(),
            candidate.source_attempt_id.clone(),
        )
    }));

    // A lifecycle fence makes every unaccounted Started call explicitly
    // Unknown.  Leaving it absent would block the Worker governor forever and
    // pretending it was unsent could duplicate a remote side effect.
    tx.execute(
        "INSERT OR IGNORE INTO hive_worker_provider_call_outcomes (
             provider_call_id, state, outcome, remote_acceptance,
             unknown_reason, finished_at
         )
         SELECT call.provider_call_id, 'unknown', 'lifecycle_interrupted',
                'possibly_sent', ?2, ?3
         FROM hive_worker_provider_calls call
         JOIN hive_runs run ON run.id = call.run_id
         WHERE run.workflow_goal_id = ?1
           AND run.kind = 'worker_workflow'
           AND NOT EXISTS (
               SELECT 1 FROM hive_worker_provider_call_outcomes terminal
               WHERE terminal.provider_call_id = call.provider_call_id
           )",
        params![request.goal_id, request.reason.trim(), timestamp],
    )?;
    tx.execute(
        "UPDATE hive_run_attempts
         SET finished_at = COALESCE(finished_at, ?2), outcome = 'cancelled',
             stop_reason = ?3, error = NULL
         WHERE run_id IN (
             SELECT id FROM hive_runs
             WHERE workflow_goal_id = ?1 AND kind = 'worker_workflow'
               AND status IN (
                   'queued', 'leased', 'running', 'sleeping', 'awaiting_input',
                   'retry_wait', 'recovery_required'
               )
         ) AND finished_at IS NULL",
        params![request.goal_id, timestamp, request.reason.trim()],
    )?;
    tx.execute(
        "UPDATE hive_runs
         SET status = 'cancelled', lease_owner = NULL, lease_token = NULL,
             lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
             last_stop_reason = ?2, last_error = NULL,
             finished_at = COALESCE(finished_at, ?3), updated_at = ?3
         WHERE workflow_goal_id = ?1 AND kind = 'worker_workflow'
           AND status IN (
               'queued', 'leased', 'running', 'sleeping', 'awaiting_input',
               'retry_wait', 'recovery_required'
           )",
        params![request.goal_id, request.reason.trim(), timestamp],
    )?;
    let attempt_status = if target == "cancelled" {
        "cancelled"
    } else {
        "paused"
    };
    tx.execute(
        "UPDATE workflow_execution_attempts
         SET status = ?2, stop_reason = ?3, ended_at = COALESCE(ended_at, ?4),
             updated_at = ?4
         WHERE goal_id = ?1
           AND (
               status = 'running'
               OR (
                   ?2 = 'cancelled' AND status = 'paused'
                   AND stop_reason = 'awaiting_acceptance'
               )
           )",
        params![
            request.goal_id,
            attempt_status,
            request.reason.trim(),
            timestamp,
        ],
    )?;
    let step_status = if target == "cancelled" {
        "cancelled"
    } else {
        "pending"
    };
    tx.execute(
        "UPDATE workflow_plan_steps
         SET status = ?2, claimed_attempt_id = NULL, revision = revision + 1
         WHERE claimed_attempt_id IN (
             SELECT id FROM workflow_execution_attempts WHERE goal_id = ?1
         ) AND status = 'in_progress'",
        params![request.goal_id, step_status],
    )?;
    if cancel_plan {
        tx.execute(
            "UPDATE workflow_plan_revisions SET status = 'cancelled'
             WHERE goal_id = ?1 AND status IN ('proposed', 'approved', 'active')",
            [request.goal_id.as_str()],
        )?;
    }
    let next_revision = revision
        .checked_add(1)
        .ok_or_else(|| WorkflowError::Validation("workflow revision overflow".to_string()))?;
    tx.execute(
        "UPDATE workflow_goals
         SET status = ?2, status_reason = ?3, revision = ?4,
             cancelled_at = CASE WHEN ?2 = 'cancelled' THEN ?5 ELSE cancelled_at END,
             updated_at = ?5
         WHERE id = ?1 AND revision = ?6",
        params![
            request.goal_id,
            target,
            request.reason.trim(),
            next_revision,
            timestamp,
            revision,
        ],
    )?;
    tx.execute(
        "INSERT INTO workflow_events (
             session_id, goal_id, aggregate_revision, operation_id, event_type,
             actor, payload_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'user', ?6, ?7)",
        params![
            session_id,
            request.goal_id,
            next_revision,
            request.operation_id,
            if target == "cancelled" {
                "worker_workflow_cancelled"
            } else {
                "worker_workflow_paused"
            },
            serde_json::json!({
                "worker_id": request.worker_id,
                "reason": request.reason.trim(),
            })
            .to_string(),
            timestamp,
        ],
    )?;
    let result = WorkerWorkflowLifecycleResult {
        changed: true,
        worker_id: request.worker_id.clone(),
        worker_revision,
        owner_user_id,
        session_id,
        workflow_goal_id: request.goal_id.clone(),
        goal_revision: next_revision,
        goal_status: target.to_string(),
        affected_run_ids: affected.iter().map(|(run_id, _)| run_id.clone()).collect(),
        affected_attempt_ids: affected
            .into_iter()
            .map(|(_, attempt_id)| attempt_id)
            .collect(),
    };
    store_lifecycle_receipt(tx, request, target, &result)?;
    Ok(result)
}

fn activation_receipt_request(request: &WorkerWorkflowActivationRequest) -> Value {
    serde_json::json!({
        "worker_id": request.worker_id,
        "expected_worker_revision": request.expected_worker_revision,
        "owner_user_id": request.owner_user_id,
        "goal_id": request.goal_id,
        "expected_goal_revision": request.expected_goal_revision,
        "source": request.source,
    })
}

fn lifecycle_receipt_request(request: &WorkerWorkflowLifecycleRequest, target: &str) -> Value {
    serde_json::json!({
        "worker_id": request.worker_id,
        "expected_worker_revision": request.expected_worker_revision,
        "owner_user_id": request.owner_user_id,
        "goal_id": request.goal_id,
        "expected_goal_revision": request.expected_goal_revision,
        "target": target,
        "reason": request.reason,
    })
}

fn load_activation_receipt(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowActivationRequest,
) -> Result<Option<WorkerWorkflowActivation>, WorkflowError> {
    load_typed_receipt(
        tx,
        &request.operation_id,
        "worker_workflow_activate",
        activation_receipt_request(request),
    )
}

fn store_activation_receipt(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowActivationRequest,
    result: &WorkerWorkflowActivation,
) -> Result<(), WorkflowError> {
    store_typed_receipt(
        tx,
        &request.operation_id,
        &result.session_id,
        "worker_workflow_activate",
        activation_receipt_request(request),
        result,
        request.now,
    )
}

fn load_lifecycle_receipt(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowLifecycleRequest,
    target: &str,
) -> Result<Option<WorkerWorkflowLifecycleResult>, WorkflowError> {
    load_typed_receipt(
        tx,
        &request.operation_id,
        "worker_workflow_lifecycle",
        lifecycle_receipt_request(request, target),
    )
}

fn store_lifecycle_receipt(
    tx: &Transaction<'_>,
    request: &WorkerWorkflowLifecycleRequest,
    target: &str,
    result: &WorkerWorkflowLifecycleResult,
) -> Result<(), WorkflowError> {
    store_typed_receipt(
        tx,
        &request.operation_id,
        &result.session_id,
        "worker_workflow_lifecycle",
        lifecycle_receipt_request(request, target),
        result,
        request.now,
    )
}

fn load_typed_receipt<T: for<'de> Deserialize<'de>>(
    tx: &Transaction<'_>,
    operation_id: &str,
    expected_action: &str,
    expected_request: Value,
) -> Result<Option<T>, WorkflowError> {
    let receipt: Option<(String, String)> = tx
        .query_row(
            "SELECT action, result_json FROM workflow_idempotency
             WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((action, result_json)) = receipt else {
        return Ok(None);
    };
    let envelope: Value = serde_json::from_str(&result_json)?;
    if action != expected_action || envelope.get("request") != Some(&expected_request) {
        return Err(WorkflowError::Conflict(
            "workflow operation id was already used for a different mutation".to_string(),
        ));
    }
    let result = envelope
        .get("result")
        .cloned()
        .ok_or_else(|| WorkflowError::Database("workflow receipt has no result".to_string()))?;
    Ok(Some(serde_json::from_value(result)?))
}

fn store_typed_receipt<T: Serialize>(
    tx: &Transaction<'_>,
    operation_id: &str,
    session_id: &str,
    action: &str,
    request: Value,
    result: &T,
    now: DateTime<Utc>,
) -> Result<(), WorkflowError> {
    let result_json = serde_json::to_string(&serde_json::json!({
        "schema_version": 1,
        "request": request,
        "result": result,
    }))?;
    tx.execute(
        "INSERT INTO workflow_idempotency (
             operation_id, session_id, action, result_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            operation_id,
            session_id,
            action,
            result_json,
            canonical_timestamp(now),
        ],
    )?;
    Ok(())
}

fn validate_activation_request(
    request: &WorkerWorkflowActivationRequest,
) -> Result<(), WorkflowError> {
    for (label, value) in [
        ("worker_id", request.worker_id.as_str()),
        ("goal_id", request.goal_id.as_str()),
        ("operation_id", request.operation_id.as_str()),
    ] {
        if value.trim().is_empty() || value.trim() != value || value.len() > 512 {
            return Err(WorkflowError::Validation(format!(
                "invalid Worker Workflow {label}"
            )));
        }
    }
    if request.expected_worker_revision == 0 || request.expected_goal_revision == 0 {
        return Err(WorkflowError::Validation(
            "Worker and Workflow revisions must be positive".to_string(),
        ));
    }
    Ok(())
}

fn valid_absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 16 * 1024
        && !value.chars().any(|character| character == '\0')
        && Path::new(value).is_absolute()
}

fn nonnegative_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| conversion_error(index, "negative integer"))
}

fn optional_nonnegative_u64(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| u64::try_from(value).map_err(|_| conversion_error(index, "negative integer")))
        .transpose()
}

fn conversion_error(index: usize, message: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Integer,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.to_string(),
        )),
    )
}
