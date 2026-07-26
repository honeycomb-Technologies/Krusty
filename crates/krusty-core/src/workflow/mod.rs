//! Canonical durable Goal and Plan workflow.
//!
//! Collaboration mode, permissions, user-owned Goals, agent-owned plan
//! revisions, and bounded execution attempts are deliberately independent.
//! Markdown is a presentation/import format, never an execution control plane.

mod manager;
mod model;

#[cfg(test)]
mod tests;

/// Mandatory safety envelope for one automatically continued Goal attempt.
/// User/project overrides may be more restrictive; an active Goal never falls
/// back to the unlimited interactive default.
pub const DEFAULT_GOAL_ATTEMPT_MAX_TURNS: u32 = 16;
pub const DEFAULT_GOAL_ATTEMPT_MAX_TOOL_CALLS: u32 = 96;
pub const DEFAULT_GOAL_ATTEMPT_MAX_WALL_TIME_SECS: u64 = 900;
pub const DEFAULT_GOAL_ATTEMPT_MAX_RESEARCH_ACTIONS: u32 = 24;

pub use manager::{WorkflowError, WorkflowManager};
pub use model::{
    AttemptProgressInput, AttemptStatus, CollaborationMode, CompleteStepInput, CreateGoalInput,
    CriterionInput, CriterionStatus, EditGoalInput, ExecutionAttempt, Goal, GoalCriterion,
    GoalStatus, PlanProposalInput, PlanRevision, PlanRevisionStatus, SetCriterionInput,
    StartAttemptInput, StepDependency, StepProposalInput, WorkflowMutation, WorkflowSnapshot,
    WorkflowStep, WorkflowStepStatus,
};
