//! Canonical durable Goal and Plan workflow.
//!
//! Collaboration mode, permissions, user-owned Goals, agent-owned plan
//! revisions, and bounded execution attempts are deliberately independent.
//! Markdown is a presentation/import format, never an execution control plane.

mod manager;
mod model;

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

pub use manager::{WorkflowError, WorkflowManager};
pub use model::{
    AttemptProgressInput, AttemptStatus, CollaborationMode, CompleteStepInput, CreateGoalInput,
    CriterionInput, CriterionStatus, EditGoalInput, ExecutionAttempt, Goal, GoalCriterion,
    GoalStatus, PlanProposalInput, PlanRevision, PlanRevisionStatus, SetCriterionInput,
    StartAttemptInput, StepDependency, StepProposalInput, WorkflowMutation, WorkflowSnapshot,
    WorkflowStep, WorkflowStepStatus,
};
