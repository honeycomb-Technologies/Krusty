use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(format!(
                        "unknown {} value: {other}",
                        stringify!($name)
                    )),
                }
            }
        }
    };
}

string_enum! {
    /// The conversational posture for a session. It does not grant authority or
    /// activate durable work.
    pub enum CollaborationMode {
        Default => "default",
        Plan => "plan"
    }
}

string_enum! {
    /// User-owned durable Goal lifecycle.
    pub enum GoalStatus {
        Draft => "draft",
        Active => "active",
        Paused => "paused",
        Blocked => "blocked",
        Completed => "completed",
        Cancelled => "cancelled"
    }
}

impl GoalStatus {
    pub const fn is_unfinished(self) -> bool {
        matches!(
            self,
            Self::Draft | Self::Active | Self::Paused | Self::Blocked
        )
    }
}

string_enum! {
    pub enum CriterionStatus {
        Pending => "pending",
        Passed => "passed",
        Failed => "failed",
        Waived => "waived"
    }
}

string_enum! {
    pub enum PlanRevisionStatus {
        Proposed => "proposed",
        Approved => "approved",
        Active => "active",
        Superseded => "superseded",
        Completed => "completed",
        Cancelled => "cancelled"
    }
}

string_enum! {
    pub enum WorkflowStepStatus {
        Pending => "pending",
        InProgress => "in_progress",
        Blocked => "blocked",
        Completed => "completed",
        Failed => "failed",
        Skipped => "skipped",
        Cancelled => "cancelled"
    }
}

impl WorkflowStepStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Skipped | Self::Cancelled
        )
    }
}

string_enum! {
    pub enum AttemptStatus {
        Running => "running",
        Paused => "paused",
        Succeeded => "succeeded",
        Failed => "failed",
        Cancelled => "cancelled"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Goal {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub objective: String,
    pub constraints: Vec<String>,
    pub status: GoalStatus,
    pub status_reason: Option<String>,
    pub needs_definition: bool,
    pub revision: u64,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub source: String,
    pub legacy_plan_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub activated_at: Option<String>,
    pub completed_at: Option<String>,
    pub cancelled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalCriterion {
    pub id: String,
    pub goal_id: String,
    pub position: u32,
    pub description: String,
    pub required: bool,
    pub status: CriterionStatus,
    pub evidence: Vec<String>,
    pub verifier: Option<String>,
    pub verified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanRevision {
    pub id: String,
    pub goal_id: String,
    pub revision_number: u64,
    pub status: PlanRevisionStatus,
    pub title: String,
    pub rationale: Option<String>,
    pub source_message_id: Option<i64>,
    pub predecessor_id: Option<String>,
    pub legacy_markdown: Option<String>,
    pub created_at: String,
    pub approved_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStep {
    pub id: String,
    pub plan_revision_id: String,
    pub parent_step_id: Option<String>,
    pub display_key: String,
    pub position: u32,
    pub description: String,
    pub context: Option<String>,
    pub acceptance_criteria: Vec<String>,
    pub required: bool,
    pub status: WorkflowStepStatus,
    pub outcome: Option<String>,
    pub evidence: Vec<String>,
    pub claimed_attempt_id: Option<String>,
    pub revision: u64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepDependency {
    pub step_id: String,
    pub depends_on_step_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAttempt {
    pub id: String,
    pub goal_id: String,
    pub plan_revision_id: Option<String>,
    pub step_id: Option<String>,
    pub status: AttemptStatus,
    pub stop_reason: Option<String>,
    pub permission_mode: String,
    pub goal_revision_at_start: u64,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_wall_time_secs: u64,
    pub max_research_actions: u32,
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub research_action_count: u32,
    pub progress_revision: u64,
    pub blocker_fingerprint: Option<String>,
    pub blocker_streak: u32,
    pub started_at: String,
    pub updated_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSnapshot {
    pub schema_version: u32,
    pub aggregate_revision: u64,
    pub collaboration_mode: CollaborationMode,
    pub permission_mode: String,
    pub goal: Goal,
    pub criteria: Vec<GoalCriterion>,
    pub plan_revision: Option<PlanRevision>,
    pub steps: Vec<WorkflowStep>,
    pub dependencies: Vec<StepDependency>,
    pub latest_attempt: Option<ExecutionAttempt>,
    pub allowed_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowMutation {
    pub changed: bool,
    pub operation_id: String,
    pub snapshot: WorkflowSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CriterionInput {
    pub description: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateGoalInput {
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub criteria: Vec<CriterionInput>,
    pub token_budget: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditGoalInput {
    pub title: Option<String>,
    pub objective: Option<String>,
    pub constraints: Option<Vec<String>>,
    pub criteria: Option<Vec<CriterionInput>>,
    pub token_budget: Option<Option<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepProposalInput {
    pub display_key: String,
    pub description: String,
    pub context: Option<String>,
    #[serde(default)]
    pub parent_display_key: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanProposalInput {
    pub title: String,
    pub rationale: Option<String>,
    pub source_message_id: Option<i64>,
    #[serde(default)]
    pub predecessor_id: Option<String>,
    #[serde(default)]
    pub legacy_markdown: Option<String>,
    pub steps: Vec<StepProposalInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartAttemptInput {
    pub step_id: Option<String>,
    pub permission_mode: String,
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_wall_time_secs: u64,
    pub max_research_actions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptProgressInput {
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub research_action_count: u32,
    pub material_progress: bool,
    pub blocker_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompleteStepInput {
    pub attempt_id: String,
    pub outcome: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetCriterionInput {
    pub status: CriterionStatus,
    pub evidence: Vec<String>,
    pub verifier: String,
}

pub(crate) const fn default_true() -> bool {
    true
}
