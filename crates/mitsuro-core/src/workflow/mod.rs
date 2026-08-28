//! Canonical durable Goal and Plan workflow.
//!
//! Collaboration mode, permissions, user-owned Goals, agent-owned plan
//! revisions, and bounded execution attempts are deliberately independent.
//! Markdown is a presentation/import format, never an execution control plane.

mod acceptance;
mod manager;
mod model;
mod worker;

#[cfg(test)]
mod tests;

/// Safety envelope for one Goal execution attempt. Exhausting an attempt rolls
/// the claimed step back to ready work; it does not terminate the approved
/// Goal. Explicit parent-run and token ceilings remain separate contracts.
pub const DEFAULT_GOAL_ATTEMPT_MAX_TURNS: u32 = 24;
pub const DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS: u32 = 96;
pub const DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS: u64 = 900;
// Research reads are counted per tool invocation, including every member of a
// parallel read batch. Keep this finite, but high enough that a normal
// multi-file implementation pass does not exhaust the budget before editing.
pub const DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS: u32 = 48;

pub use acceptance::{
    UserGoalCriterionAcceptance, UserGoalCriterionDecision, UserWorkerGoalAcceptanceDecision,
    UserWorkerGoalAcceptanceRequest, WorkflowAcceptanceModeV1, WorkflowAcceptanceSpecV1,
    WorkflowAcceptanceValidationError, WorkflowPathExpectationV1, WorkflowStructuralCheckV1,
    MAX_USER_ACCEPTANCE_CRITERIA, MAX_USER_ACCEPTANCE_EVIDENCE_ITEMS,
    MAX_WORKFLOW_ACCEPTANCE_ARGUMENTS, MAX_WORKFLOW_ACCEPTANCE_CHECKS,
    MAX_WORKFLOW_ACCEPTANCE_TEXT_BYTES, MAX_WORKFLOW_ACCEPTANCE_TIMEOUT_SECS,
    MAX_WORKFLOW_ACCEPTANCE_TOTAL_ARGUMENT_BYTES, WORKFLOW_ACCEPTANCE_SPEC_VERSION,
};

pub use manager::{pause_worker_goals_for_archive_in_transaction, WorkflowError, WorkflowManager};
pub use model::{
    AttemptProgressInput, AttemptStatus, CollaborationMode, CompleteStepInput, CreateGoalInput,
    CriterionInput, CriterionStatus, EditGoalInput, ExecutionAttempt, Goal, GoalCriterion,
    GoalStatus, PlanProposalInput, PlanRevision, PlanRevisionStatus, SetCriterionInput,
    StartAttemptInput, StepDependency, StepProposalInput, WorkflowMutation, WorkflowSnapshot,
    WorkflowStep, WorkflowStepStatus,
};
pub use worker::{
    activate_or_resume_worker_workflow_in_transaction,
    archive_worker_goal_acceptances_in_transaction, cancel_worker_workflow_in_transaction,
    finalize_worker_workflow_attempt_in_transaction,
    materialize_due_worker_workflow_rollovers_in_transaction, pause_worker_workflow_in_transaction,
    reconcile_worker_workflow_run_in_transaction, WorkerWorkflowActivation,
    WorkerWorkflowActivationDisposition, WorkerWorkflowActivationRequest,
    WorkerWorkflowActivationSource, WorkerWorkflowLifecycleRequest, WorkerWorkflowLifecycleResult,
    WorkerWorkflowReconciliation,
};
