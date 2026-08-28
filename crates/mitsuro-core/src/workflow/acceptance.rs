//! Typed acceptance contracts for durable Worker Workflow results.
//!
//! Free-form acceptance prose is presentation, not executable authority.  A
//! legacy criterion therefore always maps to [`WorkflowAcceptanceModeV1::UserReview`].
//! Automatic verification may only execute a frozen, user-approved structural
//! contract and must use a dedicated network-denied verifier runtime.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WORKFLOW_ACCEPTANCE_SPEC_VERSION: u8 = 1;
pub const MAX_WORKFLOW_ACCEPTANCE_CHECKS: usize = 16;
pub const MAX_WORKFLOW_ACCEPTANCE_ARGUMENTS: usize = 32;
pub const MAX_WORKFLOW_ACCEPTANCE_TEXT_BYTES: usize = 4 * 1024;
pub const MAX_WORKFLOW_ACCEPTANCE_TOTAL_ARGUMENT_BYTES: usize = 16 * 1024;
pub const MAX_WORKFLOW_ACCEPTANCE_TIMEOUT_SECS: u32 = 120;
pub const MAX_USER_ACCEPTANCE_CRITERIA: usize = 32;
pub const MAX_USER_ACCEPTANCE_EVIDENCE_ITEMS: usize = 16;

/// Versioned acceptance policy attached to one exact step acceptance item or
/// Goal criterion. Existing string-only rows deserialize through an explicit
/// compatibility reader as `UserReview`; no command is inferred from prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowAcceptanceSpecV1 {
    pub schema_version: u8,
    pub mode: WorkflowAcceptanceModeV1,
}

impl WorkflowAcceptanceSpecV1 {
    pub const fn user_review() -> Self {
        Self {
            schema_version: WORKFLOW_ACCEPTANCE_SPEC_VERSION,
            mode: WorkflowAcceptanceModeV1::UserReview,
        }
    }

    /// Compatibility boundary for every pre-contract/free-form criterion.
    /// The text remains display context only and is deliberately ignored.
    pub const fn from_legacy_free_form(_display_text: &str) -> Self {
        Self::user_review()
    }

    pub const fn requires_user_review(&self) -> bool {
        matches!(&self.mode, WorkflowAcceptanceModeV1::UserReview)
    }

    pub fn validate(&self) -> Result<(), WorkflowAcceptanceValidationError> {
        if self.schema_version != WORKFLOW_ACCEPTANCE_SPEC_VERSION {
            return Err(WorkflowAcceptanceValidationError::UnsupportedVersion(
                self.schema_version,
            ));
        }
        match &self.mode {
            WorkflowAcceptanceModeV1::UserReview => Ok(()),
            WorkflowAcceptanceModeV1::Structural { checks }
            | WorkflowAcceptanceModeV1::StructuralAndSemantic { checks, .. } => {
                validate_checks(checks)?;
                if let WorkflowAcceptanceModeV1::StructuralAndSemantic { rubric, .. } = &self.mode {
                    validate_text("semantic rubric", rubric)?;
                }
                Ok(())
            }
        }
    }
}

impl Default for WorkflowAcceptanceSpecV1 {
    fn default() -> Self {
        Self::user_review()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowAcceptanceModeV1 {
    /// Explicit owner decision is required. This is the only safe compatibility
    /// value for legacy/free-form criteria.
    UserReview,
    /// Host-owned structural checks are sufficient. No provider may select or
    /// rewrite a command.
    Structural {
        checks: Vec<WorkflowStructuralCheckV1>,
    },
    /// Structural checks remain mandatory. A bounded semantic reviewer may
    /// veto or request user review, but can never override a failed check.
    StructuralAndSemantic {
        checks: Vec<WorkflowStructuralCheckV1>,
        rubric: String,
    },
}

/// Frozen checks interpreted by a dedicated verifier, never by `bash -c` and
/// never through an Agent tool call. The verifier sandbox always denies the
/// network; there is intentionally no serializable allow-network variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowStructuralCheckV1 {
    CommandExit {
        argv: Vec<String>,
        relative_cwd: Option<String>,
        expected_exit_code: i32,
        timeout_secs: u32,
    },
    PathState {
        relative_path: String,
        expected: WorkflowPathExpectationV1,
    },
    ContentDigest {
        relative_path: String,
        sha256: String,
    },
}

impl WorkflowStructuralCheckV1 {
    pub const fn network_allowed(&self) -> bool {
        false
    }

    fn validate(&self) -> Result<(), WorkflowAcceptanceValidationError> {
        match self {
            Self::CommandExit {
                argv,
                relative_cwd,
                timeout_secs,
                ..
            } => {
                if argv.is_empty() || argv.len() > MAX_WORKFLOW_ACCEPTANCE_ARGUMENTS {
                    return Err(WorkflowAcceptanceValidationError::InvalidArgumentVector);
                }
                let mut total_bytes = 0usize;
                for argument in argv {
                    validate_text("command argument", argument)?;
                    if argument.contains('\0') {
                        return Err(WorkflowAcceptanceValidationError::InvalidText(
                            "command argument",
                        ));
                    }
                    total_bytes = total_bytes.saturating_add(argument.len());
                }
                if total_bytes > MAX_WORKFLOW_ACCEPTANCE_TOTAL_ARGUMENT_BYTES {
                    return Err(WorkflowAcceptanceValidationError::InvalidArgumentVector);
                }
                if let Some(relative_cwd) = relative_cwd {
                    validate_workspace_relative_path("command working directory", relative_cwd)?;
                }
                if !(1..=MAX_WORKFLOW_ACCEPTANCE_TIMEOUT_SECS).contains(timeout_secs) {
                    return Err(WorkflowAcceptanceValidationError::InvalidTimeout);
                }
                Ok(())
            }
            Self::PathState { relative_path, .. } => {
                validate_workspace_relative_path("path-state path", relative_path)
            }
            Self::ContentDigest {
                relative_path,
                sha256,
            } => {
                validate_workspace_relative_path("content-digest path", relative_path)?;
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(WorkflowAcceptanceValidationError::InvalidSha256);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPathExpectationV1 {
    Missing,
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserWorkerGoalAcceptanceDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserGoalCriterionDecision {
    Passed,
    Failed,
    Waived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserGoalCriterionAcceptance {
    pub criterion_id: String,
    pub decision: UserGoalCriterionDecision,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// Explicit authenticated-owner resolution for one exact pending acceptance
/// candidate. All Workflow/Worker/step identities are derived from the durable
/// acceptance run; the client supplies no mutation target other than that run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserWorkerGoalAcceptanceRequest {
    pub acceptance_run_id: String,
    pub expected_goal_revision: u64,
    pub decision: UserWorkerGoalAcceptanceDecision,
    pub reason: String,
    #[serde(default)]
    pub criteria: Vec<UserGoalCriterionAcceptance>,
}

impl UserWorkerGoalAcceptanceRequest {
    pub fn validate(&self) -> Result<(), WorkflowAcceptanceValidationError> {
        validate_text("acceptance run id", &self.acceptance_run_id)?;
        validate_text("acceptance reason", &self.reason)?;
        if self.expected_goal_revision == 0 {
            return Err(WorkflowAcceptanceValidationError::InvalidRevision);
        }
        if self.criteria.len() > MAX_USER_ACCEPTANCE_CRITERIA {
            return Err(WorkflowAcceptanceValidationError::TooManyCriteria);
        }
        let mut criterion_ids = std::collections::HashSet::with_capacity(self.criteria.len());
        for criterion in &self.criteria {
            validate_text("Goal criterion id", &criterion.criterion_id)?;
            if !criterion_ids.insert(criterion.criterion_id.as_str()) {
                return Err(WorkflowAcceptanceValidationError::DuplicateCriterion);
            }
            if criterion.evidence.len() > MAX_USER_ACCEPTANCE_EVIDENCE_ITEMS {
                return Err(WorkflowAcceptanceValidationError::TooManyEvidenceItems);
            }
            for evidence in &criterion.evidence {
                validate_text("criterion evidence", evidence)?;
            }
            if matches!(
                criterion.decision,
                UserGoalCriterionDecision::Passed | UserGoalCriterionDecision::Failed
            ) && criterion.evidence.is_empty()
            {
                return Err(WorkflowAcceptanceValidationError::MissingEvidence);
            }
        }
        if self.decision == UserWorkerGoalAcceptanceDecision::Reject && !self.criteria.is_empty() {
            return Err(WorkflowAcceptanceValidationError::CriteriaOnRejectedStep);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkflowAcceptanceValidationError {
    #[error("unsupported Workflow acceptance contract version {0}")]
    UnsupportedVersion(u8),
    #[error("Workflow acceptance contract requires between one and sixteen checks")]
    InvalidCheckCount,
    #[error("Workflow acceptance command argv is empty or exceeds its bound")]
    InvalidArgumentVector,
    #[error("Workflow acceptance command timeout is outside its bound")]
    InvalidTimeout,
    #[error("Workflow acceptance {0} is empty, malformed, or exceeds its byte bound")]
    InvalidText(&'static str),
    #[error("Workflow acceptance {0} must be workspace-relative without '..'")]
    InvalidRelativePath(&'static str),
    #[error("Workflow acceptance content digest is not lowercase SHA-256")]
    InvalidSha256,
    #[error("Workflow acceptance revision must be nonzero")]
    InvalidRevision,
    #[error("Workflow acceptance request contains too many Goal criteria")]
    TooManyCriteria,
    #[error("Workflow acceptance request repeats a Goal criterion")]
    DuplicateCriterion,
    #[error("Workflow acceptance request contains too many evidence items")]
    TooManyEvidenceItems,
    #[error("passed or failed Goal criteria require concrete evidence")]
    MissingEvidence,
    #[error("a rejected step cannot mutate Goal criteria")]
    CriteriaOnRejectedStep,
}

fn validate_checks(
    checks: &[WorkflowStructuralCheckV1],
) -> Result<(), WorkflowAcceptanceValidationError> {
    if checks.is_empty() || checks.len() > MAX_WORKFLOW_ACCEPTANCE_CHECKS {
        return Err(WorkflowAcceptanceValidationError::InvalidCheckCount);
    }
    for check in checks {
        check.validate()?;
    }
    Ok(())
}

fn validate_text(
    label: &'static str,
    value: &str,
) -> Result<(), WorkflowAcceptanceValidationError> {
    if value.trim().is_empty()
        || value.trim() != value
        || value.len() > MAX_WORKFLOW_ACCEPTANCE_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(WorkflowAcceptanceValidationError::InvalidText(label));
    }
    Ok(())
}

fn validate_workspace_relative_path(
    label: &'static str,
    value: &str,
) -> Result<(), WorkflowAcceptanceValidationError> {
    validate_text(label, value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(WorkflowAcceptanceValidationError::InvalidRelativePath(
            label,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_free_form_is_always_user_review() {
        for prose in [
            "cargo test passes",
            "echo test ok",
            "the Worker says this is done",
        ] {
            let spec = WorkflowAcceptanceSpecV1::from_legacy_free_form(prose);
            assert_eq!(spec, WorkflowAcceptanceSpecV1::user_review());
            assert!(spec.requires_user_review());
            assert!(spec.validate().is_ok());
        }
    }

    #[test]
    fn structural_command_is_exact_and_network_denied() {
        let check = WorkflowStructuralCheckV1::CommandExit {
            argv: vec!["cargo".into(), "test".into(), "--lib".into()],
            relative_cwd: Some("crates/mitsuro-core".into()),
            expected_exit_code: 0,
            timeout_secs: 60,
        };
        assert!(!check.network_allowed());
        let spec = WorkflowAcceptanceSpecV1 {
            schema_version: 1,
            mode: WorkflowAcceptanceModeV1::Structural {
                checks: vec![check],
            },
        };
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn structural_contract_rejects_escape_and_malformed_digest() {
        let escape = WorkflowAcceptanceSpecV1 {
            schema_version: 1,
            mode: WorkflowAcceptanceModeV1::Structural {
                checks: vec![WorkflowStructuralCheckV1::PathState {
                    relative_path: "../outside".into(),
                    expected: WorkflowPathExpectationV1::File,
                }],
            },
        };
        assert!(matches!(
            escape.validate(),
            Err(WorkflowAcceptanceValidationError::InvalidRelativePath(_))
        ));

        let digest = WorkflowAcceptanceSpecV1 {
            schema_version: 1,
            mode: WorkflowAcceptanceModeV1::Structural {
                checks: vec![WorkflowStructuralCheckV1::ContentDigest {
                    relative_path: "src/lib.rs".into(),
                    sha256: "ABC".into(),
                }],
            },
        };
        assert!(matches!(
            digest.validate(),
            Err(WorkflowAcceptanceValidationError::InvalidSha256)
        ));
    }

    #[test]
    fn explicit_user_request_is_bounded_and_reject_cannot_touch_criteria() {
        let request = UserWorkerGoalAcceptanceRequest {
            acceptance_run_id: "acceptance-run-1".into(),
            expected_goal_revision: 4,
            decision: UserWorkerGoalAcceptanceDecision::Reject,
            reason: "The result does not satisfy the approved design".into(),
            criteria: vec![UserGoalCriterionAcceptance {
                criterion_id: "criterion-1".into(),
                decision: UserGoalCriterionDecision::Passed,
                evidence: vec!["reviewed by owner".into()],
            }],
        };
        assert_eq!(
            request.validate(),
            Err(WorkflowAcceptanceValidationError::CriteriaOnRejectedStep)
        );
    }
}
