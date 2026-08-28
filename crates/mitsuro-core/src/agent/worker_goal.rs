//! Trusted outcome boundary for one fenced Hive Worker Goal run.
//!
//! Model prose and tool arguments are not durable Workflow authority. The
//! orchestrator builds this contract from the already validated Worker Goal
//! binding and runtime-observed provider/tool results, then hands it to the
//! Hive host's atomic outcome committer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::storage::WorkerRunOrigin;

const MAX_ID_BYTES: usize = 256;
const MAX_WORKSPACE_PATH_BYTES: usize = 16 * 1024;
pub const MAX_WORKER_GOAL_PROVIDER_CALL_IDS: usize = 256;
pub const MAX_WORKER_GOAL_EVIDENCE_ITEMS: usize = 32;
pub const MAX_WORKER_GOAL_EVIDENCE_SUMMARY_BYTES: usize = 2 * 1024;
pub const MAX_WORKER_GOAL_EFFECT_SUMMARY_BYTES: usize = 8 * 1024;

/// Runtime conclusion for the claimed attempt. `Progressed` deliberately does
/// not assert that the step or Goal is complete. `Succeeded` is reserved for a
/// separate typed acceptance authority; the generic workspace-tool loop must
/// never derive it from model prose or lexical command classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAttemptOutcome {
    Succeeded,
    Progressed,
    Blocked,
    Failed,
    Cancelled,
    BudgetExhausted,
    NeedsAttention,
}

/// Source class for one bounded piece of runtime-observed evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalEvidenceKind {
    WorkspaceObservation,
    WorkspaceMutation,
    Verification,
    ToolFailure,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalEvidence {
    kind: WorkerGoalEvidenceKind,
    summary: String,
}

impl WorkerGoalEvidence {
    pub fn new(
        kind: WorkerGoalEvidenceKind,
        summary: impl Into<String>,
    ) -> Result<Self, WorkerGoalOutcomeInputError> {
        let summary = summary.into();
        validate_bounded_summary(
            "evidence summary",
            &summary,
            MAX_WORKER_GOAL_EVIDENCE_SUMMARY_BYTES,
        )?;
        Ok(Self { kind, summary })
    }

    pub const fn kind(&self) -> WorkerGoalEvidenceKind {
        self.kind
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    fn validate(&self) -> Result<(), WorkerGoalOutcomeInputError> {
        validate_bounded_summary(
            "evidence summary",
            &self.summary,
            MAX_WORKER_GOAL_EVIDENCE_SUMMARY_BYTES,
        )
    }
}

/// Bounded user-neutral description of observed workspace effects. This is
/// evidence context, not a place for Workflow identifiers or status commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalEffectSummary {
    summary: String,
    workspace_mutated: bool,
}

impl WorkerGoalEffectSummary {
    pub fn new(
        summary: impl Into<String>,
        workspace_mutated: bool,
    ) -> Result<Self, WorkerGoalOutcomeInputError> {
        let summary = summary.into();
        validate_bounded_optional_text(
            "effect summary",
            &summary,
            MAX_WORKER_GOAL_EFFECT_SUMMARY_BYTES,
        )?;
        Ok(Self {
            summary,
            workspace_mutated,
        })
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub const fn workspace_mutated(&self) -> bool {
        self.workspace_mutated
    }

    fn validate(&self) -> Result<(), WorkerGoalOutcomeInputError> {
        validate_bounded_optional_text(
            "effect summary",
            &self.summary,
            MAX_WORKER_GOAL_EFFECT_SUMMARY_BYTES,
        )
    }
}

/// Runtime counters observed by core. The model cannot supply these values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalOutcomeCounters {
    /// AgentTurn provider calls observed by the orchestrator. Auxiliary
    /// classifier calls remain independently governed in the provider ledger.
    pub provider_calls: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub successful_tool_calls: u32,
    pub failed_tool_calls: u32,
    pub research_actions: u32,
}

impl WorkerGoalOutcomeCounters {
    fn validate(self) -> Result<(), WorkerGoalOutcomeInputError> {
        if self
            .successful_tool_calls
            .saturating_add(self.failed_tool_calls)
            != self.tool_calls
            || self.research_actions > self.tool_calls
        {
            return Err(WorkerGoalOutcomeInputError::InvalidCounters);
        }
        Ok(())
    }
}

/// Exact fenced authority plus bounded outcome material for one commit.
///
/// Identity fields are private and have no public constructor. Core creates
/// them only from a validated `WorkerGoalExecutionContext` and provider permits
/// admitted during that run; a model cannot choose a Worker, Goal, step, run,
/// lease, or provider-call id through this API.
#[derive(Clone, PartialEq, Eq)]
pub struct WorkerGoalOutcomeCommitInput {
    worker_id: String,
    worker_revision: u64,
    owner_user_id: Option<String>,
    session_id: String,
    run_id: String,
    run_lease_token: String,
    run_lease_epoch: u64,
    run_origin: WorkerRunOrigin,
    goal_id: String,
    goal_revision: u64,
    workflow_aggregate_revision: u64,
    attempt_id: String,
    plan_revision_id: String,
    plan_revision_number: u64,
    step_id: String,
    step_revision: u64,
    workspace_dir: PathBuf,
    /// Ordered AgentTurn call identities; the last item is the held final
    /// no-tool response. Auxiliary governed calls are resolved from the same
    /// run ledger by the atomic committer.
    provider_call_ids: Vec<String>,
    outcome: WorkerGoalAttemptOutcome,
    evidence: Vec<WorkerGoalEvidence>,
    effect: WorkerGoalEffectSummary,
    counters: WorkerGoalOutcomeCounters,
}

impl WorkerGoalOutcomeCommitInput {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_validated_run(
        worker_id: String,
        worker_revision: u64,
        owner_user_id: Option<String>,
        session_id: String,
        run_id: String,
        run_lease_token: String,
        run_lease_epoch: u64,
        run_origin: WorkerRunOrigin,
        goal_id: String,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        attempt_id: String,
        plan_revision_id: String,
        plan_revision_number: u64,
        step_id: String,
        step_revision: u64,
        workspace_dir: PathBuf,
        provider_call_ids: Vec<String>,
        outcome: WorkerGoalAttemptOutcome,
        evidence: Vec<WorkerGoalEvidence>,
        effect: WorkerGoalEffectSummary,
        counters: WorkerGoalOutcomeCounters,
    ) -> Result<Self, WorkerGoalOutcomeInputError> {
        for (label, value) in [
            ("Worker id", worker_id.as_str()),
            ("session id", session_id.as_str()),
            ("run id", run_id.as_str()),
            ("run lease token", run_lease_token.as_str()),
            ("Goal id", goal_id.as_str()),
            ("attempt id", attempt_id.as_str()),
            ("plan revision id", plan_revision_id.as_str()),
            ("step id", step_id.as_str()),
        ] {
            validate_bounded_text(label, value, MAX_ID_BYTES)?;
        }
        if let Some(owner_user_id) = owner_user_id.as_deref() {
            validate_bounded_text("owner user id", owner_user_id, MAX_ID_BYTES)?;
        }
        if worker_revision == 0
            || run_lease_epoch == 0
            || goal_revision == 0
            || workflow_aggregate_revision == 0
            || plan_revision_number == 0
            || step_revision == 0
        {
            return Err(WorkerGoalOutcomeInputError::InvalidRevision);
        }
        let workspace_text = workspace_dir.to_string_lossy();
        if !workspace_dir.is_absolute()
            || workspace_text.len() > MAX_WORKSPACE_PATH_BYTES
            || workspace_text.chars().any(|character| character == '\0')
        {
            return Err(WorkerGoalOutcomeInputError::InvalidWorkspace);
        }
        if provider_call_ids.is_empty() {
            return Err(WorkerGoalOutcomeInputError::MissingProviderCall);
        }
        if provider_call_ids.len() > MAX_WORKER_GOAL_PROVIDER_CALL_IDS {
            return Err(WorkerGoalOutcomeInputError::TooManyProviderCalls);
        }
        let mut unique_provider_calls = HashSet::with_capacity(provider_call_ids.len());
        for provider_call_id in &provider_call_ids {
            validate_bounded_text("provider call id", provider_call_id, MAX_ID_BYTES)?;
            if !unique_provider_calls.insert(provider_call_id.as_str()) {
                return Err(WorkerGoalOutcomeInputError::DuplicateProviderCall);
            }
        }
        if evidence.len() > MAX_WORKER_GOAL_EVIDENCE_ITEMS {
            return Err(WorkerGoalOutcomeInputError::TooManyEvidenceItems);
        }
        for evidence_item in &evidence {
            evidence_item.validate()?;
        }
        effect.validate()?;
        counters.validate()?;
        if usize::try_from(counters.provider_calls).ok() != Some(provider_call_ids.len())
            || counters.turns != counters.provider_calls
        {
            return Err(WorkerGoalOutcomeInputError::InvalidCounters);
        }

        Ok(Self {
            worker_id,
            worker_revision,
            owner_user_id,
            session_id,
            run_id,
            run_lease_token,
            run_lease_epoch,
            run_origin,
            goal_id,
            goal_revision,
            workflow_aggregate_revision,
            attempt_id,
            plan_revision_id,
            plan_revision_number,
            step_id,
            step_revision,
            workspace_dir,
            provider_call_ids,
            outcome,
            evidence,
            effect,
            counters,
        })
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }
    pub const fn worker_revision(&self) -> u64 {
        self.worker_revision
    }
    pub fn owner_user_id(&self) -> Option<&str> {
        self.owner_user_id.as_deref()
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn run_lease_token(&self) -> &str {
        &self.run_lease_token
    }
    pub const fn run_lease_epoch(&self) -> u64 {
        self.run_lease_epoch
    }
    pub const fn run_origin(&self) -> WorkerRunOrigin {
        self.run_origin
    }
    pub fn goal_id(&self) -> &str {
        &self.goal_id
    }
    pub const fn goal_revision(&self) -> u64 {
        self.goal_revision
    }
    pub const fn workflow_aggregate_revision(&self) -> u64 {
        self.workflow_aggregate_revision
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn plan_revision_id(&self) -> &str {
        &self.plan_revision_id
    }
    pub const fn plan_revision_number(&self) -> u64 {
        self.plan_revision_number
    }
    pub fn step_id(&self) -> &str {
        &self.step_id
    }
    pub const fn step_revision(&self) -> u64 {
        self.step_revision
    }
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }
    pub fn provider_call_ids(&self) -> &[String] {
        &self.provider_call_ids
    }
    pub fn final_provider_call_id(&self) -> &str {
        self.provider_call_ids
            .last()
            .expect("validated Worker Goal outcome always has a provider call")
    }
    pub const fn outcome(&self) -> WorkerGoalAttemptOutcome {
        self.outcome
    }
    pub fn evidence(&self) -> &[WorkerGoalEvidence] {
        &self.evidence
    }
    pub fn effect(&self) -> &WorkerGoalEffectSummary {
        &self.effect
    }
    pub const fn counters(&self) -> WorkerGoalOutcomeCounters {
        self.counters
    }
}

impl std::fmt::Debug for WorkerGoalOutcomeCommitInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerGoalOutcomeCommitInput")
            .field("worker_id", &self.worker_id)
            .field("worker_revision", &self.worker_revision)
            .field("owner_user_id", &self.owner_user_id)
            .field("session_id", &self.session_id)
            .field("run_id", &self.run_id)
            .field("run_lease_token", &"[REDACTED]")
            .field("run_lease_epoch", &self.run_lease_epoch)
            .field("run_origin", &self.run_origin)
            .field("goal_id", &self.goal_id)
            .field("goal_revision", &self.goal_revision)
            .field(
                "workflow_aggregate_revision",
                &self.workflow_aggregate_revision,
            )
            .field("attempt_id", &self.attempt_id)
            .field("plan_revision_id", &self.plan_revision_id)
            .field("plan_revision_number", &self.plan_revision_number)
            .field("step_id", &self.step_id)
            .field("step_revision", &self.step_revision)
            .field("workspace_dir", &self.workspace_dir)
            .field("provider_call_ids", &self.provider_call_ids)
            .field("outcome", &self.outcome)
            .field("evidence", &self.evidence)
            .field("effect", &self.effect)
            .field("counters", &self.counters)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalOutcomeCommitDisposition {
    Inserted,
    AdoptedExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalOutcomeCommit {
    pub disposition: WorkerGoalOutcomeCommitDisposition,
}

#[derive(Debug, Error)]
pub enum WorkerGoalOutcomeCommitError {
    #[error("Worker Goal outcome was rejected by a proven stale fence: {0}")]
    StaleRejected(String),
    #[error("Worker Goal outcome conflicts with durable state or durable state is corrupt: {0}")]
    ConflictOrCorrupt(String),
    #[error("Worker Goal outcome commit may have happened but cannot be proven: {0}")]
    CommitUncertain(String),
}

impl WorkerGoalOutcomeCommitError {
    /// Only this class proves that the caller no longer owns the exact run.
    /// Conflict and uncertainty must remain available for fenced recovery.
    pub const fn is_proven_stale(&self) -> bool {
        matches!(self, Self::StaleRejected(_))
    }
}

/// Trusted atomic persistence capability supplied by the Hive execution host.
pub trait WorkerGoalOutcomeCommitter: Send + Sync {
    fn commit_outcome(
        &self,
        input: &WorkerGoalOutcomeCommitInput,
    ) -> Result<WorkerGoalOutcomeCommit, WorkerGoalOutcomeCommitError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkerGoalOutcomeInputError {
    #[error("{0} is empty, malformed, or exceeds its byte limit")]
    InvalidText(&'static str),
    #[error("Worker Goal outcome contains a zero revision or lease epoch")]
    InvalidRevision,
    #[error("Worker Goal outcome workspace is not absolute")]
    InvalidWorkspace,
    #[error("Worker Goal outcome contains too many provider call identities")]
    TooManyProviderCalls,
    #[error("Worker Goal outcome has no final provider call identity")]
    MissingProviderCall,
    #[error("Worker Goal outcome repeats a provider call identity")]
    DuplicateProviderCall,
    #[error("Worker Goal outcome contains too many evidence items")]
    TooManyEvidenceItems,
    #[error("Worker Goal outcome counters are inconsistent")]
    InvalidCounters,
    #[error("Worker Goal outcome counters exceed the frozen attempt budget")]
    BudgetExceeded,
}

fn validate_bounded_text(
    label: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), WorkerGoalOutcomeInputError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
    {
        return Err(WorkerGoalOutcomeInputError::InvalidText(label));
    }
    Ok(())
}

fn validate_bounded_optional_text(
    label: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), WorkerGoalOutcomeInputError> {
    if value.len() > max_bytes || value.chars().any(|character| character == '\0') {
        return Err(WorkerGoalOutcomeInputError::InvalidText(label));
    }
    Ok(())
}

fn validate_bounded_summary(
    label: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), WorkerGoalOutcomeInputError> {
    if value.trim().is_empty()
        || value.len() > max_bytes
        || value.chars().any(|character| character == '\0')
    {
        return Err(WorkerGoalOutcomeInputError::InvalidText(label));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_and_effect_text_are_bounded() {
        assert!(WorkerGoalEvidence::new(
            WorkerGoalEvidenceKind::Verification,
            "focused test passed"
        )
        .is_ok());
        assert!(matches!(
            WorkerGoalEvidence::new(
                WorkerGoalEvidenceKind::Runtime,
                "x".repeat(MAX_WORKER_GOAL_EVIDENCE_SUMMARY_BYTES + 1)
            ),
            Err(WorkerGoalOutcomeInputError::InvalidText("evidence summary"))
        ));
        assert!(WorkerGoalEffectSummary::new(
            "x".repeat(MAX_WORKER_GOAL_EFFECT_SUMMARY_BYTES + 1),
            false
        )
        .is_err());
    }

    #[test]
    fn only_stale_errors_are_proven_terminal_fences() {
        assert!(WorkerGoalOutcomeCommitError::StaleRejected("stale".into()).is_proven_stale());
        assert!(
            !WorkerGoalOutcomeCommitError::ConflictOrCorrupt("conflict".into()).is_proven_stale()
        );
        assert!(
            !WorkerGoalOutcomeCommitError::CommitUncertain("uncertain".into()).is_proven_stale()
        );
    }
}
