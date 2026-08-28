use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hive::HiveRunStatus;
use crate::storage::WorkerRunGovernorProjection;

use super::HiveRunExecutionContextV1;

/// Stable durable marker proving that the owner requested Stop for one exact
/// ordinary Worker direct-chat run. Store completion authority matches this
/// value exactly; callers must not reuse it for lifecycle cancellation.
pub const WORKER_CONVERSATION_STOP_REQUESTED_REASON: &str =
    "Worker conversation stop requested by user";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveRunKind {
    Dispatch,
    Scheduled,
    ControllerChild,
    LegacyResume,
    /// One member run of a Hive group turn, executing on the member Worker's
    /// own controller lane.
    GroupTurn,
    /// A Worker-to-Worker delivery that woke the recipient's private DM lane.
    WorkerMessage,
    /// Periodic wake for an always-on Worker on its private DM lane.
    WorkerHeartbeat,
    /// One tool-free first turn in which a new Worker initiates its private
    /// conversation without manufacturing a canonical user message.
    WorkerIntroduction,
    /// One user-visible response on a Worker's serialized private DM lane.
    /// This run kind is never replayed after an uncertain provider boundary.
    WorkerConversation,
    /// One bounded attempt against a user-approved durable Workflow Goal.
    /// The run commits a typed Goal outcome rather than a chat response and
    /// is never replayed after crossing a provider or workspace boundary.
    WorkerWorkflow,
    /// One tool-free, transcript-frozen Introduction review. Its terminal
    /// authority is the linked review audit, never an assistant response.
    WorkerIntroductionReview,
    /// One immutable owner-facing decision boundary for a committed
    /// `Progressed` Worker Workflow result. V1 is never claimed or executed;
    /// it remains `awaiting_input` until an exact owner or lifecycle result.
    WorkerWorkflowAcceptance,
}

impl HiveRunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Scheduled => "scheduled",
            Self::ControllerChild => "controller_child",
            Self::LegacyResume => "legacy_resume",
            Self::GroupTurn => "group_turn",
            Self::WorkerMessage => "worker_message",
            Self::WorkerHeartbeat => "worker_heartbeat",
            Self::WorkerIntroduction => "worker_introduction",
            Self::WorkerConversation => "worker_conversation",
            Self::WorkerWorkflow => "worker_workflow",
            Self::WorkerIntroductionReview => "worker_introduction_review",
            Self::WorkerWorkflowAcceptance => "worker_workflow_acceptance",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "dispatch" => Some(Self::Dispatch),
            "scheduled" => Some(Self::Scheduled),
            "controller_child" => Some(Self::ControllerChild),
            "legacy_resume" => Some(Self::LegacyResume),
            "group_turn" => Some(Self::GroupTurn),
            "worker_message" => Some(Self::WorkerMessage),
            "worker_heartbeat" => Some(Self::WorkerHeartbeat),
            "worker_introduction" => Some(Self::WorkerIntroduction),
            "worker_conversation" => Some(Self::WorkerConversation),
            "worker_workflow" => Some(Self::WorkerWorkflow),
            "worker_introduction_review" => Some(Self::WorkerIntroductionReview),
            "worker_workflow_acceptance" => Some(Self::WorkerWorkflowAcceptance),
            _ => None,
        }
    }

    /// Group turns, peer deliveries, and heartbeats are idempotent enough to
    /// requeue after a crashed `running` lease. User-facing dispatch and
    /// calendar occurrences stay in `recovery_required` so side effects are
    /// not silently replayed.
    pub fn replays_after_expired_running(self) -> bool {
        matches!(
            self,
            Self::GroupTurn | Self::WorkerMessage | Self::WorkerHeartbeat
        )
    }
}

impl std::fmt::Display for HiveRunKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveRun {
    pub id: String,
    pub controller_id: String,
    pub session_id: Option<String>,
    pub schedule_id: Option<String>,
    pub occurrence_id: Option<String>,
    /// Authoritative Worker linkage; never recover this identity from config.
    #[serde(default)]
    pub worker_id: Option<String>,
    /// Canonical user/objective message that caused this run.
    #[serde(default)]
    pub objective_message_id: Option<i64>,
    pub kind: HiveRunKind,
    pub objective: String,
    pub config: Value,
    /// Frozen capability, revision, and conversation-lane binding.
    #[serde(default)]
    pub execution_context: Option<HiveRunExecutionContextV1>,
    /// Inclusive canonical transcript boundary for this response.
    #[serde(default)]
    pub conversation_through_message_id: Option<i64>,
    /// Deterministically keyed final assistant row, once committed.
    #[serde(default)]
    pub response_message_id: Option<i64>,
    /// Exact provider Started row whose visible output became the response.
    #[serde(default)]
    pub response_provider_call_id: Option<String>,
    /// Deterministically keyed group-room projection, when this is a member run.
    #[serde(default)]
    pub response_group_message_id: Option<String>,
    /// Canonical durable Workflow Goal bound to this run, when the run is a
    /// bounded Worker Workflow attempt.
    #[serde(default)]
    pub workflow_goal_id: Option<String>,
    /// Canonical Workflow execution attempt.  One attempt can back exactly
    /// one Hive run.
    #[serde(default)]
    pub workflow_attempt_id: Option<String>,
    /// Migration-74 gate/origin projection loaded from authoritative columns.
    #[serde(default)]
    pub governor: Option<WorkerRunGovernorProjection>,
    pub status: HiveRunStatus,
    pub priority: i32,
    pub concurrency_key: Option<String>,
    pub scheduled_for: Option<String>,
    pub available_at: String,
    pub wake_at: Option<String>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub lease_owner: Option<String>,
    pub lease_token: Option<String>,
    pub lease_epoch: Option<u64>,
    pub lease_expires_at: Option<String>,
    pub heartbeat_at: Option<String>,
    pub last_stop_reason: Option<String>,
    pub last_error: Option<String>,
    pub outcome: Option<Value>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveRunAttemptOutcome {
    Leased,
    Succeeded,
    Failed,
    RetryScheduled,
    Sleeping,
    AwaitingInput,
    RecoveryRequired,
    Cancelled,
    Abandoned,
    DeadLetter,
}

impl HiveRunAttemptOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::RetryScheduled => "retry_scheduled",
            Self::Sleeping => "sleeping",
            Self::AwaitingInput => "awaiting_input",
            Self::RecoveryRequired => "recovery_required",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
            Self::DeadLetter => "dead_letter",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "leased" => Some(Self::Leased),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "retry_scheduled" => Some(Self::RetryScheduled),
            "sleeping" => Some(Self::Sleeping),
            "awaiting_input" => Some(Self::AwaitingInput),
            "recovery_required" => Some(Self::RecoveryRequired),
            "cancelled" => Some(Self::Cancelled),
            "abandoned" => Some(Self::Abandoned),
            "dead_letter" => Some(Self::DeadLetter),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveRunAttempt {
    pub id: String,
    pub run_id: String,
    pub attempt_no: u32,
    /// Daemon instance that claimed this attempt. This is execution-plane
    /// identity; "worker" is reserved for the Hive Worker product concept.
    #[serde(alias = "worker_id")]
    pub executor_id: String,
    pub lease_token: String,
    pub lease_epoch: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub outcome: HiveRunAttemptOutcome,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    pub retry_at: Option<String>,
    pub trace_sequence_start: Option<i64>,
    pub trace_sequence_end: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ClaimRunRequest {
    /// Claiming daemon instance id, persisted as the attempt's executor.
    pub executor_id: String,
    pub lease_epoch: u64,
    pub now: DateTime<Utc>,
    pub lease_duration: Duration,
    pub global_concurrency_limit: u32,
}

/// The scheduler-level lease that fences an executor mutation. Run leases
/// stop duplicate executors for one run; this additional fence stops an
/// entire stale daemon generation after another process takes over the
/// scheduler lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonFence {
    pub lease_name: String,
    pub owner_id: String,
    pub fencing_token: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedHiveRun {
    pub run: HiveRun,
    pub attempt_id: String,
    pub attempt_no: u32,
    pub lease_token: String,
}

#[derive(Debug, Clone)]
pub struct RunCompletion {
    pub target_status: HiveRunStatus,
    pub now: DateTime<Utc>,
    pub available_at: Option<DateTime<Utc>>,
    pub wake_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    pub outcome: Option<Value>,
    pub trace_sequence_end: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaseReconciliation {
    pub requeued_unstarted: usize,
    pub recovery_required: usize,
    /// Expired Worker Introduction attempts whose canonical first assistant
    /// message was already committed before the executor disappeared.
    pub recovered_succeeded: usize,
    /// Expired attempts with an exact terminal failure audit, including an
    /// acknowledged semantic-invalid reviewer response.
    pub recovered_failed: usize,
    /// Expired ordinary Worker conversations whose exact typed Stop marker
    /// was committed before the prior daemon disappeared.
    pub recovered_cancelled: usize,
    pub requeued_runs: Vec<ReconciledRun>,
    pub recovery_required_runs: Vec<ReconciledRun>,
    pub recovered_succeeded_runs: Vec<ReconciledRun>,
    pub recovered_failed_runs: Vec<ReconciledRun>,
    pub recovered_cancelled_runs: Vec<ReconciledRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledRun {
    pub run_id: String,
    pub attempt_no: u32,
}
