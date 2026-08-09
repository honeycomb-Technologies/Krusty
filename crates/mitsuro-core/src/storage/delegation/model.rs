use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::ai::models::ModelKey;
use crate::storage::{DelegatedRunRole, DelegatedRunScope};
use crate::tools::registry::{DelegationPolicy, PermissionMode};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationExecutionMode {
    Foreground,
    Detached,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DelegationCompletionPolicy {
    AllSettled,
    AnySuccess,
    Quorum { required: usize },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationFailurePolicy {
    Continue,
    FailFast,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DelegationWriterMode {
    #[default]
    Shared,
    Isolated,
}

pub const DELEGATION_EXECUTOR_ENVELOPE_VERSION: u16 = 1;
/// Durable task objectives include the bounded project instruction bundle plus
/// the coordinator's assignment and recovery wrapper. Project instructions
/// alone may consume 32 KiB, so the task envelope needs explicit headroom while
/// remaining small enough for bounded replay and session projection.
pub const MAX_DELEGATION_TASK_OBJECTIVE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationExecutorKind {
    Normal,
    Explore,
    Build,
    Plan,
    Verify,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationExecutorSessionType {
    Chat,
    Code,
}

/// Minimal reconstruction metadata for a detached child. The child objective
/// remains in the bounded immutable task specification and is authenticated by
/// `objective_sha256`; this envelope never copies parent transcript messages,
/// raw tool results, or provider output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DelegationExecutorEnvelopeV1 {
    pub version: u16,
    pub session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub session_type: DelegationExecutorSessionType,
    pub user_id: Option<String>,
    pub task_id: String,
    pub task_name: String,
    pub kind: DelegationExecutorKind,
    pub role: DelegatedRunRole,
    pub provider_id: String,
    /// Exact provider, auth surface, and transport identity used by the child.
    pub model_key: ModelKey,
    /// Exact model identifier sent to the provider after resolving `model_key`.
    pub resolved_model: String,
    pub working_dir: String,
    pub project_dir: Option<String>,
    pub sandbox_root: String,
    pub objective_sha256: String,
}

impl DelegationExecutorEnvelopeV1 {
    pub(crate) fn invalid(task_id: &str, role: DelegatedRunRole) -> Self {
        Self {
            version: 0,
            session_id: String::new(),
            parent_tool_call_id: None,
            session_type: DelegationExecutorSessionType::Code,
            user_id: None,
            task_id: task_id.to_string(),
            task_name: String::new(),
            kind: DelegationExecutorKind::Normal,
            role,
            provider_id: String::new(),
            model_key: ModelKey::new(Default::default(), "", Default::default()),
            resolved_model: String::new(),
            working_dir: String::new(),
            project_dir: None,
            sandbox_root: String::new(),
            objective_sha256: String::new(),
        }
    }

    pub fn objective_digest(objective: &str) -> String {
        format!("{:x}", Sha256::digest(objective.as_bytes()))
    }

    pub fn validate(&self, objective: &str) -> Result<()> {
        ensure!(
            self.version == DELEGATION_EXECUTOR_ENVELOPE_VERSION,
            "unsupported delegation executor envelope version"
        );
        for (value, label, limit) in [
            (self.session_id.as_str(), "session id", 512usize),
            (self.task_id.as_str(), "task id", 512),
            (self.task_name.as_str(), "task name", 512),
            (self.provider_id.as_str(), "provider id", 512),
            (self.model_key.model_id.as_str(), "model key id", 2 * 1024),
            (self.resolved_model.as_str(), "resolved model", 2 * 1024),
            (self.working_dir.as_str(), "working directory", 4 * 1024),
            (self.sandbox_root.as_str(), "sandbox root", 4 * 1024),
        ] {
            ensure!(!value.trim().is_empty(), "executor {label} is required");
            ensure!(
                value.len() <= limit,
                "executor {label} exceeds its size limit"
            );
        }
        ensure!(
            self.provider_id == self.model_key.provider.to_string(),
            "executor provider differs from its exact model key"
        );
        ensure!(
            self.parent_tool_call_id
                .as_ref()
                .is_none_or(|value| value.len() <= 512),
            "executor parent tool call id exceeds its size limit"
        );
        ensure!(
            self.user_id.as_ref().is_none_or(|value| value.len() <= 512),
            "executor user id exceeds its size limit"
        );
        ensure!(
            self.project_dir
                .as_ref()
                .is_none_or(|value| value.len() <= 4 * 1024),
            "executor project directory exceeds its size limit"
        );
        ensure!(
            self.objective_sha256 == Self::objective_digest(objective),
            "executor objective digest does not match its immutable task"
        );
        Ok(())
    }
}

/// Immutable limits inherited from the parent at group creation time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationGovernance {
    pub permission_mode: PermissionMode,
    pub delegated_turn_budget: usize,
    pub max_parallelism: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_tool_allowlist: Option<BTreeSet<String>>,
    /// Exact executable child policy. Admission equality-checks the runtime
    /// task against this stored contract instead of trusting caller metadata.
    pub delegation_policy: DelegationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationGroupContract {
    pub execution_mode: DelegationExecutionMode,
    pub completion_policy: DelegationCompletionPolicy,
    pub failure_policy: DelegationFailurePolicy,
    pub governance: DelegationGovernance,
}

impl DelegationGroupContract {
    pub fn validate(&self, task_count: usize) -> Result<()> {
        ensure!(
            task_count > 0,
            "a delegation group requires at least one task"
        );
        ensure!(task_count <= 128, "delegation group exceeds the task limit");
        ensure!(
            self.governance.delegated_turn_budget > 0,
            "delegated turn budget must be greater than zero"
        );
        ensure!(
            self.governance.max_parallelism > 0,
            "delegation max parallelism must be greater than zero"
        );
        ensure!(
            self.governance.max_parallelism <= task_count,
            "delegation max parallelism cannot exceed task count"
        );
        if let DelegationCompletionPolicy::Quorum { required } = self.completion_policy {
            ensure!(required > 0, "delegation quorum must be greater than zero");
            ensure!(
                required <= task_count,
                "delegation quorum cannot exceed task count"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationGroupState {
    Created,
    Queued,
    Running,
    ReadyForParent,
    Synthesizing,
    Complete,
    Degraded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationParentContinuationState {
    NotRequested,
    Pending,
    Queued,
    Promoted,
}

impl DelegationParentContinuationState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::Pending => "pending",
            Self::Queued => "queued",
            Self::Promoted => "promoted",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "not_requested" => Self::NotRequested,
            "pending" => Self::Pending,
            "queued" => Self::Queued,
            "promoted" => Self::Promoted,
            _ => return None,
        })
    }
}

impl DelegationGroupState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::ReadyForParent => "ready_for_parent",
            Self::Synthesizing => "synthesizing",
            Self::Complete => "complete",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "created" => Self::Created,
            "queued" => Self::Queued,
            "running" => Self::Running,
            "ready_for_parent" => Self::ReadyForParent,
            "synthesizing" => Self::Synthesizing,
            "complete" => Self::Complete,
            "degraded" => Self::Degraded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Created => matches!(next, Self::Queued | Self::Running | Self::Cancelled),
            Self::Queued => matches!(next, Self::Running | Self::Failed | Self::Cancelled),
            Self::Running => matches!(
                next,
                Self::ReadyForParent | Self::Degraded | Self::Failed | Self::Cancelled
            ),
            Self::ReadyForParent => matches!(
                next,
                Self::Synthesizing
                    | Self::Complete
                    | Self::Degraded
                    | Self::Failed
                    | Self::Cancelled
            ),
            Self::Synthesizing => matches!(
                next,
                Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled
            ),
            Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationTaskState {
    Created,
    Queued,
    Leased,
    Running,
    Retrying,
    Complete,
    Degraded,
    Failed,
    Cancelled,
}

impl DelegationTaskState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Retrying => "retrying",
            Self::Complete => "complete",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "created" => Self::Created,
            "queued" => Self::Queued,
            "leased" => Self::Leased,
            "running" => Self::Running,
            "retrying" => Self::Retrying,
            "complete" => Self::Complete,
            "degraded" => Self::Degraded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Created => matches!(next, Self::Queued | Self::Cancelled),
            Self::Queued => matches!(next, Self::Leased | Self::Failed | Self::Cancelled),
            Self::Leased => matches!(
                next,
                Self::Queued | Self::Running | Self::Failed | Self::Cancelled
            ),
            Self::Running => matches!(
                next,
                Self::Retrying | Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled
            ),
            Self::Retrying => matches!(next, Self::Queued | Self::Failed | Self::Cancelled),
            Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationTaskSpec {
    pub delegation_task_id: String,
    pub task_key: String,
    pub objective: String,
    pub role: DelegatedRunRole,
    #[serde(default)]
    pub target_scope: Vec<DelegatedRunScope>,
    pub max_attempts: usize,
    /// Task keys that must produce usable terminal results before this task is
    /// eligible for admission. Keys, rather than generated durable IDs, keep
    /// the graph portable across foreground, detached, and replayed runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Parent-declared paths or path prefixes this task expects to modify.
    /// This is a planning and diagnostics contract only: isolated workspaces
    /// and integration-time patches remain the authoritative safety boundary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_intent: Vec<String>,
    /// Exact task-level policy. Legacy records inherit the group policy, while
    /// structured groups may persist a narrower capability subset per task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_policy: Option<DelegationPolicy>,
    #[serde(default)]
    pub writer_mode: DelegationWriterMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_baseline: Option<String>,
    /// Recovery-only metadata lives in dedicated task columns and is skipped
    /// from ordinary task/session projections.
    #[serde(skip)]
    pub executor_envelope: Option<DelegationExecutorEnvelopeV1>,
}

impl DelegationTaskSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.delegation_task_id.trim().is_empty(),
            "delegation task id is required"
        );
        ensure!(
            !self.task_key.trim().is_empty(),
            "delegation task key is required"
        );
        ensure!(
            !self.objective.trim().is_empty(),
            "delegation task objective is required"
        );
        ensure!(
            self.objective.len() <= MAX_DELEGATION_TASK_OBJECTIVE_BYTES,
            "delegation task objective exceeds the durable size limit"
        );
        ensure!(
            self.target_scope.len() <= 64,
            "delegation task target scope exceeds the item limit"
        );
        ensure!(
            self.target_scope
                .iter()
                .all(|scope| scope.path.len() <= 4 * 1024 && scope.label.len() <= 512),
            "delegation task target scope exceeds the durable size limit"
        );
        ensure!(
            self.max_attempts > 0,
            "delegation task max attempts must be greater than zero"
        );
        ensure!(
            self.depends_on.len() <= 128,
            "delegation task dependency list exceeds the item limit"
        );
        ensure!(
            self.depends_on
                .iter()
                .all(|dependency| !dependency.trim().is_empty() && dependency.len() <= 512),
            "delegation task dependency key is empty or too long"
        );
        ensure!(
            self.write_intent.len() <= 256,
            "delegation task write intent exceeds the item limit"
        );
        ensure!(
            self.write_intent
                .iter()
                .all(|path| !path.trim().is_empty() && path.len() <= 4 * 1024),
            "delegation task write intent path is empty or too long"
        );
        if let Some(envelope) = self.executor_envelope.as_ref() {
            envelope.validate(&self.objective)?;
            ensure!(
                envelope.task_id == self.delegation_task_id,
                "executor task identity differs from its immutable task"
            );
            ensure!(
                envelope.role == self.role,
                "executor role differs from its immutable task"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DelegationGroupStartInput {
    pub delegation_group_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub contract: DelegationGroupContract,
    pub tasks: Vec<DelegationTaskSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationTaskRecord {
    pub delegation_group_id: String,
    pub ordinal: usize,
    pub specification: DelegationTaskSpec,
    pub state: DelegationTaskState,
    pub attempt_count: usize,
    pub result: Option<Value>,
    pub error_summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationTaskLease {
    pub task: DelegationTaskRecord,
    pub lease_owner_id: String,
    pub lease_expires_at_ms: i64,
}

/// Database-backed capacity policy. The first process to initialize an
/// authority persists these values; later processes sharing the database use
/// that durable policy instead of introducing process-local ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationCapacityPolicy {
    pub initial_limit: usize,
    pub minimum_limit: usize,
    pub maximum_limit: usize,
    pub ramp_step: usize,
    pub healthy_completions_before_ramp: usize,
    pub default_cooldown_ms: i64,
}

impl Default for DelegationCapacityPolicy {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(4);
        let initial_limit = parallelism.saturating_mul(2).max(8);
        Self {
            initial_limit,
            minimum_limit: 1,
            maximum_limit: initial_limit.saturating_mul(4).max(32),
            ramp_step: (parallelism / 2).max(1),
            healthy_completions_before_ramp: initial_limit,
            default_cooldown_ms: 2_000,
        }
    }
}

impl DelegationCapacityPolicy {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.initial_limit > 0,
            "capacity initial limit must be positive"
        );
        ensure!(
            self.minimum_limit > 0,
            "capacity minimum limit must be positive"
        );
        ensure!(
            self.initial_limit >= self.minimum_limit,
            "capacity initial limit cannot be below its minimum"
        );
        ensure!(
            self.maximum_limit >= self.initial_limit,
            "capacity maximum limit cannot be below its initial limit"
        );
        ensure!(self.ramp_step > 0, "capacity ramp step must be positive");
        ensure!(
            self.healthy_completions_before_ramp > 0,
            "capacity healthy threshold must be positive"
        );
        ensure!(
            self.default_cooldown_ms > 0,
            "capacity cooldown must be positive"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegationCapacityClass {
    ReadOnly,
    WriteShared,
    WriteIsolated,
    Verification,
}

impl DelegationCapacityClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WriteShared => "write_shared",
            Self::WriteIsolated => "write_isolated",
            Self::Verification => "verification",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationCapacityRequest {
    pub authority_key: String,
    pub domain_key: String,
    pub partition_key: String,
    pub scheduling_class: DelegationCapacityClass,
    pub isolation_group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationCapacityFeedback {
    Healthy,
    Neutral,
    Timeout,
    RateLimited { retry_after_ms: Option<i64> },
    ServiceUnavailable { retry_after_ms: Option<i64> },
    Overloaded { retry_after_ms: Option<i64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationSynthesisLease {
    pub group: DelegationGroupRecord,
    pub lease_owner_id: String,
    pub lease_expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationGroupRecord {
    pub delegation_group_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub contract: DelegationGroupContract,
    pub state: DelegationGroupState,
    pub parent_continuation_state: DelegationParentContinuationState,
    pub parent_continuation_id: Option<String>,
    pub synthesis_owner_id: Option<String>,
    pub synthesis_lease_expires_at_ms: Option<i64>,
    pub synthesis_attempt_count: usize,
    pub tasks: Vec<DelegationTaskRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationEventType {
    GroupCreated,
    GroupQueued,
    GroupStateChanged,
    TaskClaimed,
    TaskRunning,
    TaskStateChanged,
    ParentContinuationQueued,
    ParentContinuationPromoted,
    Other(String),
}

impl DelegationEventType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::GroupCreated => "group_created",
            Self::GroupQueued => "group_queued",
            Self::GroupStateChanged => "group_state_changed",
            Self::TaskClaimed => "task_claimed",
            Self::TaskRunning => "task_running",
            Self::TaskStateChanged => "task_state_changed",
            Self::ParentContinuationQueued => "parent_continuation_queued",
            Self::ParentContinuationPromoted => "parent_continuation_promoted",
            Self::Other(value) => value.as_str(),
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "group_created" => Self::GroupCreated,
            "group_queued" => Self::GroupQueued,
            "group_state_changed" => Self::GroupStateChanged,
            "task_claimed" => Self::TaskClaimed,
            "task_running" => Self::TaskRunning,
            "task_state_changed" => Self::TaskStateChanged,
            "parent_continuation_queued" => Self::ParentContinuationQueued,
            "parent_continuation_promoted" => Self::ParentContinuationPromoted,
            _ => Self::Other(value.to_owned()),
        }
    }
}

impl Serialize for DelegationEventType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DelegationEventType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::parse(&String::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegationEventRecord {
    /// Monotonic database cursor. Consumers replay `event_id > cursor`.
    pub event_id: i64,
    pub parent_session_id: String,
    pub delegation_group_id: String,
    pub delegation_task_id: Option<String>,
    pub event_type: DelegationEventType,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}
