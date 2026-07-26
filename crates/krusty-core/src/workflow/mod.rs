//! Canonical durable Goal and Plan workflow.
//!
//! Collaboration mode, permissions, user-owned Goals, agent-owned plan
//! revisions, and bounded execution attempts are deliberately independent.
//! Markdown is a presentation/import format, never an execution control plane.

mod manager;
mod model;

#[cfg(test)]
mod tests;

pub use manager::{WorkflowError, WorkflowManager};
pub use model::{
    AttemptProgressInput, AttemptStatus, CollaborationMode, CompleteStepInput, CreateGoalInput,
    CriterionInput, CriterionStatus, EditGoalInput, ExecutionAttempt, Goal, GoalCriterion,
    GoalStatus, PlanProposalInput, PlanRevision, PlanRevisionStatus, SetCriterionInput,
    StartAttemptInput, StepDependency, StepProposalInput, WorkflowMutation, WorkflowSnapshot,
    WorkflowStep, WorkflowStepStatus,
};
