//! Worker-aware Goal projection and lifecycle control.
//!
//! Goal/plan authoring remains in `WorkflowManager`; execution, cancellation,
//! and workspace ownership cross the typed Hive daemon control plane.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};

use mitsuro_core::agent::{
    WorkerGoalAttemptOutcome, WorkerGoalEvidenceKind, WorkerGoalOutcomeCounters,
};
use mitsuro_core::storage::{
    hash_request_bytes, Database, HiveWorker, HiveWorkerIntroductionStatus,
    HiveWorkerIntroductionStore, HiveWorkerStatus, HiveWorkerStore, SessionType,
    SqliteWorkerGoalAcceptanceStore, WorkerGoalAcceptanceCandidateRecord,
    WorkerGoalAcceptanceCandidateState, WorkerGoalAcceptanceSourceSummary, WorkspaceMode,
};
use mitsuro_core::workflow::{
    CreateGoalInput, GoalStatus, PlanProposalInput, PlanRevisionStatus, WorkflowError,
    WorkflowManager, WorkflowSnapshot,
};
use mitsuro_hive_protocol::{
    ActivateOrResumeWorkerWorkflowCommand, ResolveWorkerGoalAcceptanceCommand,
    SetWorkerWorkspaceCommand, WorkerWorkflowLifecycleCommand, WorkerWorkspaceMode,
};

use super::{idempotency_key_from_headers, open_session_manager};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::routes::session_access::{current_user_id, load_owned_session, request_workspace_scope};
use crate::utils::workspace::{
    normalize_resolved_requested_workspace, WorkspaceNormalizationPolicy,
};
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WorkerGoalWorkspaceProjection {
    mode: WorkspaceMode,
    working_dir: Option<String>,
    project_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WorkerGoalRunProjection {
    run_id: String,
    run_status: String,
    attempt_id: String,
    attempt_status: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum WorkerGoalAction {
    CreateGoal,
    ApprovePlan,
    Activate,
    Pause,
    Cancel,
    SetWorkspace,
    ResolveAcceptance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WorkerGoalPendingCriterionProjection {
    criterion_id: String,
    description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WorkerGoalSourceEvidenceProjection {
    kind: WorkerGoalEvidenceKind,
    summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WorkerGoalSourceEffectProjection {
    summary: String,
    workspace_mutated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WorkerGoalAcceptanceSourceProjection {
    outcome: WorkerGoalAttemptOutcome,
    evidence: Vec<WorkerGoalSourceEvidenceProjection>,
    effect: WorkerGoalSourceEffectProjection,
    counters: WorkerGoalOutcomeCounters,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct WorkerGoalPendingAcceptanceProjection {
    acceptance_run_id: String,
    source_run_id: String,
    goal_id: String,
    attempt_id: String,
    step_id: String,
    expected_worker_revision: u64,
    expected_goal_revision: u64,
    step_revision: u64,
    step_description: String,
    is_final_step: bool,
    required_goal_criteria: Vec<WorkerGoalPendingCriterionProjection>,
    source_summary: WorkerGoalAcceptanceSourceProjection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WorkerGoalProjection {
    schema_version: u32,
    worker_id: String,
    worker_revision: u64,
    worker_status: HiveWorkerStatus,
    session_id: String,
    workspace: WorkerGoalWorkspaceProjection,
    introduction_status: Option<String>,
    introduction_ready: bool,
    workflow: Option<WorkflowSnapshot>,
    active_run: Option<WorkerGoalRunProjection>,
    pending_acceptance: Option<WorkerGoalPendingAcceptanceProjection>,
    attention: Vec<String>,
    read_only_reason: Option<String>,
    allowed_actions: Vec<WorkerGoalAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ApproveWorkerGoalRequest {
    goal_id: String,
    plan_revision_id: String,
    expected_worker_revision: u64,
    expected_goal_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CreateWorkerGoalRequest {
    expected_worker_revision: u64,
    goal: CreateGoalInput,
    plan: PlanProposalInput,
}

#[derive(Debug, Deserialize)]
pub(super) struct MutateWorkerGoalRequest {
    goal_id: String,
    expected_worker_revision: u64,
    expected_goal_revision: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct TransitionWorkerGoalRequest {
    goal_id: String,
    expected_worker_revision: u64,
    expected_goal_revision: u64,
    reason: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetWorkerGoalWorkspaceRequest {
    expected_worker_revision: u64,
    workspace_mode: WorkspaceMode,
    working_dir: Option<String>,
    project_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResolveWorkerGoalAcceptanceRequest {
    expected_worker_revision: u64,
    #[serde(flatten)]
    command: ResolveWorkerGoalAcceptanceCommand,
}

pub(super) async fn get_worker_goal(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

pub(super) async fn create_worker_goal(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateWorkerGoalRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    let idempotency_key = required_idempotency_key(&headers, "create a Worker Goal")?;
    let before = load_worker_goal_projection(&state, user.as_ref(), &worker_id)?;
    let operation_id = worker_goal_create_operation_id(&worker_id, &idempotency_key, &request)?;
    if request.expected_worker_revision == 0
        || before.worker_revision != request.expected_worker_revision
    {
        return Err(AppError::Conflict(
            "Worker revision changed; refresh and try again".to_string(),
        ));
    }
    validate_worker_first_plan_provenance(&request.plan)?;

    // A committed create makes CreateGoal disappear from allowed_actions.
    // Permit only the exact unfinished-Goal shape to reach the atomic core
    // receipt: the same operation replays, while any different request must
    // conflict on the existing unfinished Goal without a second mutation.
    let may_be_atomic_replay = before
        .workflow
        .as_ref()
        .is_some_and(|workflow| workflow.goal.status.is_unfinished());
    if !before
        .allowed_actions
        .contains(&WorkerGoalAction::CreateGoal)
        && !may_be_atomic_replay
    {
        ensure_worker_goal_action(&before, WorkerGoalAction::CreateGoal)?;
    }

    WorkflowManager::new(state.db_path.as_ref().clone())
        .map_err(map_workflow_error)?
        .create_goal_with_plan(
            &before.session_id,
            request.goal,
            request.plan,
            &operation_id,
            "user",
        )
        .map_err(map_workflow_error)?;

    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

pub(super) async fn approve_worker_goal(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ApproveWorkerGoalRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    let idempotency_key = required_idempotency_key(&headers, "approve a Worker Goal plan")?;
    let before = load_worker_goal_projection(&state, user.as_ref(), &worker_id)?;
    let operation_id = worker_goal_approve_operation_id(&worker_id, &idempotency_key, &request)?;
    let may_be_atomic_replay = is_worker_goal_approve_replay_shape(&before, &request);
    if !may_be_atomic_replay {
        ensure_worker_goal_fence(
            &before,
            &request.goal_id,
            request.expected_worker_revision,
            request.expected_goal_revision,
        )?;
        ensure_worker_goal_action(&before, WorkerGoalAction::ApprovePlan)?;
        if before.active_run.is_some() {
            return Err(AppError::Conflict(
                "Pause or finish the active Worker Goal run before approving a plan".to_string(),
            ));
        }
        if before
            .workflow
            .as_ref()
            .and_then(|workflow| workflow.plan_revision.as_ref())
            .is_none_or(|plan| {
                plan.id != request.plan_revision_id || plan.status != PlanRevisionStatus::Proposed
            })
        {
            return Err(AppError::Conflict(
                "The proposed Worker Goal plan changed; refresh and try again".to_string(),
            ));
        }
    }

    WorkflowManager::new(state.db_path.as_ref().clone())
        .map_err(map_workflow_error)?
        .approve_plan(
            &before.session_id,
            &request.goal_id,
            &request.plan_revision_id,
            request.expected_goal_revision,
            &operation_id,
            "user",
        )
        .map_err(map_workflow_error)?;

    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

pub(super) async fn activate_worker_goal(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<MutateWorkerGoalRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    let idempotency_key = required_idempotency_key(&headers, "activate a Worker Goal")?;
    let before = load_worker_goal_projection(&state, user.as_ref(), &worker_id)?;
    ensure_worker_goal_fence(
        &before,
        &request.goal_id,
        request.expected_worker_revision,
        request.expected_goal_revision,
    )?;
    ensure_worker_goal_action(&before, WorkerGoalAction::Activate)?;

    let result = state
        .hive_runtime
        .activate_or_resume_worker_workflow_for_user(
            current_user_id(user.as_ref()),
            ActivateOrResumeWorkerWorkflowCommand {
                worker_id: worker_id.clone(),
                expected_worker_revision: request.expected_worker_revision,
                goal_id: request.goal_id.clone(),
                expected_goal_revision: request.expected_goal_revision,
            },
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    validate_worker_goal_daemon_result(&result, &before, &request.goal_id)?;

    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

pub(super) async fn pause_worker_goal(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<TransitionWorkerGoalRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    transition_worker_goal(state, user, worker_id, headers, request, false).await
}

pub(super) async fn cancel_worker_goal(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<TransitionWorkerGoalRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    transition_worker_goal(state, user, worker_id, headers, request, true).await
}

pub(super) async fn resolve_worker_goal_acceptance(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ResolveWorkerGoalAcceptanceRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    let idempotency_key = required_idempotency_key(&headers, "resolve a Worker Goal acceptance")?;
    let before = load_worker_goal_projection(&state, user.as_ref(), &worker_id)?;
    let candidate = load_owned_acceptance_candidate(
        &state,
        user.as_ref(),
        &worker_id,
        &request.command.acceptance_run_id,
    )?;
    if request.expected_worker_revision == 0
        || request.expected_worker_revision != candidate.worker_revision
        || request.command.expected_goal_revision == 0
        || request.command.expected_goal_revision != candidate.goal_revision
        || candidate.session_id != before.session_id
    {
        return Err(AppError::Conflict(
            "Worker Goal acceptance revision fence changed; refresh and try again".to_string(),
        ));
    }
    if acceptance_requires_pending_action_gate(
        candidate.state,
        before.worker_revision,
        request.expected_worker_revision,
    )? {
        ensure_worker_goal_action(&before, WorkerGoalAction::ResolveAcceptance)?;
    }

    let expected_decision = request.command.decision;
    let result = state
        .hive_runtime
        .resolve_worker_goal_acceptance_for_user(
            current_user_id(user.as_ref()),
            request.command,
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    if result.acceptance_run_id != candidate.acceptance_run_id
        || result.source_run_id != candidate.source_run_id
        || result.workflow_goal_id != candidate.workflow_goal_id
        || result.source_attempt_id != candidate.source_attempt_id
        || result.step_id != candidate.step_id
        || result.decision != expected_decision
        || result.goal_revision <= candidate.goal_revision
    {
        return Err(AppError::Internal(
            "Worker Goal acceptance response conflicts with its frozen candidate".to_string(),
        ));
    }

    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

/// A pending decision must still bind the live Worker projection. A terminal
/// candidate is a lost-response replay: its immutable acceptance record, not a
/// later Worker lifecycle revision, is the authority used by the daemon store.
fn acceptance_requires_pending_action_gate(
    state: WorkerGoalAcceptanceCandidateState,
    current_worker_revision: u64,
    expected_worker_revision: u64,
) -> Result<bool, AppError> {
    match state {
        WorkerGoalAcceptanceCandidateState::AwaitingUser
        | WorkerGoalAcceptanceCandidateState::NeedsUser => {
            if current_worker_revision != expected_worker_revision {
                return Err(AppError::Conflict(
                    "Worker Goal acceptance Worker revision changed; refresh and try again"
                        .to_string(),
                ));
            }
            Ok(true)
        }
        WorkerGoalAcceptanceCandidateState::Accepted
        | WorkerGoalAcceptanceCandidateState::Rejected => Ok(false),
        WorkerGoalAcceptanceCandidateState::Verifying
        | WorkerGoalAcceptanceCandidateState::Stale => Err(AppError::Conflict(
            "Worker Goal acceptance is no longer awaiting a review".to_string(),
        )),
    }
}

async fn transition_worker_goal(
    state: AppState,
    user: Option<CurrentUser>,
    worker_id: String,
    headers: HeaderMap,
    request: TransitionWorkerGoalRequest,
    cancel: bool,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    let action = if cancel { "cancel" } else { "pause" };
    let idempotency_key = required_idempotency_key(&headers, &format!("{action} a Worker Goal"))?;
    let reason = request.reason.trim();
    if reason.is_empty() {
        return Err(AppError::BadRequest(
            "Worker Goal lifecycle reason is required".to_string(),
        ));
    }
    let before = load_worker_goal_projection(&state, user.as_ref(), &worker_id)?;
    ensure_worker_goal_fence(
        &before,
        &request.goal_id,
        request.expected_worker_revision,
        request.expected_goal_revision,
    )?;
    ensure_worker_goal_action(
        &before,
        if cancel {
            WorkerGoalAction::Cancel
        } else {
            WorkerGoalAction::Pause
        },
    )?;
    let command = WorkerWorkflowLifecycleCommand {
        worker_id: worker_id.clone(),
        expected_worker_revision: request.expected_worker_revision,
        goal_id: request.goal_id.clone(),
        expected_goal_revision: request.expected_goal_revision,
        reason: reason.to_string(),
    };
    let result = if cancel {
        state
            .hive_runtime
            .cancel_worker_workflow_for_user(
                current_user_id(user.as_ref()),
                command,
                &idempotency_key,
            )
            .await
    } else {
        state
            .hive_runtime
            .pause_worker_workflow_for_user(
                current_user_id(user.as_ref()),
                command,
                &idempotency_key,
            )
            .await
    }
    .map_err(crate::hive_runtime::control_plane_app_error)?;
    validate_worker_goal_daemon_result(&result, &before, &request.goal_id)?;

    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

pub(super) async fn set_worker_goal_workspace(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SetWorkerGoalWorkspaceRequest>,
) -> Result<Json<WorkerGoalProjection>, AppError> {
    let idempotency_key = required_idempotency_key(&headers, "change a Worker workspace")?;
    let before = load_worker_goal_projection(&state, user.as_ref(), &worker_id)?;
    if before.worker_revision != request.expected_worker_revision {
        return Err(AppError::Conflict(format!(
            "Worker revision changed from {} to {}",
            request.expected_worker_revision, before.worker_revision
        )));
    }
    ensure_worker_goal_action(&before, WorkerGoalAction::SetWorkspace)?;

    if request.workspace_mode == WorkspaceMode::Neutral
        && (request.working_dir.is_some() || request.project_dir.is_some())
    {
        return Err(AppError::BadRequest(
            "A neutral Worker workspace cannot carry filesystem paths".to_string(),
        ));
    }
    let scope = request_workspace_scope(&state, user.as_ref());
    let workspace = normalize_resolved_requested_workspace(
        request.working_dir.as_deref(),
        request.project_dir.as_deref(),
        Some(request.workspace_mode),
        WorkspaceNormalizationPolicy {
            default_mode_without_paths: WorkspaceMode::Neutral,
            selected_fallback_dir: None,
        },
        &scope.base_dir,
        &scope.allowed_root,
    )?;
    if workspace.workspace_mode != WorkspaceMode::Neutral
        && (workspace.working_dir.is_none()
            || workspace.project_dir.is_none()
            || workspace.working_dir != workspace.project_dir)
    {
        return Err(AppError::BadRequest(
            "A Worker Goal workspace requires one identical working and project directory"
                .to_string(),
        ));
    }
    let protocol_mode = match workspace.workspace_mode {
        WorkspaceMode::Neutral => WorkerWorkspaceMode::Neutral,
        WorkspaceMode::Selected => WorkerWorkspaceMode::Selected,
        WorkspaceMode::Created => WorkerWorkspaceMode::Created,
    };
    let expected_workspace_mode = protocol_mode;
    let expected_working_dir = workspace.working_dir.clone();
    let expected_project_dir = workspace.project_dir.clone();
    let result = state
        .hive_runtime
        .set_worker_workspace_for_user(
            current_user_id(user.as_ref()),
            SetWorkerWorkspaceCommand {
                worker_id: worker_id.clone(),
                expected_worker_revision: request.expected_worker_revision,
                workspace_mode: protocol_mode,
                working_dir: workspace.working_dir,
                project_dir: workspace.project_dir,
            },
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    if result.worker_id != worker_id
        || result.session_id != before.session_id
        || result.revision < request.expected_worker_revision
        || result.workspace_mode != expected_workspace_mode
        || result.working_dir != expected_working_dir
        || result.project_dir != expected_project_dir
    {
        return Err(AppError::Internal(
            "Worker workspace response conflicts with the scoped requested workspace".to_string(),
        ));
    }

    Ok(Json(load_worker_goal_projection(
        &state,
        user.as_ref(),
        &worker_id,
    )?))
}

fn load_worker_goal_projection(
    state: &AppState,
    user: Option<&CurrentUser>,
    worker_id: &str,
) -> Result<WorkerGoalProjection, AppError> {
    let worker_store = HiveWorkerStore::new(Database::new(&state.db_path)?);
    let worker = load_owned_worker(&worker_store, worker_id, user)?;
    let session_id = worker
        .dm_session_id
        .clone()
        .ok_or_else(|| AppError::Conflict("Worker has no private DM session".to_string()))?;
    let session_manager = open_session_manager(state)?;
    let session = load_owned_session(&session_manager, &session_id, user)?;
    if session.session_type != SessionType::Hive
        || worker.dm_session_id.as_deref() != Some(session.id.as_str())
    {
        return Err(AppError::NotFound(format!("Worker {worker_id} not found")));
    }

    let db = Database::new(&state.db_path)?;
    let introduction = HiveWorkerIntroductionStore::new(&db).get_by_worker(worker_id)?;
    let introduction_status = introduction
        .as_ref()
        .map(|introduction| introduction.status.as_str().to_string());
    let introduction_ready = introduction.as_ref().is_some_and(|introduction| {
        matches!(
            introduction.status,
            HiveWorkerIntroductionStatus::Confirmed | HiveWorkerIntroductionStatus::Skipped
        )
    });
    let workflow_manager =
        WorkflowManager::new(state.db_path.as_ref().clone()).map_err(map_workflow_error)?;
    let workflow = workflow_manager
        .get_snapshot(&session_id)
        .map_err(map_workflow_error)?;
    let active_run = workflow
        .as_ref()
        .map(|workflow| load_active_worker_goal_run(&db, &worker, workflow))
        .transpose()?
        .flatten();
    let pending_acceptance = workflow
        .as_ref()
        .map(|workflow| {
            load_pending_worker_goal_acceptance(
                state,
                &db,
                current_user_id(user),
                &worker,
                workflow,
            )
        })
        .transpose()?
        .flatten();

    let mut attention = Vec::new();
    if let Some(error) = introduction
        .as_ref()
        .and_then(|introduction| introduction.last_error.as_deref())
    {
        attention.push(error.to_string());
    }
    if active_run
        .as_ref()
        .is_some_and(|run| run.run_status == "recovery_required")
    {
        attention.push("Worker Goal execution needs recovery before it can continue".to_string());
    }
    if pending_acceptance.is_some() {
        attention.push("Review the completed Worker Goal step before it can continue".to_string());
    }

    let read_only_reason = match worker.status {
        HiveWorkerStatus::Archived => Some("This Worker is archived".to_string()),
        HiveWorkerStatus::Paused => Some("This Worker is paused".to_string()),
        HiveWorkerStatus::Active if !introduction_ready => {
            Some("Finish or skip the Worker Introduction first".to_string())
        }
        HiveWorkerStatus::Active if pending_acceptance.is_some() => {
            Some("Review the completed Worker Goal step".to_string())
        }
        HiveWorkerStatus::Active if session.workspace_mode == WorkspaceMode::Neutral => {
            Some("Choose a workspace before creating or starting a Goal".to_string())
        }
        HiveWorkerStatus::Active => None,
    };
    let allowed_actions = worker_goal_actions(
        &worker,
        session.workspace_mode,
        introduction_ready,
        workflow.as_ref(),
        active_run.as_ref(),
        pending_acceptance.is_some(),
    );

    // The current WorkflowManager API owns its own connection, so this HTTP
    // projection cannot share one SQLite read transaction yet. Re-read every
    // revision-bearing seam and fail retryably instead of returning a mixed
    // Worker/Goal/run snapshot if a concurrent daemon mutation crossed it.
    let current_worker = worker_store
        .get(worker_id)?
        .ok_or_else(|| AppError::NotFound(format!("Worker {worker_id} not found")))?;
    let current_session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| AppError::NotFound(format!("Worker {worker_id} not found")))?;
    let current_introduction = HiveWorkerIntroductionStore::new(&db).get_by_worker(worker_id)?;
    let current_workflow = workflow_manager
        .get_snapshot(&session_id)
        .map_err(map_workflow_error)?;
    let current_active_run = current_workflow
        .as_ref()
        .map(|workflow| load_active_worker_goal_run(&db, &current_worker, workflow))
        .transpose()?
        .flatten();
    let current_pending_acceptance = current_workflow
        .as_ref()
        .map(|workflow| {
            load_pending_worker_goal_acceptance(
                state,
                &db,
                current_user_id(user),
                &current_worker,
                workflow,
            )
        })
        .transpose()?
        .flatten();
    if current_worker.revision != worker.revision
        || current_worker.status != worker.status
        || current_worker.user_id != worker.user_id
        || current_worker.dm_session_id != worker.dm_session_id
        || current_session.session_type != session.session_type
        || current_session.user_id != session.user_id
        || current_session.workspace_mode != session.workspace_mode
        || current_session.working_dir != session.working_dir
        || current_session.project_dir != session.project_dir
        || current_introduction != introduction
        || current_workflow != workflow
        || current_active_run != active_run
        || current_pending_acceptance != pending_acceptance
    {
        return Err(AppError::Conflict(
            "Worker Goal changed while its projection was loading; retry".to_string(),
        ));
    }

    Ok(WorkerGoalProjection {
        schema_version: 2,
        worker_id: worker.id,
        worker_revision: worker.revision,
        worker_status: worker.status,
        session_id,
        workspace: WorkerGoalWorkspaceProjection {
            mode: session.workspace_mode,
            working_dir: session.working_dir,
            project_dir: session.project_dir,
        },
        introduction_status,
        introduction_ready,
        workflow,
        active_run,
        pending_acceptance,
        attention,
        read_only_reason,
        allowed_actions,
    })
}

fn load_active_worker_goal_run(
    db: &Database,
    worker: &HiveWorker,
    workflow: &WorkflowSnapshot,
) -> Result<Option<WorkerGoalRunProjection>, AppError> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT run.id, run.status, run.workflow_attempt_id, attempt.status
             FROM hive_runs run
             JOIN workflow_execution_attempts attempt
               ON attempt.id = run.workflow_attempt_id
             WHERE run.worker_id = ?1 AND run.session_id = ?2
               AND run.kind = 'worker_workflow'
               AND run.workflow_goal_id = ?3
               AND run.status IN (
                   'queued', 'leased', 'running', 'sleeping', 'awaiting_input',
                   'retry_wait', 'recovery_required'
               )
             ORDER BY run.created_at DESC, run.id DESC LIMIT 1",
        )
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut rows = statement
        .query((
            worker.id.as_str(),
            workflow.goal.session_id.as_str(),
            workflow.goal.id.as_str(),
        ))
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(row) = rows
        .next()
        .map_err(|error| AppError::Internal(error.to_string()))?
    else {
        return Ok(None);
    };
    Ok(Some(WorkerGoalRunProjection {
        run_id: row
            .get(0)
            .map_err(|error| AppError::Internal(error.to_string()))?,
        run_status: row
            .get(1)
            .map_err(|error| AppError::Internal(error.to_string()))?,
        attempt_id: row
            .get(2)
            .map_err(|error| AppError::Internal(error.to_string()))?,
        attempt_status: row
            .get(3)
            .map_err(|error| AppError::Internal(error.to_string()))?,
    }))
}

fn load_pending_worker_goal_acceptance(
    state: &AppState,
    db: &Database,
    owner_user_id: Option<&str>,
    worker: &HiveWorker,
    workflow: &WorkflowSnapshot,
) -> Result<Option<WorkerGoalPendingAcceptanceProjection>, AppError> {
    let mut statement = db
        .conn()
        .prepare(
            "SELECT acceptance_run_id
             FROM hive_worker_goal_acceptance_candidates
             WHERE worker_id = ?1 AND owner_user_id IS ?2
               AND session_id = ?3 AND workflow_goal_id = ?4
               AND state IN ('awaiting_user', 'needs_user')
             ORDER BY created_at DESC, acceptance_run_id DESC LIMIT 1",
        )
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut rows = statement
        .query((
            worker.id.as_str(),
            owner_user_id,
            workflow.goal.session_id.as_str(),
            workflow.goal.id.as_str(),
        ))
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let acceptance_run_id = rows
        .next()
        .map_err(|error| AppError::Internal(error.to_string()))?
        .map(|row| row.get::<_, String>(0))
        .transpose()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(acceptance_run_id) = acceptance_run_id else {
        return Ok(None);
    };
    let candidate = SqliteWorkerGoalAcceptanceStore::new(state.db_path.as_ref())
        .candidate(&acceptance_run_id, owner_user_id)
        .map_err(|error| AppError::Internal(error.to_string()))?
        .ok_or_else(|| {
            AppError::Conflict(
                "Worker Goal acceptance changed while its projection was loading".to_string(),
            )
        })?;
    if candidate.worker_id != worker.id
        || candidate.worker_revision != worker.revision
        || candidate.session_id != workflow.goal.session_id
        || candidate.workflow_goal_id != workflow.goal.id
        || candidate.goal_revision != workflow.aggregate_revision
        || !matches!(
            candidate.state,
            WorkerGoalAcceptanceCandidateState::AwaitingUser
                | WorkerGoalAcceptanceCandidateState::NeedsUser
        )
    {
        return Err(AppError::Conflict(
            "Worker Goal acceptance binding changed; retry".to_string(),
        ));
    }
    let step = workflow
        .steps
        .iter()
        .find(|step| step.id == candidate.step_id)
        .filter(|step| {
            step.plan_revision_id == candidate.plan_revision_id
                && step.revision == candidate.step_revision
        })
        .ok_or_else(|| {
            AppError::Conflict("Worker Goal acceptance step changed; retry".to_string())
        })?;
    let required_goal_criteria = candidate
        .acceptance_contract
        .goal_specs
        .iter()
        .map(|spec| {
            workflow
                .criteria
                .iter()
                .find(|criterion| criterion.id == spec.criterion_id)
                .map(|criterion| WorkerGoalPendingCriterionProjection {
                    criterion_id: criterion.id.clone(),
                    description: criterion.description.clone(),
                })
                .ok_or_else(|| {
                    AppError::Conflict(
                        "Worker Goal acceptance criterion changed; retry".to_string(),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let is_final_step = workflow.steps.iter().all(|other| {
        other.id == step.id
            || !other.required
            || matches!(
                other.status,
                mitsuro_core::workflow::WorkflowStepStatus::Completed
                    | mitsuro_core::workflow::WorkflowStepStatus::Skipped
            )
    });
    let source_summary = project_worker_goal_acceptance_source(&candidate.source_summary);
    Ok(Some(WorkerGoalPendingAcceptanceProjection {
        acceptance_run_id: candidate.acceptance_run_id,
        source_run_id: candidate.source_run_id,
        goal_id: candidate.workflow_goal_id,
        attempt_id: candidate.source_attempt_id,
        step_id: candidate.step_id,
        expected_worker_revision: candidate.worker_revision,
        expected_goal_revision: candidate.goal_revision,
        step_revision: candidate.step_revision,
        step_description: step.description.clone(),
        is_final_step,
        required_goal_criteria,
        source_summary,
    }))
}

fn project_worker_goal_acceptance_source(
    source: &WorkerGoalAcceptanceSourceSummary,
) -> WorkerGoalAcceptanceSourceProjection {
    WorkerGoalAcceptanceSourceProjection {
        outcome: source.outcome,
        evidence: source
            .evidence
            .iter()
            .map(|evidence| WorkerGoalSourceEvidenceProjection {
                kind: evidence.kind(),
                summary: evidence.summary().to_string(),
            })
            .collect(),
        effect: WorkerGoalSourceEffectProjection {
            summary: source.effect.summary().to_string(),
            workspace_mutated: source.effect.workspace_mutated(),
        },
        counters: source.counters,
    }
}

fn load_owned_acceptance_candidate(
    state: &AppState,
    user: Option<&CurrentUser>,
    worker_id: &str,
    acceptance_run_id: &str,
) -> Result<WorkerGoalAcceptanceCandidateRecord, AppError> {
    let candidate = SqliteWorkerGoalAcceptanceStore::new(state.db_path.as_ref())
        .candidate(acceptance_run_id, current_user_id(user))
        .map_err(|error| AppError::Internal(error.to_string()))?
        .filter(|candidate| candidate.worker_id == worker_id)
        .ok_or_else(|| AppError::NotFound("Worker Goal acceptance was not found".to_string()))?;
    Ok(candidate)
}

fn worker_goal_actions(
    worker: &HiveWorker,
    workspace_mode: WorkspaceMode,
    introduction_ready: bool,
    workflow: Option<&WorkflowSnapshot>,
    active_run: Option<&WorkerGoalRunProjection>,
    pending_acceptance: bool,
) -> Vec<WorkerGoalAction> {
    if worker.status == HiveWorkerStatus::Archived {
        return Vec::new();
    }
    let Some(workflow) = workflow else {
        if worker.status != HiveWorkerStatus::Active || !introduction_ready {
            return Vec::new();
        }
        let mut actions = vec![WorkerGoalAction::SetWorkspace];
        if matches!(
            workspace_mode,
            WorkspaceMode::Selected | WorkspaceMode::Created
        ) {
            actions.push(WorkerGoalAction::CreateGoal);
        }
        return actions;
    };
    if worker.status == HiveWorkerStatus::Paused {
        return matches!(
            workflow.goal.status,
            GoalStatus::Draft | GoalStatus::Active | GoalStatus::Paused | GoalStatus::Blocked
        )
        .then_some(WorkerGoalAction::Cancel)
        .into_iter()
        .collect();
    }

    if pending_acceptance {
        let mut actions = vec![WorkerGoalAction::ResolveAcceptance];
        if matches!(
            workflow.goal.status,
            GoalStatus::Draft | GoalStatus::Active | GoalStatus::Paused | GoalStatus::Blocked
        ) {
            actions.push(WorkerGoalAction::Cancel);
        }
        return actions;
    }

    if !workflow.goal.status.is_unfinished() && introduction_ready {
        let mut actions = vec![WorkerGoalAction::SetWorkspace];
        if matches!(
            workspace_mode,
            WorkspaceMode::Selected | WorkspaceMode::Created
        ) {
            actions.push(WorkerGoalAction::CreateGoal);
        }
        return actions;
    }

    let mut actions = Vec::new();
    if active_run.is_some() {
        actions.push(WorkerGoalAction::Pause);
    }
    if matches!(
        workflow.goal.status,
        GoalStatus::Draft | GoalStatus::Active | GoalStatus::Paused | GoalStatus::Blocked
    ) {
        actions.push(WorkerGoalAction::Cancel);
    }
    if active_run.is_none()
        && introduction_ready
        && workflow
            .plan_revision
            .as_ref()
            .is_some_and(|plan| plan.status == PlanRevisionStatus::Proposed)
    {
        actions.push(WorkerGoalAction::ApprovePlan);
    }
    if active_run.is_none()
        && introduction_ready
        && matches!(
            workspace_mode,
            WorkspaceMode::Selected | WorkspaceMode::Created
        )
        && workflow
            .plan_revision
            .as_ref()
            .is_some_and(|plan| plan.status == PlanRevisionStatus::Active)
        && matches!(
            workflow.goal.status,
            GoalStatus::Draft | GoalStatus::Active | GoalStatus::Paused | GoalStatus::Blocked
        )
    {
        actions.push(WorkerGoalAction::Activate);
    }
    actions
}

fn ensure_worker_goal_fence(
    projection: &WorkerGoalProjection,
    goal_id: &str,
    expected_worker_revision: u64,
    expected_goal_revision: u64,
) -> Result<(), AppError> {
    if expected_worker_revision == 0 || expected_goal_revision == 0 {
        return Err(AppError::BadRequest(
            "Worker and Goal expected revisions must be at least 1".to_string(),
        ));
    }
    let workflow = projection
        .workflow
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Worker Goal not found".to_string()))?;
    if projection.worker_revision != expected_worker_revision
        || workflow.goal.id != goal_id
        || workflow.aggregate_revision != expected_goal_revision
    {
        return Err(AppError::Conflict(
            "Worker Goal revision fence changed; refresh and try again".to_string(),
        ));
    }
    Ok(())
}

fn ensure_worker_goal_action(
    projection: &WorkerGoalProjection,
    action: WorkerGoalAction,
) -> Result<(), AppError> {
    if projection.allowed_actions.contains(&action) {
        return Ok(());
    }
    let action_name = match action {
        WorkerGoalAction::CreateGoal => "create_goal",
        WorkerGoalAction::ApprovePlan => "approve_plan",
        WorkerGoalAction::Activate => "activate",
        WorkerGoalAction::Pause => "pause",
        WorkerGoalAction::Cancel => "cancel",
        WorkerGoalAction::SetWorkspace => "set_workspace",
        WorkerGoalAction::ResolveAcceptance => "resolve_acceptance",
    };
    Err(AppError::Conflict(format!(
        "Worker Goal action {action_name} is not currently allowed"
    )))
}

fn validate_worker_first_plan_provenance(plan: &PlanProposalInput) -> Result<(), AppError> {
    if plan.source_message_id.is_some()
        || plan.predecessor_id.is_some()
        || plan.legacy_markdown.is_some()
    {
        return Err(AppError::BadRequest(
            "A Worker's first plan cannot claim message, predecessor, or legacy provenance"
                .to_string(),
        ));
    }
    Ok(())
}

fn worker_goal_create_operation_id(
    worker_id: &str,
    idempotency_key: &str,
    request: &CreateWorkerGoalRequest,
) -> Result<String, AppError> {
    let request_json =
        serde_json::to_vec(request).map_err(|error| AppError::BadRequest(error.to_string()))?;
    let request_hash = hash_request_bytes(request_json);
    let operation_digest = hash_request_bytes(
        [
            b"worker-goal-create-v1".as_slice(),
            &[0],
            worker_id.as_bytes(),
            &[0],
            idempotency_key.as_bytes(),
            &[0],
            request_hash.as_bytes(),
        ]
        .concat(),
    );
    Ok(format!("worker-goal-create:{operation_digest}"))
}

fn worker_goal_approve_operation_id(
    worker_id: &str,
    idempotency_key: &str,
    request: &ApproveWorkerGoalRequest,
) -> Result<String, AppError> {
    let request_json =
        serde_json::to_vec(request).map_err(|error| AppError::BadRequest(error.to_string()))?;
    let request_hash = hash_request_bytes(request_json);
    let operation_digest = hash_request_bytes(
        [
            b"worker-goal-approve-v1".as_slice(),
            &[0],
            worker_id.as_bytes(),
            &[0],
            idempotency_key.as_bytes(),
            &[0],
            request_hash.as_bytes(),
        ]
        .concat(),
    );
    Ok(format!("worker-goal-approve:{operation_digest}"))
}

fn is_worker_goal_approve_replay_shape(
    projection: &WorkerGoalProjection,
    request: &ApproveWorkerGoalRequest,
) -> bool {
    request.expected_worker_revision > 0
        && request.expected_goal_revision > 0
        && projection.worker_revision == request.expected_worker_revision
        && projection.active_run.is_none()
        && projection.workflow.as_ref().is_some_and(|workflow| {
            workflow.goal.id == request.goal_id
                && request
                    .expected_goal_revision
                    .checked_add(1)
                    .is_some_and(|revision| workflow.aggregate_revision == revision)
                && workflow.goal.status.is_unfinished()
                && workflow.plan_revision.as_ref().is_some_and(|plan| {
                    plan.id == request.plan_revision_id && plan.status == PlanRevisionStatus::Active
                })
        })
}

fn validate_worker_goal_daemon_result(
    result: &mitsuro_hive_protocol::WorkerWorkflowResponse,
    before: &WorkerGoalProjection,
    goal_id: &str,
) -> Result<(), AppError> {
    if result.worker_id != before.worker_id
        || result.session_id != before.session_id
        || result.goal_id != goal_id
        || result.worker_revision < before.worker_revision
    {
        return Err(AppError::Internal(
            "Worker Goal daemon response conflicts with its durable binding".to_string(),
        ));
    }
    Ok(())
}

fn load_owned_worker(
    store: &HiveWorkerStore,
    worker_id: &str,
    user: Option<&CurrentUser>,
) -> Result<HiveWorker, AppError> {
    let worker = store
        .get(worker_id)?
        .ok_or_else(|| AppError::NotFound(format!("Worker {worker_id} not found")))?;
    if worker.user_id.as_deref() != current_user_id(user) {
        return Err(AppError::NotFound(format!("Worker {worker_id} not found")));
    }
    Ok(worker)
}

fn required_idempotency_key(headers: &HeaderMap, action: &str) -> Result<String, AppError> {
    idempotency_key_from_headers(headers)?
        .ok_or_else(|| AppError::BadRequest(format!("Idempotency-Key is required to {action}")))
}

fn map_workflow_error(error: WorkflowError) -> AppError {
    match error {
        WorkflowError::NotFound(message) => AppError::NotFound(message),
        WorkflowError::Conflict(message)
        | WorkflowError::InvalidTransition(message)
        | WorkflowError::WorkspaceRequired(message) => AppError::Conflict(message),
        WorkflowError::Validation(message) => AppError::BadRequest(message),
        WorkflowError::Database(message) => AppError::Internal(message),
        WorkflowError::Sql(error) => AppError::Internal(error.to_string()),
        WorkflowError::Json(error) => AppError::BadRequest(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use mitsuro_core::agent::{WorkerGoalEffectSummary, WorkerGoalEvidence};
    use mitsuro_core::storage::{NewHiveWorker, SessionManager};
    use mitsuro_core::workflow::{CriterionInput, StepProposalInput};

    use super::*;

    fn goal_input() -> CreateGoalInput {
        CreateGoalInput {
            title: "Recover the Worker Goal".into(),
            objective: "Keep lifecycle ownership on the Worker control plane".into(),
            constraints: vec![],
            criteria: vec![CriterionInput {
                description: "The next Worker run is daemon-owned".into(),
                required: true,
            }],
            token_budget: None,
        }
    }

    fn plan_input() -> PlanProposalInput {
        PlanProposalInput {
            title: "Recovery plan".into(),
            rationale: None,
            source_message_id: None,
            predecessor_id: None,
            legacy_markdown: None,
            steps: vec![StepProposalInput {
                display_key: "1".into(),
                description: "Resume through the Worker daemon".into(),
                context: None,
                parent_display_key: None,
                dependencies: vec![],
                acceptance_criteria: vec!["A Worker run owns the step".into()],
                required: true,
            }],
        }
    }

    fn approve_request(
        snapshot: &WorkflowSnapshot,
        worker_revision: u64,
    ) -> ApproveWorkerGoalRequest {
        ApproveWorkerGoalRequest {
            goal_id: snapshot.goal.id.clone(),
            plan_revision_id: snapshot
                .plan_revision
                .as_ref()
                .expect("proposed plan")
                .id
                .clone(),
            expected_worker_revision: worker_revision,
            expected_goal_revision: snapshot.aggregate_revision,
        }
    }

    fn approved_projection(
        worker_id: &str,
        worker_revision: u64,
        snapshot: WorkflowSnapshot,
    ) -> WorkerGoalProjection {
        WorkerGoalProjection {
            schema_version: 1,
            worker_id: worker_id.into(),
            worker_revision,
            worker_status: HiveWorkerStatus::Active,
            session_id: snapshot.goal.session_id.clone(),
            workspace: WorkerGoalWorkspaceProjection {
                mode: WorkspaceMode::Selected,
                working_dir: Some("/tmp/worker".into()),
                project_dir: Some("/tmp/worker".into()),
            },
            introduction_status: Some("confirmed".into()),
            introduction_ready: true,
            workflow: Some(snapshot),
            active_run: None,
            pending_acceptance: None,
            attention: Vec::new(),
            read_only_reason: None,
            allowed_actions: vec![WorkerGoalAction::Activate, WorkerGoalAction::Cancel],
        }
    }

    #[test]
    fn actions_gate_introduction_and_recover_active_goal_without_run() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join("worker-goal-actions.db");
        let sessions = SessionManager::new(Database::new(&path).expect("database"));
        let session_id = sessions
            .create_session(
                "Worker DM",
                None,
                Some(temp.path().to_string_lossy().as_ref()),
            )
            .expect("session");
        let worker_store = HiveWorkerStore::new(Database::new(&path).expect("database"));
        let worker = worker_store
            .create(&NewHiveWorker {
                dm_session_id: Some(session_id.clone()),
                ..NewHiveWorker::new("goal-action-worker")
            })
            .expect("worker");
        let manager = WorkflowManager::new(path).expect("workflow manager");
        let proposed = manager
            .create_goal_with_plan(
                &session_id,
                goal_input(),
                plan_input(),
                "action-create",
                "test",
            )
            .expect("goal and plan");

        let before_introduction = worker_goal_actions(
            &worker,
            WorkspaceMode::Selected,
            false,
            Some(&proposed.snapshot),
            None,
            false,
        );
        assert!(before_introduction.contains(&WorkerGoalAction::Cancel));
        assert!(!before_introduction.contains(&WorkerGoalAction::ApprovePlan));

        let plan_id = proposed
            .snapshot
            .plan_revision
            .as_ref()
            .expect("plan")
            .id
            .clone();
        let approved = manager
            .approve_plan(
                &session_id,
                &proposed.snapshot.goal.id,
                &plan_id,
                proposed.snapshot.aggregate_revision,
                "action-approve",
                "test",
            )
            .expect("approve");
        let legacy_active = manager
            .activate_goal(
                &session_id,
                &approved.snapshot.goal.id,
                approved.snapshot.aggregate_revision,
                "legacy-generic-activate",
                "test",
            )
            .expect("legacy active fixture");
        let recovery_actions = worker_goal_actions(
            &worker,
            WorkspaceMode::Selected,
            true,
            Some(&legacy_active.snapshot),
            None,
            false,
        );
        assert!(recovery_actions.contains(&WorkerGoalAction::Activate));
        assert!(!recovery_actions.contains(&WorkerGoalAction::Pause));
    }

    #[test]
    fn first_worker_plan_rejects_forged_lineage() {
        let mut plan = plan_input();
        plan.predecessor_id = Some("foreign-plan".into());
        assert!(matches!(
            validate_worker_first_plan_provenance(&plan),
            Err(AppError::BadRequest(message)) if message.contains("cannot claim")
        ));
    }

    #[test]
    fn terminal_acceptance_replay_uses_frozen_revision_after_worker_changes() {
        assert!(matches!(
            acceptance_requires_pending_action_gate(
                WorkerGoalAcceptanceCandidateState::Accepted,
                9,
                3,
            ),
            Ok(false)
        ));
        assert!(matches!(
            acceptance_requires_pending_action_gate(
                WorkerGoalAcceptanceCandidateState::Rejected,
                9,
                3,
            ),
            Ok(false)
        ));
        assert!(matches!(
            acceptance_requires_pending_action_gate(
                WorkerGoalAcceptanceCandidateState::AwaitingUser,
                9,
                3,
            ),
            Err(AppError::Conflict(message)) if message.contains("revision changed")
        ));
    }

    #[test]
    fn acceptance_source_projection_is_bounded_typed_and_content_safe() {
        let source = WorkerGoalAcceptanceSourceSummary {
            outcome: WorkerGoalAttemptOutcome::Progressed,
            evidence: vec![WorkerGoalEvidence::new(
                WorkerGoalEvidenceKind::Verification,
                "Focused checks passed",
            )
            .expect("bounded evidence")],
            effect: WorkerGoalEffectSummary::new("Updated the selected workspace", true)
                .expect("bounded effect"),
            counters: WorkerGoalOutcomeCounters {
                provider_calls: 1,
                turns: 2,
                tool_calls: 2,
                successful_tool_calls: 1,
                failed_tool_calls: 1,
                research_actions: 1,
            },
        };
        let value = serde_json::to_value(project_worker_goal_acceptance_source(&source))
            .expect("source projection should serialize");
        assert_eq!(value["outcome"], "progressed");
        assert_eq!(value["evidence"][0]["kind"], "verification");
        assert_eq!(value["effect"]["workspace_mutated"], true);
        assert_eq!(value["counters"]["tool_calls"], 2);
        let encoded = value.to_string();
        for forbidden in [
            "provider_call_ids",
            "raw_tool_output",
            "provider_output",
            "workspace_dir",
            "source_outcome_sha256",
        ] {
            assert!(!encoded.contains(forbidden), "must not expose {forbidden}");
        }
    }

    #[test]
    fn create_keys_replay_exact_body_conflict_on_mismatch_and_scope_per_worker() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join("worker-goal-http-idempotency.db");
        let sessions = SessionManager::new(Database::new(&path).expect("database"));
        let session_a = sessions
            .create_session(
                "Worker A",
                None,
                Some(temp.path().to_string_lossy().as_ref()),
            )
            .expect("Worker A session");
        let session_b = sessions
            .create_session(
                "Worker B",
                None,
                Some(temp.path().to_string_lossy().as_ref()),
            )
            .expect("Worker B session");
        let manager = WorkflowManager::new(path).expect("workflow manager");
        let request = CreateWorkerGoalRequest {
            expected_worker_revision: 3,
            goal: goal_input(),
            plan: plan_input(),
        };
        let operation_a = worker_goal_create_operation_id("worker-a", "shared-key", &request)
            .expect("Worker A operation");
        let created = manager
            .create_goal_with_plan(
                &session_a,
                request.goal.clone(),
                request.plan.clone(),
                &operation_a,
                "test",
            )
            .expect("first create");
        let replayed = manager
            .create_goal_with_plan(
                &session_a,
                request.goal.clone(),
                request.plan.clone(),
                &operation_a,
                "test",
            )
            .expect("same body should replay");
        assert_eq!(replayed, created);

        let mut changed = request.clone();
        changed.goal.title = "Different Goal".into();
        let mismatch = worker_goal_create_operation_id("worker-a", "shared-key", &changed)
            .expect("mismatch operation");
        assert_ne!(mismatch, operation_a);
        assert!(matches!(
            manager.create_goal_with_plan(
                &session_a,
                changed.goal,
                changed.plan,
                &mismatch,
                "test",
            ),
            Err(WorkflowError::Conflict(_))
        ));
        assert_eq!(
            manager
                .get_snapshot(&session_a)
                .expect("Worker A snapshot")
                .expect("Worker A Goal")
                .goal
                .id,
            created.snapshot.goal.id,
            "a different body must not create a second Goal",
        );

        let operation_b = worker_goal_create_operation_id("worker-b", "shared-key", &request)
            .expect("Worker B operation");
        assert_ne!(operation_b, operation_a);
        assert!(manager
            .create_goal_with_plan(
                &session_b,
                request.goal,
                request.plan,
                &operation_b,
                "test",
            )
            .is_ok(), "the same external key must remain independent across Workers");
    }

    #[test]
    fn approve_keys_replay_only_exact_post_commit_shape_and_scope_per_worker() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = temp.path().join("worker-goal-approve-idempotency.db");
        let sessions = SessionManager::new(Database::new(&path).expect("database"));
        let session_a = sessions
            .create_session(
                "Worker A",
                None,
                Some(temp.path().to_string_lossy().as_ref()),
            )
            .expect("Worker A session");
        let session_b = sessions
            .create_session(
                "Worker B",
                None,
                Some(temp.path().to_string_lossy().as_ref()),
            )
            .expect("Worker B session");
        let manager = WorkflowManager::new(path).expect("workflow manager");
        let proposed_a = manager
            .create_goal_with_plan(
                &session_a,
                goal_input(),
                plan_input(),
                "approve-test-create-a",
                "test",
            )
            .expect("Worker A goal");
        let request_a = approve_request(&proposed_a.snapshot, 3);
        let operation_a = worker_goal_approve_operation_id("worker-a", "shared-key", &request_a)
            .expect("Worker A operation");
        let approved_a = manager
            .approve_plan(
                &session_a,
                &request_a.goal_id,
                &request_a.plan_revision_id,
                request_a.expected_goal_revision,
                &operation_a,
                "test",
            )
            .expect("first approval");
        let post_commit = approved_projection("worker-a", 3, approved_a.snapshot.clone());
        assert!(is_worker_goal_approve_replay_shape(
            &post_commit,
            &request_a
        ));
        let replayed = manager
            .approve_plan(
                &session_a,
                &request_a.goal_id,
                &request_a.plan_revision_id,
                request_a.expected_goal_revision,
                &operation_a,
                "test",
            )
            .expect("lost response should replay");
        assert_eq!(replayed, approved_a);

        let mut changed_body = request_a.clone();
        changed_body.expected_worker_revision += 1;
        let changed_operation =
            worker_goal_approve_operation_id("worker-a", "shared-key", &changed_body)
                .expect("changed operation");
        assert_ne!(changed_operation, operation_a);
        assert!(!is_worker_goal_approve_replay_shape(
            &post_commit,
            &changed_body
        ));
        assert!(matches!(
            manager.approve_plan(
                &session_a,
                &changed_body.goal_id,
                &changed_body.plan_revision_id,
                changed_body.expected_goal_revision,
                &changed_operation,
                "test",
            ),
            Err(WorkflowError::Conflict(_))
        ));

        let different_key_operation =
            worker_goal_approve_operation_id("worker-a", "different-key", &request_a)
                .expect("different-key operation");
        assert_ne!(different_key_operation, operation_a);
        assert!(matches!(
            manager.approve_plan(
                &session_a,
                &request_a.goal_id,
                &request_a.plan_revision_id,
                request_a.expected_goal_revision,
                &different_key_operation,
                "test",
            ),
            Err(WorkflowError::Conflict(_))
        ));
        let after_conflicts = manager
            .get_snapshot(&session_a)
            .expect("Worker A snapshot")
            .expect("Worker A Goal");
        assert_eq!(
            after_conflicts.aggregate_revision,
            approved_a.snapshot.aggregate_revision
        );
        assert_eq!(
            after_conflicts
                .plan_revision
                .as_ref()
                .expect("active plan")
                .id,
            request_a.plan_revision_id
        );

        let proposed_b = manager
            .create_goal_with_plan(
                &session_b,
                goal_input(),
                plan_input(),
                "approve-test-create-b",
                "test",
            )
            .expect("Worker B goal");
        let request_b = approve_request(&proposed_b.snapshot, 3);
        let operation_b = worker_goal_approve_operation_id("worker-b", "shared-key", &request_b)
            .expect("Worker B operation");
        assert_ne!(operation_b, operation_a);
        assert!(
            manager
                .approve_plan(
                    &session_b,
                    &request_b.goal_id,
                    &request_b.plan_revision_id,
                    request_b.expected_goal_revision,
                    &operation_b,
                    "test",
                )
                .is_ok(),
            "the same external key must remain independent across Workers"
        );
    }
}
