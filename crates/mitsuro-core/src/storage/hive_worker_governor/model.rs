use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ai::models::ModelKey;
use crate::ai::types::Usage;
use crate::hive::{DstFoldPolicy, DstGapPolicy};
use crate::tools::registry::PermissionMode;

/// Product defaults for every existing and newly created Hive Worker.
///
/// These are deliberately finite. "Always on" means a sequence of bounded,
/// recoverable runs; it never means unlimited provider calls inside one day.
pub const DEFAULT_WORKER_DAILY_CALL_LIMIT: u64 = 128;
pub const DEFAULT_WORKER_DAILY_TOKEN_LIMIT: u64 = 1_000_000;
pub const DEFAULT_WORKER_IDLE_BASE_SECS: u64 = 900;
pub const DEFAULT_WORKER_IDLE_MAX_SECS: u64 = 21_600;
pub const DEFAULT_WORKER_GOVERNOR_TIMEZONE: &str = "UTC";
/// Recovery authority should be used immediately for the next direct message,
/// never stockpiled as durable permission for future autonomous work.
pub const WORKER_GOVERNOR_RECOVERY_GRANT_TTL_SECS: i64 = 5 * 60;

/// Validation ceilings prevent malformed clients from turning integer fields
/// into an accidental unbounded policy while still leaving ample edit range.
pub const MAX_WORKER_DAILY_CALL_LIMIT: u64 = 1_000_000;
pub const MAX_WORKER_DAILY_TOKEN_LIMIT: u64 = 1_000_000_000_000;
pub const MAX_WORKER_IDLE_SECS: u64 = 31_536_000;

pub(crate) const MAX_WORKER_GOVERNOR_ID_BYTES: usize = 256;
pub(crate) const MAX_WORKER_GOVERNOR_LANE_BYTES: usize = 512;
pub(crate) const MAX_WORKER_GOVERNOR_REASON_BYTES: usize = 2_048;
pub(crate) const MAX_WORKER_GOVERNOR_CURRENCY_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRunOrigin {
    UserDm,
    UserGroup,
    UserLifecycleAction,
    UserWorkflowActivation,
    ManualRunNow,
    Scheduled,
    Heartbeat,
    WorkerPeer,
    ScheduledGroup,
    WorkflowRollover,
    /// Storage-authored owner acceptance boundary. V1 performs no provider
    /// call, but the distinct origin prevents it inheriting execution rights
    /// from the source Workflow attempt.
    WorkflowAcceptance,
    LifecycleSweep,
    ControllerChild,
}

impl WorkerRunOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserDm => "user_dm",
            Self::UserGroup => "user_group",
            Self::UserLifecycleAction => "user_lifecycle_action",
            Self::UserWorkflowActivation => "user_workflow_activation",
            Self::ManualRunNow => "manual_run_now",
            Self::Scheduled => "scheduled",
            Self::Heartbeat => "heartbeat",
            Self::WorkerPeer => "worker_peer",
            Self::ScheduledGroup => "scheduled_group",
            Self::WorkflowRollover => "workflow_rollover",
            Self::WorkflowAcceptance => "workflow_acceptance",
            Self::LifecycleSweep => "lifecycle_sweep",
            Self::ControllerChild => "controller_child",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "user_dm" => Some(Self::UserDm),
            "user_group" => Some(Self::UserGroup),
            "user_lifecycle_action" => Some(Self::UserLifecycleAction),
            "user_workflow_activation" => Some(Self::UserWorkflowActivation),
            "manual_run_now" => Some(Self::ManualRunNow),
            "scheduled" => Some(Self::Scheduled),
            "heartbeat" => Some(Self::Heartbeat),
            "worker_peer" => Some(Self::WorkerPeer),
            "scheduled_group" => Some(Self::ScheduledGroup),
            "workflow_rollover" => Some(Self::WorkflowRollover),
            "workflow_acceptance" => Some(Self::WorkflowAcceptance),
            "lifecycle_sweep" => Some(Self::LifecycleSweep),
            "controller_child" => Some(Self::ControllerChild),
            _ => None,
        }
    }

    /// Child work must inherit its parent's effective origin before it reaches
    /// this store. A bare `ControllerChild` is therefore never admissible.
    pub fn is_autonomous(self) -> bool {
        matches!(
            self,
            Self::Scheduled
                | Self::Heartbeat
                | Self::WorkerPeer
                | Self::ScheduledGroup
                | Self::WorkflowRollover
                | Self::LifecycleSweep
        )
    }
}

impl std::fmt::Display for WorkerRunOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGovernorGateReason {
    PolicyUnavailable,
    UnresolvedProviderCall,
    DailyCallCapReached,
    DailyTokenCapReached,
    QuietHours,
    IdleBackoff,
}

impl WorkerGovernorGateReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PolicyUnavailable => "policy_unavailable",
            Self::UnresolvedProviderCall => "unresolved_provider_call",
            Self::DailyCallCapReached => "daily_call_cap_reached",
            Self::DailyTokenCapReached => "daily_token_cap_reached",
            Self::QuietHours => "quiet_hours",
            Self::IdleBackoff => "idle_backoff",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "policy_unavailable" => Some(Self::PolicyUnavailable),
            "unresolved_provider_call" => Some(Self::UnresolvedProviderCall),
            "daily_call_cap_reached" => Some(Self::DailyCallCapReached),
            "daily_token_cap_reached" => Some(Self::DailyTokenCapReached),
            "quiet_hours" => Some(Self::QuietHours),
            "idle_backoff" => Some(Self::IdleBackoff),
            _ => None,
        }
    }
}

impl std::fmt::Display for WorkerGovernorGateReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGovernorDisposition {
    Allow,
    Defer,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveWorkerGovernorPolicy {
    pub worker_id: String,
    pub revision: u64,
    pub daily_call_limit: u64,
    pub daily_token_limit: u64,
    pub timezone: String,
    pub quiet_start_minute: Option<u16>,
    pub quiet_end_minute: Option<u16>,
    pub quiet_gap_policy: DstGapPolicy,
    pub quiet_fold_policy: DstFoldPolicy,
    pub idle_base_secs: u64,
    pub idle_max_secs: u64,
    /// First instant at which provider spend is authoritative for this Worker.
    pub tracking_started_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveWorkerGovernorPolicyUpdate {
    pub daily_call_limit: u64,
    pub daily_token_limit: u64,
    pub timezone: String,
    pub quiet_start_minute: Option<u16>,
    pub quiet_end_minute: Option<u16>,
    pub quiet_gap_policy: DstGapPolicy,
    pub quiet_fold_policy: DstFoldPolicy,
    pub idle_base_secs: u64,
    pub idle_max_secs: u64,
}

impl Default for HiveWorkerGovernorPolicyUpdate {
    fn default() -> Self {
        Self {
            daily_call_limit: DEFAULT_WORKER_DAILY_CALL_LIMIT,
            daily_token_limit: DEFAULT_WORKER_DAILY_TOKEN_LIMIT,
            timezone: DEFAULT_WORKER_GOVERNOR_TIMEZONE.to_string(),
            quiet_start_minute: None,
            quiet_end_minute: None,
            quiet_gap_policy: DstGapPolicy::ShiftForward,
            quiet_fold_policy: DstFoldPolicy::First,
            idle_base_secs: DEFAULT_WORKER_IDLE_BASE_SECS,
            idle_max_secs: DEFAULT_WORKER_IDLE_MAX_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerGovernorPolicyCas {
    Updated(HiveWorkerGovernorPolicy),
    Conflict(HiveWorkerGovernorPolicy),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerConversationLane {
    DirectMessage,
    Group { group_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenModelPriceSnapshot {
    /// ISO-style currency code when monetary pricing is known.
    pub currency: Option<String>,
    /// Integer microunits per one million tokens. No floating point prices are
    /// authoritative in the durable ledger.
    pub input_microunits_per_million: Option<u64>,
    pub output_microunits_per_million: Option<u64>,
    pub cache_creation_microunits_per_million: Option<u64>,
    pub cache_read_microunits_per_million: Option<u64>,
    pub catalog_source: String,
    pub catalog_revision: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BeginWorkerProviderCall {
    pub provider_call_id: String,
    pub worker_id: String,
    /// Exact profile/model/document revision frozen by the claimed run.
    pub expected_worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub conversation_lane: WorkerConversationLane,
    pub run_id: String,
    pub run_lease_token: String,
    pub run_lease_epoch: u64,
    pub expected_model_key: ModelKey,
    pub expected_model_catalog_revision: Option<String>,
    pub expected_permission_mode: PermissionMode,
    /// Must already be inherited for child calls. `ControllerChild` itself is
    /// rejected because it does not state whether the root was foreground.
    pub origin: WorkerRunOrigin,
    pub lane_key: String,
    pub call_kind: String,
    pub workflow_goal_id: Option<String>,
    pub workflow_attempt_id: Option<String>,
    pub reserved_tokens: u64,
    pub pricing: Option<FrozenModelPriceSnapshot>,
    pub override_grant_id: Option<String>,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerProviderCall {
    pub provider_call_id: String,
    pub worker_id: String,
    pub worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub group_id: Option<String>,
    pub run_id: String,
    pub run_lease_token: String,
    pub run_lease_epoch: u64,
    pub run_lease_expires_at: String,
    pub workflow_goal_id: Option<String>,
    pub workflow_attempt_id: Option<String>,
    pub origin: WorkerRunOrigin,
    pub lane_key: String,
    pub call_kind: String,
    pub provider_id: String,
    pub model_id: String,
    pub model_key_json: String,
    pub model_key_fingerprint: String,
    pub model_catalog_revision: Option<String>,
    pub permission_mode: PermissionMode,
    pub pricing: Option<FrozenModelPriceSnapshot>,
    pub policy_revision: u64,
    pub timezone: String,
    pub local_day: String,
    pub reserved_tokens: u64,
    pub override_grant_id: Option<String>,
    pub started_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallTerminalState {
    Completed,
    Unknown,
}

impl ProviderCallTerminalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "completed" => Some(Self::Completed),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallRemoteAcceptance {
    NotSent,
    PossiblySent,
    Acknowledged,
}

impl ProviderCallRemoteAcceptance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotSent => "not_sent",
            Self::PossiblySent => "possibly_sent",
            Self::Acknowledged => "acknowledged",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "not_sent" => Some(Self::NotSent),
            "possibly_sent" => Some(Self::PossiblySent),
            "acknowledged" => Some(Self::Acknowledged),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FinishWorkerProviderCall {
    pub provider_call_id: String,
    pub worker_id: String,
    pub run_id: String,
    pub state: ProviderCallTerminalState,
    pub outcome: String,
    pub remote_acceptance: ProviderCallRemoteAcceptance,
    pub usage: Option<Usage>,
    pub estimated_cost_microunits: Option<u64>,
    pub unknown_reason: Option<String>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerProviderCallOutcome {
    pub provider_call_id: String,
    pub state: ProviderCallTerminalState,
    pub outcome: String,
    pub remote_acceptance: ProviderCallRemoteAcceptance,
    pub usage: Option<Usage>,
    pub usage_total_tokens: Option<u64>,
    pub estimated_cost_microunits: Option<u64>,
    pub unknown_reason: Option<String>,
    pub finished_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishWorkerProviderCallResult {
    Inserted(WorkerProviderCallOutcome),
    AlreadyRecorded(WorkerProviderCallOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorDailyUsage {
    pub local_day: String,
    pub timezone: String,
    pub starts_at: String,
    pub resets_at: String,
    pub calls_used: u64,
    pub calls_limit: u64,
    /// Completed calls contribute reported logical usage; Started, Unknown,
    /// and usage-less calls contribute their reservation.
    pub tokens_used_or_reserved: u64,
    pub tokens_limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorIdleProjection {
    pub lane_key: String,
    pub idle_streak: u32,
    pub not_before: Option<String>,
    pub last_material_at: Option<String>,
    pub last_outcome_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorDecision {
    pub disposition: WorkerGovernorDisposition,
    pub primary_reason: Option<WorkerGovernorGateReason>,
    pub reasons: Vec<WorkerGovernorGateReason>,
    pub evaluated_at: String,
    pub next_eligible_at: Option<String>,
    pub policy_revision: u64,
    pub tracking_started_at: String,
    pub daily: WorkerGovernorDailyUsage,
    pub idle: WorkerGovernorIdleProjection,
    pub override_grant_id: Option<String>,
}

/// Read-only admission projection for one canonical Worker conversation lane.
///
/// The reservation is explicit because the token-cap answer depends on the
/// size of the next request. Product read surfaces use a one-token minimum
/// probe; the real executor always re-evaluates with its exact reservation
/// before crossing the provider boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorLaneDecisionProjection {
    pub origin: WorkerRunOrigin,
    pub lane_key: String,
    pub reservation_tokens: u64,
    pub decision: WorkerGovernorDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorCurrencyCost {
    /// Frozen currency identifier copied from the provider-call price snapshot.
    pub currency: String,
    /// Decimal string preserves the full SQLite integer range across JSON/JS.
    pub estimated_cost_microunits: String,
    pub priced_call_count: u64,
}

/// Current local-day cost projection. Calls without both a frozen currency and
/// a committed estimated cost are counted, never silently treated as free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorDailyCostProjection {
    pub local_day: String,
    pub timezone: String,
    pub starts_at: String,
    pub resets_at: String,
    pub by_currency: Vec<WorkerGovernorCurrencyCost>,
    pub unpriced_call_count: u64,
}

/// Exact-owner, exact-DM read model for the Worker governor. It intentionally
/// exposes aggregate accounting only: provider prompts, outputs, model wire
/// payloads, and request content never enter this projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveWorkerGovernorProjection {
    pub schema_version: u32,
    pub worker_id: String,
    pub worker_revision: u64,
    pub dm_session_id: String,
    pub evaluated_at: String,
    pub policy: HiveWorkerGovernorPolicy,
    pub daily: WorkerGovernorDailyUsage,
    pub autonomous_dm: WorkerGovernorLaneDecisionProjection,
    pub foreground_dm: WorkerGovernorLaneDecisionProjection,
    /// Immutable Started ledger rows whose effect is no longer safely known.
    pub unresolved_started_count: u64,
    /// One exact ordinary DM completed remotely but lost its canonical
    /// response before commit and is awaiting explicit owner settlement.
    pub response_loss_recovery_required: bool,
    pub estimated_daily_cost: WorkerGovernorDailyCostProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginWorkerProviderCallResult {
    Started(WorkerProviderCall),
    /// The durable Started row already existed. Callers must not cross the
    /// provider boundary again; this is an accounting replay, not permission
    /// to replay an uncertain remote request.
    AlreadyStarted(WorkerProviderCall),
    Gated(WorkerGovernorDecision),
}

#[derive(Debug, Clone)]
pub struct GrantWorkerGovernorOverride {
    pub id: String,
    pub operation_id: String,
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub bypass_unresolved_provider_call: bool,
    pub bypass_daily_call_cap: bool,
    pub bypass_daily_token_cap: bool,
    pub bypass_quiet_hours: bool,
    pub bypass_idle_backoff: bool,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGovernorOverrideGrant {
    pub id: String,
    pub operation_id: String,
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub bypass_unresolved_provider_call: bool,
    pub bypass_daily_call_cap: bool,
    pub bypass_daily_token_cap: bool,
    pub bypass_quiet_hours: bool,
    pub bypass_idle_backoff: bool,
    pub reason: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct ReconcileUnknownProviderCall {
    pub provider_call_id: String,
    pub worker_id: String,
    pub run_id: String,
    pub daemon_lease_name: String,
    pub daemon_owner_id: String,
    pub daemon_fencing_token: u64,
    pub reason: String,
    pub reconciled_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RecordWorkerIdleOutcome {
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub run_id: String,
    pub lane_key: String,
    pub origin: WorkerRunOrigin,
    pub material: bool,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerIdleOutcome {
    Updated(WorkerGovernorIdleProjection),
    AlreadyRecorded(WorkerGovernorIdleProjection),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRunGovernorProjection {
    pub run_id: String,
    pub origin: Option<WorkerRunOrigin>,
    pub lane_key: Option<String>,
    pub gate_reason: Option<WorkerGovernorGateReason>,
    pub next_eligible_at: Option<String>,
    pub policy_revision: Option<u64>,
    pub override_grant_id: Option<String>,
}
