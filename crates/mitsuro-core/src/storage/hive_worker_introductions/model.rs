use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::models::ModelKey;
use crate::ai::providers::ProviderId;
use crate::ai::types::Usage;

pub const WORKER_INTRODUCTION_PROPOSAL_VERSION: u32 = 1;
pub const MAX_WORKER_INTRODUCTION_FACTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveWorkerIntroductionStatus {
    Queued,
    Running,
    AwaitingContext,
    ReviewReady,
    Confirmed,
    Skipped,
    Failed,
    NeedsRecovery,
}

impl HiveWorkerIntroductionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::AwaitingContext => "awaiting_context",
            Self::ReviewReady => "review_ready",
            Self::Confirmed => "confirmed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::NeedsRecovery => "needs_recovery",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "awaiting_context" => Some(Self::AwaitingContext),
            "review_ready" => Some(Self::ReviewReady),
            "confirmed" => Some(Self::Confirmed),
            "skipped" => Some(Self::Skipped),
            "failed" => Some(Self::Failed),
            "needs_recovery" => Some(Self::NeedsRecovery),
            _ => None,
        }
    }

    pub fn allows_autonomy(self) -> bool {
        matches!(self, Self::Confirmed | Self::Skipped)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Skipped | Self::Failed)
    }
}

impl std::fmt::Display for HiveWorkerIntroductionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveWorkerIntroduction {
    pub worker_id: String,
    pub run_id: Option<String>,
    pub status: HiveWorkerIntroductionStatus,
    pub prompt_version: u32,
    pub opening_message_id: Option<i64>,
    pub proposal: Option<Value>,
    pub proposal_revision: u32,
    pub decision: Option<Value>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

/// The only fact categories that the restricted Introduction reviewer may
/// propose. The target is deliberately encoded in this enum: no model output
/// can select an arbitrary document, memory namespace, permission, or tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionFactKind {
    Role,
    Purpose,
    Responsibility,
    WorkingStyle,
    Boundary,
    ToolExpectation,
    MemoryExpectation,
    Cadence,
    UserPreference,
    UserCorrection,
    RelationshipContext,
}

impl WorkerIntroductionFactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::Purpose => "purpose",
            Self::Responsibility => "responsibility",
            Self::WorkingStyle => "working_style",
            Self::Boundary => "boundary",
            Self::ToolExpectation => "tool_expectation",
            Self::MemoryExpectation => "memory_expectation",
            Self::Cadence => "cadence",
            Self::UserPreference => "user_preference",
            Self::UserCorrection => "user_correction",
            Self::RelationshipContext => "relationship_context",
        }
    }

    pub fn managed_identity(self) -> bool {
        matches!(self, Self::Role | Self::Purpose | Self::Responsibility)
    }

    pub fn managed_soul(self) -> bool {
        matches!(
            self,
            Self::WorkingStyle
                | Self::Boundary
                | Self::ToolExpectation
                | Self::MemoryExpectation
                | Self::Cadence
        )
    }

    pub fn worker_private_memory(self) -> bool {
        matches!(
            self,
            Self::UserPreference | Self::UserCorrection | Self::RelationshipContext
        )
    }
}

/// The complete evidence-backed setup contract for one Worker Introduction.
///
/// This is trusted projection state, not provider output. Coverage is derived
/// only from the closed fact-kind enum after each fact's exact USER evidence
/// has passed the canonical transcript fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerIntroductionEvidenceAxis {
    Identity,
    Purpose,
    WorkingStyle,
    Boundary,
    Tools,
    Memory,
    Cadence,
}

impl WorkerIntroductionEvidenceAxis {
    pub const ALL: [Self; 7] = [
        Self::Identity,
        Self::Purpose,
        Self::WorkingStyle,
        Self::Boundary,
        Self::Tools,
        Self::Memory,
        Self::Cadence,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Purpose => "purpose",
            Self::WorkingStyle => "working_style",
            Self::Boundary => "boundary",
            Self::Tools => "tools",
            Self::Memory => "memory",
            Self::Cadence => "cadence",
        }
    }

    pub const fn from_fact_kind(kind: WorkerIntroductionFactKind) -> Option<Self> {
        match kind {
            WorkerIntroductionFactKind::Role => Some(Self::Identity),
            WorkerIntroductionFactKind::Purpose => Some(Self::Purpose),
            WorkerIntroductionFactKind::WorkingStyle => Some(Self::WorkingStyle),
            WorkerIntroductionFactKind::Boundary => Some(Self::Boundary),
            WorkerIntroductionFactKind::ToolExpectation => Some(Self::Tools),
            WorkerIntroductionFactKind::MemoryExpectation => Some(Self::Memory),
            WorkerIntroductionFactKind::Cadence => Some(Self::Cadence),
            WorkerIntroductionFactKind::Responsibility
            | WorkerIntroductionFactKind::UserPreference
            | WorkerIntroductionFactKind::UserCorrection
            | WorkerIntroductionFactKind::RelationshipContext => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerIntroductionEvidenceCoverage {
    pub covered: Vec<WorkerIntroductionEvidenceAxis>,
    pub missing: Vec<WorkerIntroductionEvidenceAxis>,
}

impl WorkerIntroductionEvidenceCoverage {
    pub fn from_fact_kinds(kinds: impl IntoIterator<Item = WorkerIntroductionFactKind>) -> Self {
        let covered_set = kinds
            .into_iter()
            .filter_map(WorkerIntroductionEvidenceAxis::from_fact_kind)
            .collect::<std::collections::HashSet<_>>();
        let covered = WorkerIntroductionEvidenceAxis::ALL
            .into_iter()
            .filter(|axis| covered_set.contains(axis))
            .collect::<Vec<_>>();
        let missing = WorkerIntroductionEvidenceAxis::ALL
            .into_iter()
            .filter(|axis| !covered_set.contains(axis))
            .collect::<Vec<_>>();
        Self { covered, missing }
    }

    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

impl WorkerIntroductionReviewerOutputV1 {
    pub fn evidence_coverage(&self) -> WorkerIntroductionEvidenceCoverage {
        WorkerIntroductionEvidenceCoverage::from_fact_kinds(self.facts.iter().map(|fact| fact.kind))
    }
}

impl std::fmt::Display for WorkerIntroductionFactKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionReviewReadiness {
    GatherMore,
    ReviewReady,
}

/// Provider-authored output. IDs, revision, binding, hashes, and proposal
/// scope are intentionally absent and are supplied only by trusted code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionReviewerFactV1 {
    pub kind: WorkerIntroductionFactKind,
    pub statement: String,
    pub evidence_message_id: i64,
    pub evidence_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionReviewerOutputV1 {
    pub readiness: WorkerIntroductionReviewReadiness,
    #[serde(default)]
    pub facts: Vec<WorkerIntroductionReviewerFactV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionProposalBasisV1 {
    pub opening_message_id: i64,
    pub through_message_id: i64,
    pub user_message_ids: Vec<i64>,
    pub transcript_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionProposalFactV1 {
    pub fact_id: String,
    pub kind: WorkerIntroductionFactKind,
    pub statement: String,
    pub evidence_message_id: i64,
    pub evidence_excerpt: String,
}

/// Fully authoritative proposal persisted to the lifecycle ledger and shown
/// by clients. Every field omitted from the provider output is generated or
/// reloaded at the storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionProposalV1 {
    pub schema_version: u32,
    pub proposal_id: String,
    pub revision: u32,
    pub worker_id: String,
    pub session_id: String,
    pub basis: WorkerIntroductionProposalBasisV1,
    pub base_identity_digest: String,
    pub base_soul_digest: String,
    pub facts: Vec<WorkerIntroductionProposalFactV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionDecisionKind {
    Confirmed,
    Rejected,
    KeepTalking,
}

impl WorkerIntroductionDecisionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::KeepTalking => "keep_talking",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionSelectedFactV1 {
    pub fact_id: String,
    pub final_statement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionDecisionV1 {
    pub schema_version: u32,
    pub proposal_id: String,
    pub proposal_revision: u32,
    pub worker_id: String,
    pub session_id: String,
    pub decision: WorkerIntroductionDecisionKind,
    pub selected_facts: Vec<WorkerIntroductionSelectedFactV1>,
    pub decided_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionReviewStatus {
    Queued,
    Claimed,
    GatherMore,
    ReviewReady,
    Confirmed,
    Rejected,
    KeepTalking,
    Failed,
    Stale,
}

impl WorkerIntroductionReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::GatherMore => "gather_more",
            Self::ReviewReady => "review_ready",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::KeepTalking => "keep_talking",
            Self::Failed => "failed",
            Self::Stale => "stale",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "claimed" => Some(Self::Claimed),
            "gather_more" => Some(Self::GatherMore),
            "review_ready" => Some(Self::ReviewReady),
            "confirmed" => Some(Self::Confirmed),
            "rejected" => Some(Self::Rejected),
            "keep_talking" => Some(Self::KeepTalking),
            "failed" => Some(Self::Failed),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerIntroductionReviewRecord {
    pub id: String,
    pub worker_id: String,
    pub session_id: String,
    pub status: WorkerIntroductionReviewStatus,
    pub claim_token: String,
    pub claim_expires_at: String,
    pub opening_message_id: i64,
    pub through_message_id: i64,
    pub user_message_ids: Vec<i64>,
    pub transcript_digest: String,
    pub base_identity_digest: String,
    pub base_soul_digest: String,
    pub worker_user_id: Option<String>,
    pub model: String,
    pub model_key: ModelKey,
    pub model_catalog_revision: Option<String>,
    pub provider_id: ProviderId,
    pub trace_run_id: String,
    pub provider_call_id: Option<String>,
    pub usage: Option<Usage>,
    pub proposal_id: Option<String>,
    pub proposal_revision: Option<u32>,
    pub reviewer_output: Option<WorkerIntroductionReviewerOutputV1>,
    pub proposal: Option<WorkerIntroductionProposalV1>,
    pub decision: Option<WorkerIntroductionDecisionV1>,
    pub last_error: Option<String>,
    pub claimed_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Exact Hive run that owns this review attempt. Legacy pre-77 audit rows
    /// remain nullable and cannot authorize a review-run completion.
    pub run_id: Option<String>,
    /// Monotonic attempt for one Worker/transcript basis.
    pub attempt_no: Option<u32>,
}

/// Read-only UI projection for the current canonical Introduction exchange.
/// `should_poll` is authored by core so clients do not independently derive
/// retry or terminal behavior from loosely related lifecycle fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionReviewProjectionState {
    Inactive,
    AwaitingContext,
    Pending,
    Claimed,
    Retrying,
    GatherMore,
    ReviewReady,
    NeedsAttention,
    Confirmed,
    Rejected,
    KeepTalking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerIntroductionReviewProjection {
    pub worker_id: String,
    pub lifecycle_status: HiveWorkerIntroductionStatus,
    pub state: WorkerIntroductionReviewProjectionState,
    pub current_through_message_id: Option<i64>,
    pub review_through_message_id: Option<i64>,
    pub review_status: Option<WorkerIntroductionReviewStatus>,
    pub is_current_through: bool,
    pub has_pending_user_input: bool,
    pub attempt_count: u32,
    pub should_poll: bool,
    pub last_error: Option<String>,
}
