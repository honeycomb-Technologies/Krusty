//! Immutable acceptance records for one exact Worker Workflow result.
//!
//! V1 exposes the explicit-owner `UserReview` path first. Structural and
//! semantic contracts are represented here, but no automatic verifier may be
//! enabled until a network-denied receipt executor and fenced provider/result
//! committer are wired end to end.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::{
    WorkerGoalAttemptOutcome, WorkerGoalEffectSummary, WorkerGoalEvidence,
    WorkerGoalOutcomeCounters,
};
#[cfg(test)]
use crate::workflow::WorkflowStructuralCheckV1;
use crate::workflow::{
    UserGoalCriterionAcceptance, UserWorkerGoalAcceptanceDecision, WorkflowAcceptanceModeV1,
    WorkflowAcceptanceSpecV1,
};

pub const WORKER_GOAL_ACCEPTANCE_CONTRACT_VERSION: u8 = 1;
pub const WORKER_GOAL_ACCEPTANCE_INTENT_VERSION: u8 = 1;
pub const MAX_WORKER_GOAL_ACCEPTANCE_RECEIPTS: usize = 32;
pub const MAX_WORKER_GOAL_ACCEPTANCE_RECEIPT_SUMMARY_BYTES: usize = 2 * 1024;
pub const MAX_WORKER_GOAL_ACCEPTANCE_RECEIPT_DURATION_MILLIS: u64 = 120_000;
/// V1 deliberately ships only the explicit-owner `UserReview` authority.
/// This may become true only with a proven executable allowlist, network and
/// process sandbox, and symlink-safe workspace containment.
pub const WORKER_GOAL_AUTOMATIC_ACCEPTANCE_ENABLED: bool = false;

/// Exact positional contract frozen when a `Progressed` source outcome stages
/// acceptance. Legacy/free-form items are represented as `UserReview`; they
/// never become executable commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerGoalAcceptanceContractV1 {
    pub schema_version: u8,
    pub step_specs: Vec<WorkflowAcceptanceSpecV1>,
    pub goal_specs: Vec<WorkerGoalCriterionAcceptanceSpecV1>,
}

impl WorkerGoalAcceptanceContractV1 {
    pub fn validate(&self) -> bool {
        let check_count = self
            .step_specs
            .iter()
            .chain(self.goal_specs.iter().map(|item| &item.spec))
            .map(|spec| match &spec.mode {
                WorkflowAcceptanceModeV1::UserReview => 0,
                WorkflowAcceptanceModeV1::Structural { checks }
                | WorkflowAcceptanceModeV1::StructuralAndSemantic { checks, .. } => checks.len(),
            })
            .sum::<usize>();
        if self.schema_version != WORKER_GOAL_ACCEPTANCE_CONTRACT_VERSION
            || self.step_specs.len() > 32
            || self.goal_specs.len() > 32
            || check_count > MAX_WORKER_GOAL_ACCEPTANCE_RECEIPTS
            || self.step_specs.iter().any(|spec| spec.validate().is_err())
            || self.goal_specs.iter().any(|item| item.validate().is_err())
        {
            return false;
        }
        let mut criterion_ids = std::collections::HashSet::with_capacity(self.goal_specs.len());
        self.goal_specs
            .iter()
            .all(|item| criterion_ids.insert(item.criterion_id.as_str()))
    }

    pub const fn expected_step_assessments(&self) -> usize {
        self.step_specs.len()
    }

    pub const fn expected_goal_assessments(&self) -> usize {
        self.goal_specs.len()
    }

    pub fn requires_user_review(&self) -> bool {
        self.step_specs
            .iter()
            .chain(self.goal_specs.iter().map(|item| &item.spec))
            .any(WorkflowAcceptanceSpecV1::requires_user_review)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerGoalCriterionAcceptanceSpecV1 {
    pub criterion_id: String,
    pub spec: WorkflowAcceptanceSpecV1,
}

impl WorkerGoalCriterionAcceptanceSpecV1 {
    fn validate(&self) -> Result<(), ()> {
        if self.criterion_id.trim().is_empty()
            || self.criterion_id.trim() != self.criterion_id
            || self.criterion_id.len() > 256
            || self.criterion_id.chars().any(char::is_control)
            || self.spec.validate().is_err()
        {
            return Err(());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAcceptanceCandidateState {
    AwaitingUser,
    Verifying,
    NeedsUser,
    Accepted,
    Rejected,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAcceptanceAuthority {
    User,
    Lifecycle,
    Structural,
    StructuralAndSemantic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAcceptanceAssessment {
    Pass,
    Fail,
    NeedsUser,
}

/// Provider output is positional. It cannot choose a Worker, Goal, attempt,
/// plan, step, criterion, command, or durable state transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerGoalAcceptanceIntentV1 {
    pub schema_version: u8,
    pub step_assessments: Vec<WorkerGoalAcceptanceAssessment>,
    pub goal_assessments: Vec<WorkerGoalAcceptanceAssessment>,
}

impl WorkerGoalAcceptanceIntentV1 {
    pub fn validate_shape(
        &self,
        expected_step_assessments: usize,
        expected_goal_assessments: usize,
    ) -> bool {
        self.schema_version == WORKER_GOAL_ACCEPTANCE_INTENT_VERSION
            && self.step_assessments.len() == expected_step_assessments
            && self.goal_assessments.len() == expected_goal_assessments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAcceptanceReceiptKind {
    CommandExit,
    PathState,
    ContentDigest,
}

/// Bounded structural observation. Raw stdout/stderr and file contents never
/// enter this record; only a stable digest and privacy-safe summary may persist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerGoalAcceptanceReceipt {
    pub check_index: u16,
    pub kind: WorkerGoalAcceptanceReceiptKind,
    pub passed: bool,
    pub summary: String,
    pub observation_sha256: String,
    pub duration_millis: u64,
}

impl WorkerGoalAcceptanceReceipt {
    pub fn validate(&self) -> bool {
        !self.summary.trim().is_empty()
            && self.summary.trim() == self.summary
            && self.summary.len() <= MAX_WORKER_GOAL_ACCEPTANCE_RECEIPT_SUMMARY_BYTES
            && !self.summary.chars().any(char::is_control)
            && self.duration_millis <= MAX_WORKER_GOAL_ACCEPTANCE_RECEIPT_DURATION_MILLIS
            && self.observation_sha256.len() == 64
            && self
                .observation_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }
}

/// Candidate frozen from a committed `Progressed` source outcome. Every
/// identity and revision is storage-authored; user/provider payloads select
/// only a decision for this already-fenced candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalAcceptanceSourceSummary {
    pub outcome: WorkerGoalAttemptOutcome,
    pub evidence: Vec<WorkerGoalEvidence>,
    pub effect: WorkerGoalEffectSummary,
    pub counters: WorkerGoalOutcomeCounters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalAcceptanceCandidateRecord {
    pub acceptance_run_id: String,
    pub source_run_id: String,
    pub worker_id: String,
    pub worker_revision: u64,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub workflow_goal_id: String,
    pub source_attempt_id: String,
    pub plan_revision_id: String,
    pub plan_revision_number: u64,
    pub step_id: String,
    pub goal_revision: u64,
    pub workflow_aggregate_revision: u64,
    pub step_revision: u64,
    pub workspace_dir: String,
    pub acceptance_contract: WorkerGoalAcceptanceContractV1,
    pub acceptance_contract_sha256: String,
    pub source_outcome_sha256: String,
    /// Privacy-safe source evidence reconstructed from the exact immutable
    /// Worker Goal outcome and verified against `source_outcome_sha256`.
    pub source_summary: WorkerGoalAcceptanceSourceSummary,
    pub state: WorkerGoalAcceptanceCandidateState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalAcceptanceResultRecord {
    pub acceptance_run_id: String,
    pub source_run_id: String,
    pub authority: WorkerGoalAcceptanceAuthority,
    pub decision: UserWorkerGoalAcceptanceDecision,
    pub reason: String,
    pub criteria: Vec<UserGoalCriterionAcceptance>,
    pub receipts: Vec<WorkerGoalAcceptanceReceipt>,
    pub provider_call_ids: Vec<String>,
    /// Frozen response projection captured in the same transaction as the
    /// owner decision. Lifecycle invalidations never expose a user response.
    pub resulting_goal_revision: Option<u64>,
    pub resulting_goal_status: Option<String>,
    pub resulting_step_status: Option<String>,
    pub committed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerGoalAcceptanceCommitDisposition {
    Inserted,
    AdoptedExact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalAcceptanceResolution {
    pub disposition: WorkerGoalAcceptanceCommitDisposition,
    pub acceptance_run_id: String,
    pub source_run_id: String,
    pub workflow_goal_id: String,
    pub source_attempt_id: String,
    pub step_id: String,
    pub decision: UserWorkerGoalAcceptanceDecision,
    pub goal_revision: u64,
    pub goal_status: String,
    pub step_status: String,
}

/// Host aggregation rule for the future automatic path. Semantic output may
/// only downgrade an all-passing structural result.
#[cfg(test)]
pub(crate) fn aggregate_automatic_acceptance(
    receipts: &[WorkerGoalAcceptanceReceipt],
    semantic: Option<&WorkerGoalAcceptanceIntentV1>,
    contract: &WorkerGoalAcceptanceContractV1,
) -> WorkerGoalAcceptanceAssessment {
    if !contract.validate() || contract.requires_user_review() {
        return WorkerGoalAcceptanceAssessment::NeedsUser;
    }
    let expected_receipt_kinds = contract
        .step_specs
        .iter()
        .chain(contract.goal_specs.iter().map(|item| &item.spec))
        .flat_map(|spec| match &spec.mode {
            WorkflowAcceptanceModeV1::UserReview => &[][..],
            WorkflowAcceptanceModeV1::Structural { checks }
            | WorkflowAcceptanceModeV1::StructuralAndSemantic { checks, .. } => checks.as_slice(),
        })
        .map(|check| match check {
            WorkflowStructuralCheckV1::CommandExit { .. } => {
                WorkerGoalAcceptanceReceiptKind::CommandExit
            }
            WorkflowStructuralCheckV1::PathState { .. } => {
                WorkerGoalAcceptanceReceiptKind::PathState
            }
            WorkflowStructuralCheckV1::ContentDigest { .. } => {
                WorkerGoalAcceptanceReceiptKind::ContentDigest
            }
        })
        .collect::<Vec<_>>();
    if receipts.is_empty()
        || receipts.len() > MAX_WORKER_GOAL_ACCEPTANCE_RECEIPTS
        || receipts.len() != expected_receipt_kinds.len()
        || receipts.iter().any(|receipt| !receipt.validate())
        || receipts.iter().enumerate().any(|(index, receipt)| {
            usize::from(receipt.check_index) != index
                || expected_receipt_kinds.get(index) != Some(&receipt.kind)
        })
    {
        return WorkerGoalAcceptanceAssessment::NeedsUser;
    }
    if receipts.iter().any(|receipt| !receipt.passed) {
        return WorkerGoalAcceptanceAssessment::Fail;
    }
    let semantic_required = contract
        .step_specs
        .iter()
        .chain(contract.goal_specs.iter().map(|item| &item.spec))
        .any(|spec| {
            matches!(
                &spec.mode,
                WorkflowAcceptanceModeV1::StructuralAndSemantic { .. }
            )
        });
    let Some(semantic) = semantic else {
        return if semantic_required {
            WorkerGoalAcceptanceAssessment::NeedsUser
        } else {
            WorkerGoalAcceptanceAssessment::Pass
        };
    };
    if !semantic.validate_shape(
        contract.expected_step_assessments(),
        contract.expected_goal_assessments(),
    ) {
        return WorkerGoalAcceptanceAssessment::NeedsUser;
    }
    if semantic
        .step_assessments
        .iter()
        .chain(&semantic.goal_assessments)
        .any(|assessment| *assessment == WorkerGoalAcceptanceAssessment::Fail)
    {
        WorkerGoalAcceptanceAssessment::Fail
    } else if semantic
        .step_assessments
        .iter()
        .chain(&semantic.goal_assessments)
        .any(|assessment| *assessment == WorkerGoalAcceptanceAssessment::NeedsUser)
    {
        WorkerGoalAcceptanceAssessment::NeedsUser
    } else {
        WorkerGoalAcceptanceAssessment::Pass
    }
}

pub(crate) fn exact_result_payload(record: &WorkerGoalAcceptanceResultRecord) -> Value {
    serde_json::json!({
        "authority": record.authority,
        "decision": record.decision,
        "reason": record.reason,
        "criteria": record.criteria,
        "receipts": record.receipts,
        "provider_call_ids": record.provider_call_ids,
        "resulting_goal_revision": record.resulting_goal_revision,
        "resulting_goal_status": record.resulting_goal_status,
        "resulting_step_status": record.resulting_step_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::WorkflowStructuralCheckV1;

    fn receipt(check_index: u16, passed: bool) -> WorkerGoalAcceptanceReceipt {
        WorkerGoalAcceptanceReceipt {
            check_index,
            kind: WorkerGoalAcceptanceReceiptKind::CommandExit,
            passed,
            summary: "Exact frozen argv exited with the expected status".into(),
            observation_sha256: "a".repeat(64),
            duration_millis: 12,
        }
    }

    fn spec(semantic: bool) -> WorkflowAcceptanceSpecV1 {
        let checks = vec![WorkflowStructuralCheckV1::CommandExit {
            argv: vec!["cargo".into(), "test".into(), "--locked".into()],
            relative_cwd: None,
            expected_exit_code: 0,
            timeout_secs: 60,
        }];
        WorkflowAcceptanceSpecV1 {
            schema_version: 1,
            mode: if semantic {
                WorkflowAcceptanceModeV1::StructuralAndSemantic {
                    checks,
                    rubric: "The exact requested behavior is covered".into(),
                }
            } else {
                WorkflowAcceptanceModeV1::Structural { checks }
            },
        }
    }

    fn contract(
        step_count: usize,
        goal_count: usize,
        semantic: bool,
    ) -> WorkerGoalAcceptanceContractV1 {
        WorkerGoalAcceptanceContractV1 {
            schema_version: WORKER_GOAL_ACCEPTANCE_CONTRACT_VERSION,
            step_specs: (0..step_count).map(|_| spec(semantic)).collect(),
            goal_specs: (0..goal_count)
                .map(|index| WorkerGoalCriterionAcceptanceSpecV1 {
                    criterion_id: format!("criterion-{index}"),
                    spec: spec(semantic),
                })
                .collect(),
        }
    }

    fn passing_receipts(count: usize) -> Vec<WorkerGoalAcceptanceReceipt> {
        (0..count)
            .map(|index| receipt(u16::try_from(index).unwrap(), true))
            .collect()
    }

    #[test]
    fn semantic_assessments_require_the_exact_frozen_positional_shape() {
        for intent in [
            WorkerGoalAcceptanceIntentV1 {
                schema_version: 1,
                step_assessments: vec![],
                goal_assessments: vec![],
            },
            WorkerGoalAcceptanceIntentV1 {
                schema_version: 1,
                step_assessments: vec![WorkerGoalAcceptanceAssessment::Pass],
                goal_assessments: vec![],
            },
            WorkerGoalAcceptanceIntentV1 {
                schema_version: 1,
                step_assessments: vec![
                    WorkerGoalAcceptanceAssessment::Pass,
                    WorkerGoalAcceptanceAssessment::Pass,
                    WorkerGoalAcceptanceAssessment::Pass,
                ],
                goal_assessments: vec![WorkerGoalAcceptanceAssessment::Pass],
            },
        ] {
            let contract = contract(2, 1, true);
            assert_eq!(
                aggregate_automatic_acceptance(&passing_receipts(3), Some(&intent), &contract,),
                WorkerGoalAcceptanceAssessment::NeedsUser
            );
        }
    }

    #[test]
    fn semantic_pass_cannot_override_failed_structural_evidence_with_exact_shape() {
        let intent = WorkerGoalAcceptanceIntentV1 {
            schema_version: 1,
            step_assessments: vec![WorkerGoalAcceptanceAssessment::Pass],
            goal_assessments: vec![],
        };
        let contract = contract(1, 0, true);
        assert_eq!(
            aggregate_automatic_acceptance(&[receipt(0, false)], Some(&intent), &contract),
            WorkerGoalAcceptanceAssessment::Fail
        );
    }

    #[test]
    fn malformed_or_missing_structural_evidence_needs_the_user() {
        assert_eq!(
            aggregate_automatic_acceptance(&[], None, &contract(1, 0, false)),
            WorkerGoalAcceptanceAssessment::NeedsUser
        );
        let mut invalid = receipt(0, true);
        invalid.observation_sha256 = "not-a-digest".into();
        assert_eq!(
            aggregate_automatic_acceptance(&[invalid], None, &contract(1, 0, false)),
            WorkerGoalAcceptanceAssessment::NeedsUser
        );
    }

    #[test]
    fn semantic_review_can_only_pass_or_downgrade_structural_pass() {
        let needs_user = WorkerGoalAcceptanceIntentV1 {
            schema_version: 1,
            step_assessments: vec![WorkerGoalAcceptanceAssessment::NeedsUser],
            goal_assessments: vec![],
        };
        let semantic_contract = contract(1, 0, true);
        assert_eq!(
            aggregate_automatic_acceptance(
                &[receipt(0, true)],
                Some(&needs_user),
                &semantic_contract,
            ),
            WorkerGoalAcceptanceAssessment::NeedsUser
        );
        assert_eq!(
            aggregate_automatic_acceptance(&[receipt(0, true)], None, &contract(1, 0, false),),
            WorkerGoalAcceptanceAssessment::Pass
        );
    }

    #[test]
    fn user_review_contract_never_enters_automatic_acceptance() {
        let contract = WorkerGoalAcceptanceContractV1 {
            schema_version: 1,
            step_specs: vec![WorkflowAcceptanceSpecV1::user_review()],
            goal_specs: Vec::new(),
        };
        assert_eq!(
            aggregate_automatic_acceptance(&[receipt(0, true)], None, &contract),
            WorkerGoalAcceptanceAssessment::NeedsUser
        );
    }

    #[test]
    fn receipts_must_match_the_frozen_check_order_and_kind_exactly() {
        let contract = contract(2, 0, false);
        assert_eq!(
            aggregate_automatic_acceptance(&[receipt(0, true)], None, &contract),
            WorkerGoalAcceptanceAssessment::NeedsUser
        );
        let mut out_of_order = passing_receipts(2);
        out_of_order[1].check_index = 0;
        assert_eq!(
            aggregate_automatic_acceptance(&out_of_order, None, &contract),
            WorkerGoalAcceptanceAssessment::NeedsUser
        );
    }
}
