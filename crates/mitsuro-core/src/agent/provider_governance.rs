//! Run-scoped provider-call governance for Hive Workers.
//!
//! The durable store owns policy evaluation and accounting. This module keeps
//! the remote boundary honest: a caller must obtain a permit before polling a
//! provider future, replays never cross the network, and an ambiguous dropped
//! permit deliberately leaves its append-only `Started` row for fenced
//! reconciliation by the Hive scheduler.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ai::models::{ModelCatalogSource, ModelKey, ResolvedModelRuntime};
use crate::ai::types::Usage;
use crate::storage::{
    hash_request_bytes, BeginWorkerProviderCall, BeginWorkerProviderCallResult, Database,
    FinishWorkerProviderCall, FinishWorkerProviderCallResult, FrozenModelPriceSnapshot,
    HiveWorkerGovernorStore, ProviderCallRemoteAcceptance, ProviderCallTerminalState,
    WorkerConversationLane, WorkerGovernorDecision, WorkerProviderCall, WorkerRunOrigin,
    MAX_WORKER_DAILY_TOKEN_LIMIT,
};
use crate::tools::registry::PermissionMode;

const MAX_BINDING_TEXT_BYTES: usize = 512;
const MAX_CHILD_SCOPE_BYTES: usize = 1_024;

/// Exact immutable authority captured from one claimed Worker run.
///
/// Callers must construct this from the claimed run and its frozen execution
/// context, never from UI input or mutable session defaults.
#[derive(Debug, Clone)]
pub struct WorkerProviderGovernorBinding {
    pub db_path: PathBuf,
    pub worker_id: String,
    pub worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub conversation_lane: WorkerConversationLane,
    pub run_id: String,
    pub run_lease_token: String,
    pub run_lease_epoch: u64,
    pub model_key: ModelKey,
    pub model_catalog_revision: Option<String>,
    pub permission_mode: PermissionMode,
    /// Child work must already carry its inherited foreground/autonomous root.
    pub origin: WorkerRunOrigin,
    pub workflow_goal_id: Option<String>,
    pub workflow_attempt_id: Option<String>,
    pub pricing: Option<FrozenModelPriceSnapshot>,
    /// Optional immutable one-call override selected by an explicit user act.
    pub override_grant_id: Option<String>,
}

impl WorkerProviderGovernorBinding {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.db_path.is_absolute(),
            "governor database path is not absolute"
        );
        validate_text("Worker id", &self.worker_id, MAX_BINDING_TEXT_BYTES)?;
        ensure!(
            self.worker_revision >= 1,
            "Worker revision must be at least one"
        );
        if let Some(owner_user_id) = self.owner_user_id.as_deref() {
            validate_text("owner user id", owner_user_id, MAX_BINDING_TEXT_BYTES)?;
        }
        validate_text("session id", &self.session_id, MAX_BINDING_TEXT_BYTES)?;
        let _ = self.conversation_lane.canonical_lane_key()?;
        validate_text("run id", &self.run_id, MAX_BINDING_TEXT_BYTES)?;
        validate_text(
            "run lease token",
            &self.run_lease_token,
            MAX_BINDING_TEXT_BYTES,
        )?;
        ensure!(
            self.run_lease_epoch <= i64::MAX as u64,
            "run lease epoch is out of range"
        );
        validate_text("model id", &self.model_key.model_id, MAX_BINDING_TEXT_BYTES)?;
        if let Some(revision) = self.model_catalog_revision.as_deref() {
            validate_text("model catalog revision", revision, MAX_BINDING_TEXT_BYTES)?;
        }
        ensure!(
            self.origin != WorkerRunOrigin::ControllerChild,
            "ControllerChild must inherit a concrete root origin"
        );
        if let Some(goal_id) = self.workflow_goal_id.as_deref() {
            validate_text("Workflow goal id", goal_id, MAX_BINDING_TEXT_BYTES)?;
        }
        if let Some(attempt_id) = self.workflow_attempt_id.as_deref() {
            validate_text("Workflow attempt id", attempt_id, MAX_BINDING_TEXT_BYTES)?;
        }
        if let Some(grant_id) = self.override_grant_id.as_deref() {
            validate_text("override grant id", grant_id, MAX_BINDING_TEXT_BYTES)?;
        }
        Ok(())
    }
}

/// Freeze the monetary rates from the exact immutable model runtime that will
/// cross the provider boundary.
///
/// The durable run owns `expected_model_key`, while the runtime contributes
/// the current exact row's catalog provenance and rates. Catalog revisions are
/// whole-catalog fingerprints, so an unrelated refresh must not fence a
/// persistent Worker whose full executable key is unchanged. Model catalog
/// prices are currently denominated in USD per one million tokens. Rates are
/// rounded upward to the nearest micro-USD so the projection never understates
/// a sub-micro rate.
pub fn freeze_worker_model_pricing(
    expected_model_key: &ModelKey,
    runtime: &ResolvedModelRuntime,
) -> Result<Option<FrozenModelPriceSnapshot>> {
    ensure!(
        &runtime.key == expected_model_key && runtime.wire_model_id == expected_model_key.model_id,
        "resolved Worker model runtime does not match the durable model key"
    );
    let input_microunits_per_million = runtime
        .input_price
        .map(|price| price_per_million_to_microunits("input", price))
        .transpose()?;
    let output_microunits_per_million = runtime
        .output_price
        .map(|price| price_per_million_to_microunits("output", price))
        .transpose()?;
    if input_microunits_per_million.is_none() && output_microunits_per_million.is_none() {
        return Ok(None);
    }

    Ok(Some(FrozenModelPriceSnapshot {
        currency: Some("USD".to_string()),
        input_microunits_per_million,
        output_microunits_per_million,
        // The shared catalog does not currently carry provider-specific cache
        // rates. A call that reports cache usage therefore remains truthfully
        // unpriced instead of inheriting an assumed discount.
        cache_creation_microunits_per_million: None,
        cache_read_microunits_per_million: None,
        catalog_source: model_catalog_source_name(runtime.catalog_source).to_string(),
        catalog_revision: runtime.catalog_revision.clone(),
    }))
}

fn model_catalog_source_name(source: ModelCatalogSource) -> &'static str {
    match source {
        ModelCatalogSource::Curated => "curated",
        ModelCatalogSource::CachedDynamic => "cached_dynamic",
        ModelCatalogSource::LiveDynamic => "live_dynamic",
        ModelCatalogSource::Custom => "custom",
        ModelCatalogSource::Legacy => "legacy",
    }
}

fn price_per_million_to_microunits(label: &str, price: f64) -> Result<u64> {
    const MICROUNITS_PER_UNIT: f64 = 1_000_000.0;
    ensure!(
        price.is_finite() && price >= 0.0,
        "Worker model {label} price is not a finite non-negative value"
    );
    let scaled = price * MICROUNITS_PER_UNIT;
    ensure!(
        scaled.is_finite() && scaled <= u64::MAX as f64,
        "Worker model {label} price is out of range"
    );
    let microunits = scaled.ceil() as u64;
    ensure!(
        microunits <= i64::MAX as u64,
        "Worker model {label} price is out of range"
    );
    Ok(microunits)
}

/// Stable semantic class for one provider call inside a Worker run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerProviderCallKind {
    AgentTurn,
    CompactionSummary,
    AutonomyClassifierFast,
    AutonomyClassifierEscalation,
    DelegatedAgentTurn,
    PostTurnLearningReview,
    WorkerIntroductionOpening,
    WorkerIntroductionOnboarding,
    WorkerIntroductionReview,
    /// Reserved semantic class for a future governed acceptance reviewer.
    /// Migration 78 deliberately rejects every provider call for V1
    /// `WorkerWorkflowAcceptance` runs.
    WorkerWorkflowAcceptance,
}

impl WorkerProviderCallKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentTurn => "agent_turn",
            Self::CompactionSummary => "compaction_summary",
            Self::AutonomyClassifierFast => "autonomy_classifier_fast",
            Self::AutonomyClassifierEscalation => "autonomy_classifier_escalation",
            Self::DelegatedAgentTurn => "delegated_agent_turn",
            Self::PostTurnLearningReview => "post_turn_learning_review",
            Self::WorkerIntroductionOpening => "worker_introduction_opening",
            Self::WorkerIntroductionOnboarding => "worker_introduction_onboarding",
            Self::WorkerIntroductionReview => "worker_introduction_review",
            Self::WorkerWorkflowAcceptance => "worker_workflow_acceptance",
        }
    }
}

/// Deterministic logical position of a provider call within one leased run.
///
/// `child_scope` is a stable durable/logical identifier such as a delegated
/// task id or tool-call id. It must never be a process-local counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerProviderCallSlot {
    pub kind: WorkerProviderCallKind,
    pub turn: u32,
    pub ordinal: u32,
    pub child_scope: Option<String>,
}

impl WorkerProviderCallSlot {
    pub const fn new(kind: WorkerProviderCallKind, turn: u32, ordinal: u32) -> Self {
        Self {
            kind,
            turn,
            ordinal,
            child_scope: None,
        }
    }

    pub fn child(
        kind: WorkerProviderCallKind,
        turn: u32,
        ordinal: u32,
        child_scope: impl Into<String>,
    ) -> Result<Self> {
        let child_scope = child_scope.into();
        validate_text(
            "provider-call child scope",
            &child_scope,
            MAX_CHILD_SCOPE_BYTES,
        )?;
        Ok(Self {
            kind,
            turn,
            ordinal,
            child_scope: Some(child_scope),
        })
    }

    fn validate(&self) -> Result<()> {
        if let Some(child_scope) = self.child_scope.as_deref() {
            validate_text(
                "provider-call child scope",
                child_scope,
                MAX_CHILD_SCOPE_BYTES,
            )?;
        }
        Ok(())
    }
}

/// Content-free terminal classification for an acknowledged or proven-unsent
/// call. Ambiguous calls have no value of this type: their permit is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerProviderTerminalOutcome {
    Completed,
    ProviderRejected,
    StreamError,
    StreamIdleTimeout,
    CancelledBeforeSend,
    CancelledAfterAcceptance,
    SemanticInvalid,
    UnsafeOutput,
    CanonicalCommitStale,
}

impl WorkerProviderTerminalOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::ProviderRejected => "provider_rejected",
            Self::StreamError => "stream_error",
            Self::StreamIdleTimeout => "stream_idle_timeout",
            Self::CancelledBeforeSend => "cancelled_before_send",
            Self::CancelledAfterAcceptance => "cancelled_after_acceptance",
            Self::SemanticInvalid => "semantic_invalid",
            Self::UnsafeOutput => "unsafe_output",
            Self::CanonicalCommitStale => "canonical_commit_stale",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerProviderCompletion {
    pub outcome: WorkerProviderTerminalOutcome,
    pub remote_acceptance: WorkerProviderCompletionAcceptance,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerProviderCompletionAcceptance {
    NotSent,
    Acknowledged,
}

impl WorkerProviderCompletion {
    pub fn acknowledged(outcome: WorkerProviderTerminalOutcome, usage: Option<Usage>) -> Self {
        Self {
            outcome,
            remote_acceptance: WorkerProviderCompletionAcceptance::Acknowledged,
            usage,
        }
    }

    pub fn not_sent(outcome: WorkerProviderTerminalOutcome) -> Result<Self> {
        ensure!(
            outcome == WorkerProviderTerminalOutcome::CancelledBeforeSend,
            "only a proven pre-send cancellation may be terminalized as not sent"
        );
        Ok(Self {
            outcome,
            remote_acceptance: WorkerProviderCompletionAcceptance::NotSent,
            usage: None,
        })
    }
}

#[derive(Debug)]
pub enum WorkerProviderAdmission {
    Allowed(WorkerProviderCallPermit),
    Gated(WorkerGovernorDecision),
    /// The call may already have crossed the remote boundary before a crash.
    /// This variant never grants a permit and must never be interpreted as a
    /// reason to resend.
    AlreadyStarted(WorkerProviderCall),
}

trait WorkerProviderLedger: Send + Sync {
    fn begin(&self, input: &BeginWorkerProviderCall) -> Result<BeginWorkerProviderCallResult>;
    fn finish(&self, input: &FinishWorkerProviderCall) -> Result<FinishWorkerProviderCallResult>;
}

struct SqliteWorkerProviderLedger {
    db_path: PathBuf,
}

impl WorkerProviderLedger for SqliteWorkerProviderLedger {
    fn begin(&self, input: &BeginWorkerProviderCall) -> Result<BeginWorkerProviderCallResult> {
        let db = Database::new(&self.db_path).context("opening Worker governor database")?;
        HiveWorkerGovernorStore::new(db).begin_provider_call(input)
    }

    fn finish(&self, input: &FinishWorkerProviderCall) -> Result<FinishWorkerProviderCallResult> {
        let db = Database::new(&self.db_path).context("opening Worker governor database")?;
        HiveWorkerGovernorStore::new(db).finish_provider_call(input)
    }
}

/// Cloneable per-run capability shared by the main loop and its auxiliary
/// provider callers. It contains no mutable process-local call counter.
#[derive(Clone)]
pub struct WorkerProviderCallGovernor {
    binding: Arc<WorkerProviderGovernorBinding>,
    ledger: Arc<dyn WorkerProviderLedger>,
}

impl std::fmt::Debug for WorkerProviderCallGovernor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerProviderCallGovernor")
            .field("worker_id", &self.binding.worker_id)
            .field("worker_revision", &self.binding.worker_revision)
            .field("session_id", &self.binding.session_id)
            .field("run_id", &self.binding.run_id)
            .field("run_lease_epoch", &self.binding.run_lease_epoch)
            .field("origin", &self.binding.origin)
            .finish_non_exhaustive()
    }
}

impl WorkerProviderCallGovernor {
    pub fn new(binding: WorkerProviderGovernorBinding) -> Result<Self> {
        binding.validate()?;
        let db_path = binding.db_path.clone();
        Ok(Self {
            binding: Arc::new(binding),
            ledger: Arc::new(SqliteWorkerProviderLedger { db_path }),
        })
    }

    pub fn binding(&self) -> &WorkerProviderGovernorBinding {
        &self.binding
    }

    pub fn provider_call_id(&self, slot: &WorkerProviderCallSlot) -> Result<String> {
        slot.validate()?;
        #[derive(Serialize)]
        struct DeterministicProviderCallId<'a> {
            version: u8,
            worker_id: &'a str,
            worker_revision: u64,
            run_id: &'a str,
            run_lease_token: &'a str,
            run_lease_epoch: u64,
            slot: &'a WorkerProviderCallSlot,
        }

        let material = serde_json::to_vec(&DeterministicProviderCallId {
            version: 1,
            worker_id: &self.binding.worker_id,
            worker_revision: self.binding.worker_revision,
            run_id: &self.binding.run_id,
            run_lease_token: &self.binding.run_lease_token,
            run_lease_epoch: self.binding.run_lease_epoch,
            slot,
        })?;
        Ok(format!("worker-call-{}", hash_request_bytes(material)))
    }

    pub fn admit(
        &self,
        slot: WorkerProviderCallSlot,
        reserved_tokens: u64,
    ) -> Result<WorkerProviderAdmission> {
        self.admit_at(slot, reserved_tokens, Utc::now())
    }

    pub(crate) fn admit_at(
        &self,
        slot: WorkerProviderCallSlot,
        reserved_tokens: u64,
        started_at: DateTime<Utc>,
    ) -> Result<WorkerProviderAdmission> {
        ensure!(
            (1..=MAX_WORKER_DAILY_TOKEN_LIMIT).contains(&reserved_tokens),
            "provider-call reservation is out of range"
        );
        let provider_call_id = self.provider_call_id(&slot)?;
        let lane_key = self.binding.conversation_lane.canonical_lane_key()?;
        let input = BeginWorkerProviderCall {
            provider_call_id,
            worker_id: self.binding.worker_id.clone(),
            expected_worker_revision: self.binding.worker_revision,
            owner_user_id: self.binding.owner_user_id.clone(),
            session_id: self.binding.session_id.clone(),
            conversation_lane: self.binding.conversation_lane.clone(),
            run_id: self.binding.run_id.clone(),
            run_lease_token: self.binding.run_lease_token.clone(),
            run_lease_epoch: self.binding.run_lease_epoch,
            expected_model_key: self.binding.model_key.clone(),
            expected_model_catalog_revision: self.binding.model_catalog_revision.clone(),
            expected_permission_mode: self.binding.permission_mode,
            origin: self.binding.origin,
            lane_key,
            call_kind: slot.kind.as_str().to_string(),
            workflow_goal_id: self.binding.workflow_goal_id.clone(),
            workflow_attempt_id: self.binding.workflow_attempt_id.clone(),
            reserved_tokens,
            pricing: self.binding.pricing.clone(),
            override_grant_id: self.binding.override_grant_id.clone(),
            started_at,
        };
        match self.ledger.begin(&input)? {
            BeginWorkerProviderCallResult::Started(call) => {
                Ok(WorkerProviderAdmission::Allowed(WorkerProviderCallPermit {
                    call,
                    ledger: Arc::clone(&self.ledger),
                }))
            }
            BeginWorkerProviderCallResult::AlreadyStarted(call) => {
                Ok(WorkerProviderAdmission::AlreadyStarted(call))
            }
            BeginWorkerProviderCallResult::Gated(decision) => {
                Ok(WorkerProviderAdmission::Gated(decision))
            }
        }
    }

    #[cfg(test)]
    fn with_ledger(
        binding: WorkerProviderGovernorBinding,
        ledger: Arc<dyn WorkerProviderLedger>,
    ) -> Result<Self> {
        binding.validate()?;
        Ok(Self {
            binding: Arc::new(binding),
            ledger,
        })
    }
}

/// Proof that the durable Started row exists for one exact logical call.
/// Dropping this value is intentionally a no-op.
pub struct WorkerProviderCallPermit {
    call: WorkerProviderCall,
    ledger: Arc<dyn WorkerProviderLedger>,
}

impl std::fmt::Debug for WorkerProviderCallPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerProviderCallPermit")
            .field("provider_call_id", &self.call.provider_call_id)
            .field("worker_id", &self.call.worker_id)
            .field("run_id", &self.call.run_id)
            .finish_non_exhaustive()
    }
}

impl WorkerProviderCallPermit {
    pub fn provider_call_id(&self) -> &str {
        &self.call.provider_call_id
    }

    pub fn started_call(&self) -> &WorkerProviderCall {
        &self.call
    }

    pub fn complete(
        &self,
        completion: WorkerProviderCompletion,
    ) -> Result<FinishWorkerProviderCallResult> {
        self.complete_at(completion, Utc::now())
    }

    pub(crate) fn complete_at(
        &self,
        completion: WorkerProviderCompletion,
        finished_at: DateTime<Utc>,
    ) -> Result<FinishWorkerProviderCallResult> {
        let remote_acceptance = match completion.remote_acceptance {
            WorkerProviderCompletionAcceptance::NotSent => ProviderCallRemoteAcceptance::NotSent,
            WorkerProviderCompletionAcceptance::Acknowledged => {
                ProviderCallRemoteAcceptance::Acknowledged
            }
        };
        let estimated_cost_microunits = estimate_worker_provider_call_cost(
            self.call.pricing.as_ref(),
            completion.usage.as_ref(),
        )?;
        self.ledger.finish(&FinishWorkerProviderCall {
            provider_call_id: self.call.provider_call_id.clone(),
            worker_id: self.call.worker_id.clone(),
            run_id: self.call.run_id.clone(),
            state: ProviderCallTerminalState::Completed,
            outcome: completion.outcome.as_str().to_string(),
            remote_acceptance,
            usage: completion.usage,
            estimated_cost_microunits,
            unknown_reason: None,
            finished_at,
        })
    }
}

fn estimate_worker_provider_call_cost(
    pricing: Option<&FrozenModelPriceSnapshot>,
    usage: Option<&Usage>,
) -> Result<Option<u64>> {
    const TOKENS_PER_MILLION: u128 = 1_000_000;
    let (Some(pricing), Some(usage)) = (pricing, usage) else {
        return Ok(None);
    };
    if pricing.currency.as_deref().is_none_or(str::is_empty) {
        return Ok(None);
    }

    let represented_tokens = usage
        .input_tokens()
        .checked_add(usage.completion_tokens)
        .context("Worker provider usage token total overflow")?;
    if usage.total_tokens > represented_tokens {
        // The provider reported tokens that cannot be assigned to a priced
        // bucket. Guessing an input/output split would under- or over-charge.
        return Ok(None);
    }

    let mut numerator = 0_u128;
    for (tokens, rate) in [
        (usage.prompt_tokens, pricing.input_microunits_per_million),
        (
            usage.completion_tokens,
            pricing.output_microunits_per_million,
        ),
        (
            usage.cache_creation_input_tokens,
            pricing.cache_creation_microunits_per_million,
        ),
        (
            usage.cache_read_input_tokens,
            pricing.cache_read_microunits_per_million,
        ),
    ] {
        if tokens == 0 {
            continue;
        }
        let Some(rate) = rate else {
            return Ok(None);
        };
        let component = (tokens as u128)
            .checked_mul(u128::from(rate))
            .context("Worker provider cost multiplication overflow")?;
        numerator = numerator
            .checked_add(component)
            .context("Worker provider cost accumulation overflow")?;
    }

    let rounded_up = numerator
        .checked_add(TOKENS_PER_MILLION - 1)
        .context("Worker provider cost rounding overflow")?
        / TOKENS_PER_MILLION;
    ensure!(
        rounded_up <= i64::MAX as u128,
        "Worker provider cost is out of range"
    );
    Ok(Some(rounded_up as u64))
}

/// Conservative reservation used by tool-free/simple calls when a provider
/// tokenizer is unavailable. Output is added separately and a non-empty call
/// always reserves at least one token.
pub fn conservative_text_token_reservation(text_parts: &[&str], max_output_tokens: usize) -> u64 {
    let input_bytes = text_parts
        .iter()
        .fold(0usize, |total, part| total.saturating_add(part.len()));
    // Three bytes/token is intentionally more conservative than the common
    // four-byte heuristic and includes fixed message/wire overhead.
    let input_tokens = input_bytes
        .saturating_add(2)
        .saturating_div(3)
        .saturating_add(64);
    bounded_reservation(input_tokens, max_output_tokens)
}

pub fn bounded_reservation(input_tokens: usize, max_output_tokens: usize) -> u64 {
    let total = input_tokens.saturating_add(max_output_tokens).max(1);
    u64::try_from(total)
        .unwrap_or(MAX_WORKER_DAILY_TOKEN_LIMIT)
        .min(MAX_WORKER_DAILY_TOKEN_LIMIT)
}

fn validate_text(label: &str, value: &str, max_bytes: usize) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} is empty");
    ensure!(value.len() <= max_bytes, "{label} exceeds the byte limit");
    ensure!(
        !value.chars().any(char::is_control),
        "{label} contains control characters"
    );
    Ok(())
}

#[cfg(test)]
#[path = "provider_governance_tests.rs"]
mod tests;
