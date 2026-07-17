use crate::storage::{LearningCandidateStatus, LearningKind, LearningSensitivity};

use super::LearningProposal;

const MAX_CANONICAL_KEY_BYTES: usize = 160;
const MAX_CONTENT_BYTES: usize = 2 * 1024;
const MAX_EVIDENCE_BYTES: usize = 512;
const AUTO_ACCEPT_CONFIDENCE: f64 = 0.95;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningDecision {
    pub status: LearningCandidateStatus,
    pub reason: String,
}

pub struct LearningPolicy;

impl LearningPolicy {
    pub fn evaluate(proposal: &LearningProposal) -> LearningDecision {
        if proposal.canonical_key.trim().is_empty()
            || proposal.canonical_key.len() > MAX_CANONICAL_KEY_BYTES
            || !proposal
                .canonical_key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
        {
            return reject("invalid canonical key");
        }
        if proposal.content.trim().is_empty() || proposal.content.len() > MAX_CONTENT_BYTES {
            return reject("content is empty or exceeds the learning budget");
        }
        if proposal.evidence_message_id <= 0
            || proposal.evidence_excerpt.trim().is_empty()
            || proposal.evidence_excerpt.len() > MAX_EVIDENCE_BYTES
        {
            return reject("exact bounded evidence is required");
        }
        if proposal.sensitivity != LearningSensitivity::Normal
            || contains_sensitive_pattern(&proposal.content)
            || contains_sensitive_pattern(&proposal.evidence_excerpt)
        {
            return reject("sensitive or prohibited material is never learned automatically");
        }
        if proposal.kind == LearningKind::Forget {
            return LearningDecision {
                status: LearningCandidateStatus::Tombstoned,
                reason: "explicit forget requests create a durable tombstone".to_string(),
            };
        }
        if proposal.explicit
            && proposal.confidence >= AUTO_ACCEPT_CONFIDENCE
            && matches!(
                proposal.kind,
                LearningKind::UserPreference | LearningKind::UserCorrection
            )
        {
            return LearningDecision {
                status: LearningCandidateStatus::AutoAccepted,
                reason: "explicit high-confidence user preference or correction".to_string(),
            };
        }

        LearningDecision {
            status: LearningCandidateStatus::Pending,
            reason: "requires user review before durable promotion".to_string(),
        }
    }
}

fn reject(reason: &str) -> LearningDecision {
    LearningDecision {
        status: LearningCandidateStatus::Rejected,
        reason: reason.to_string(),
    }
}

fn contains_sensitive_pattern(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "api_key",
        "api key",
        "access token",
        "refresh token",
        "private key",
        "password",
        "secret=",
        "authorization: bearer",
        "social security",
        "credit card",
        "medical diagnosis",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::{LearningPolicy, AUTO_ACCEPT_CONFIDENCE};
    use crate::agent::learning::LearningProposal;
    use crate::storage::{LearningCandidateStatus, LearningKind, LearningSensitivity};

    fn proposal() -> LearningProposal {
        LearningProposal {
            canonical_key: "communication.progress_updates".to_string(),
            kind: LearningKind::UserPreference,
            content: "The user prefers concise progress updates.".to_string(),
            evidence_message_id: 12,
            evidence_excerpt: "Keep the progress updates concise.".to_string(),
            explicit: true,
            confidence: AUTO_ACCEPT_CONFIDENCE,
            sensitivity: LearningSensitivity::Normal,
        }
    }

    #[test]
    fn only_explicit_safe_preferences_auto_accept() {
        assert_eq!(
            LearningPolicy::evaluate(&proposal()).status,
            LearningCandidateStatus::AutoAccepted
        );
        let mut inferred = proposal();
        inferred.explicit = false;
        assert_eq!(
            LearningPolicy::evaluate(&inferred).status,
            LearningCandidateStatus::Pending
        );
    }

    #[test]
    fn sensitive_content_is_rejected_even_when_labeled_normal() {
        let mut sensitive = proposal();
        sensitive.content = "The user's API key is secret=abc".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&sensitive).status,
            LearningCandidateStatus::Rejected
        );
    }

    #[test]
    fn forget_produces_tombstone() {
        let mut forget = proposal();
        forget.kind = LearningKind::Forget;
        assert_eq!(
            LearningPolicy::evaluate(&forget).status,
            LearningCandidateStatus::Tombstoned
        );
    }
}
