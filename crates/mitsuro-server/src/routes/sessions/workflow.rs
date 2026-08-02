use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;

use mitsuro_core::workflow::{
    AttemptProgressInput, AttemptStatus, CompleteStepInput, CreateGoalInput, EditGoalInput,
    PlanProposalInput, SetCriterionInput, StartAttemptInput, WorkflowError, WorkflowManager,
    WorkflowMutation, WorkflowSnapshot,
};

use super::{load_owned_session, open_session_manager};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum WorkflowCommand {
    CreateGoal {
        operation_id: String,
        goal: CreateGoalInput,
    },
    ImportLegacyPlan {
        operation_id: String,
        goal: CreateGoalInput,
    },
    EditGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
        goal: EditGoalInput,
    },
    ProposePlan {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
        plan: PlanProposalInput,
    },
    ApprovePlan {
        operation_id: String,
        goal_id: String,
        plan_revision_id: String,
        expected_revision: u64,
    },
    ActivateGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
    },
    PauseGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
        reason: String,
    },
    ResumeGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
    },
    BlockGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
        reason: String,
    },
    CancelGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
        reason: Option<String>,
    },
    StartAttempt {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
        attempt: StartAttemptInput,
    },
    ClaimStep {
        operation_id: String,
        goal_id: String,
        attempt_id: String,
        step_id: String,
        expected_revision: u64,
    },
    RecordAttemptProgress {
        operation_id: String,
        goal_id: String,
        attempt_id: String,
        expected_revision: u64,
        progress: AttemptProgressInput,
    },
    CompleteStep {
        operation_id: String,
        goal_id: String,
        step_id: String,
        expected_revision: u64,
        completion: CompleteStepInput,
    },
    FinishAttempt {
        operation_id: String,
        goal_id: String,
        attempt_id: String,
        expected_revision: u64,
        status: AttemptStatus,
        reason: String,
    },
    SetCriterion {
        operation_id: String,
        goal_id: String,
        criterion_id: String,
        expected_revision: u64,
        criterion: SetCriterionInput,
    },
    CompleteGoal {
        operation_id: String,
        goal_id: String,
        expected_revision: u64,
    },
}

pub(super) async fn get_workflow(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
) -> Result<Json<Option<WorkflowSnapshot>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    load_owned_session(&session_manager, &session_id, user.as_ref())?;
    let manager =
        WorkflowManager::new(state.db_path.as_ref().clone()).map_err(map_workflow_error)?;
    Ok(Json(
        manager
            .get_snapshot(&session_id)
            .map_err(map_workflow_error)?,
    ))
}

pub(super) async fn execute_workflow_command(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
    Json(command): Json<WorkflowCommand>,
) -> Result<Json<WorkflowMutation>, AppError> {
    let session_manager = open_session_manager(&state)?;
    load_owned_session(&session_manager, &session_id, user.as_ref())?;
    let manager =
        WorkflowManager::new(state.db_path.as_ref().clone()).map_err(map_workflow_error)?;

    let mutation = match command {
        WorkflowCommand::CreateGoal { operation_id, goal } => {
            manager.create_goal(&session_id, goal, &operation_id, "user")
        }
        WorkflowCommand::ImportLegacyPlan { operation_id, goal } => {
            manager.import_legacy_plan(&session_id, goal, &operation_id, "user")
        }
        WorkflowCommand::EditGoal {
            operation_id,
            goal_id,
            expected_revision,
            goal,
        } => manager.edit_goal(
            &session_id,
            &goal_id,
            expected_revision,
            goal,
            &operation_id,
            "user",
        ),
        WorkflowCommand::ProposePlan {
            operation_id,
            goal_id,
            expected_revision,
            plan,
        } => manager.propose_plan(
            &session_id,
            &goal_id,
            expected_revision,
            plan,
            &operation_id,
            "user",
        ),
        WorkflowCommand::ApprovePlan {
            operation_id,
            goal_id,
            plan_revision_id,
            expected_revision,
        } => manager.approve_plan(
            &session_id,
            &goal_id,
            &plan_revision_id,
            expected_revision,
            &operation_id,
            "user",
        ),
        WorkflowCommand::ActivateGoal {
            operation_id,
            goal_id,
            expected_revision,
        } => manager.activate_goal(
            &session_id,
            &goal_id,
            expected_revision,
            &operation_id,
            "user",
        ),
        WorkflowCommand::PauseGoal {
            operation_id,
            goal_id,
            expected_revision,
            reason,
        } => manager.pause_goal(
            &session_id,
            &goal_id,
            expected_revision,
            Some(&reason),
            &operation_id,
            "user",
        ),
        WorkflowCommand::ResumeGoal {
            operation_id,
            goal_id,
            expected_revision,
        } => manager.resume_goal(
            &session_id,
            &goal_id,
            expected_revision,
            &operation_id,
            "user",
        ),
        WorkflowCommand::BlockGoal {
            operation_id,
            goal_id,
            expected_revision,
            reason,
        } => manager.block_goal(
            &session_id,
            &goal_id,
            expected_revision,
            &reason,
            &operation_id,
            "user",
        ),
        WorkflowCommand::CancelGoal {
            operation_id,
            goal_id,
            expected_revision,
            reason,
        } => manager.cancel_goal(
            &session_id,
            &goal_id,
            expected_revision,
            reason.as_deref(),
            &operation_id,
            "user",
        ),
        WorkflowCommand::StartAttempt {
            operation_id,
            goal_id,
            expected_revision,
            attempt,
        } => manager.start_attempt(
            &session_id,
            &goal_id,
            expected_revision,
            attempt,
            &operation_id,
            "user",
        ),
        WorkflowCommand::ClaimStep {
            operation_id,
            goal_id,
            attempt_id,
            step_id,
            expected_revision,
        } => manager.claim_step(
            &session_id,
            &goal_id,
            &attempt_id,
            &step_id,
            expected_revision,
            &operation_id,
            "user",
        ),
        WorkflowCommand::RecordAttemptProgress {
            operation_id,
            goal_id,
            attempt_id,
            expected_revision,
            progress,
        } => manager.record_attempt_progress(
            &session_id,
            &goal_id,
            &attempt_id,
            expected_revision,
            progress,
            &operation_id,
            "user",
        ),
        WorkflowCommand::CompleteStep {
            operation_id,
            goal_id,
            step_id,
            expected_revision,
            completion,
        } => manager.complete_step(
            &session_id,
            &goal_id,
            &step_id,
            expected_revision,
            completion,
            &operation_id,
            "user",
        ),
        WorkflowCommand::FinishAttempt {
            operation_id,
            goal_id,
            attempt_id,
            expected_revision,
            status,
            reason,
        } => manager.finish_attempt(
            &session_id,
            &goal_id,
            &attempt_id,
            expected_revision,
            status,
            &reason,
            &operation_id,
            "user",
        ),
        WorkflowCommand::SetCriterion {
            operation_id,
            goal_id,
            criterion_id,
            expected_revision,
            criterion,
        } => manager.set_criterion(
            &session_id,
            &goal_id,
            &criterion_id,
            expected_revision,
            criterion,
            &operation_id,
            "user",
        ),
        WorkflowCommand::CompleteGoal {
            operation_id,
            goal_id,
            expected_revision,
        } => manager.complete_goal(
            &session_id,
            &goal_id,
            expected_revision,
            &operation_id,
            "user",
        ),
    }
    .map_err(map_workflow_error)?;

    // Collaboration mode remains a conversational posture. Only the explicit
    // user activation command switches the session into executable Build mode.
    if matches!(
        mutation.snapshot.goal.status,
        mitsuro_core::workflow::GoalStatus::Active
    ) {
        session_manager
            .update_session_work_mode(&session_id, mitsuro_core::storage::WorkMode::Build)?;
    }

    Ok(Json(mutation))
}

fn map_workflow_error(error: WorkflowError) -> AppError {
    match error {
        WorkflowError::NotFound(message) => AppError::NotFound(message),
        WorkflowError::Conflict(message) | WorkflowError::InvalidTransition(message) => {
            AppError::Conflict(message)
        }
        WorkflowError::Validation(message) => AppError::BadRequest(message),
        WorkflowError::Database(message) => AppError::Internal(message),
        WorkflowError::Sql(error) => AppError::Internal(error.to_string()),
        WorkflowError::Json(error) => AppError::BadRequest(error.to_string()),
    }
}
