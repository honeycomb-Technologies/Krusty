use crate::storage::{LearningCandidateStatus, LearningKind, LearningSensitivity};

use super::{LearningProposal, LearningScope};

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
        if contains_sensitive_pattern(&proposal.canonical_key) {
            return reject("sensitive or prohibited canonical key");
        }
        if !kind_scope_and_key_align(proposal) {
            return reject("learning kind, scope, and canonical key prefix do not align");
        }
        if proposal.content.trim().is_empty() || proposal.content.len() > MAX_CONTENT_BYTES {
            return reject("content is empty or exceeds the learning budget");
        }
        if !proposal.confidence.is_finite() || !(0.0..=1.0).contains(&proposal.confidence) {
            return reject("confidence must be finite and between zero and one");
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
            if !proposal.explicit
                || proposal.confidence < AUTO_ACCEPT_CONFIDENCE
                || !evidence_is_explicit(&proposal.evidence_excerpt, LearningKind::Forget)
            {
                return LearningDecision {
                    status: LearningCandidateStatus::Pending,
                    reason: "forgetting requires an explicit high-confidence user request"
                        .to_string(),
                };
            }
            return LearningDecision {
                status: LearningCandidateStatus::Tombstoned,
                reason: "explicit forget requests create a durable tombstone".to_string(),
            };
        }
        if proposal.scope == LearningScope::Project {
            return LearningDecision {
                status: LearningCandidateStatus::Pending,
                reason: "project-scoped learning requires user review".to_string(),
            };
        }
        if proposal.explicit
            && proposal.confidence >= AUTO_ACCEPT_CONFIDENCE
            && matches!(
                proposal.kind,
                LearningKind::UserPreference | LearningKind::UserCorrection
            )
            && evidence_is_explicit(&proposal.evidence_excerpt, proposal.kind)
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

fn kind_scope_and_key_align(proposal: &LearningProposal) -> bool {
    let key = proposal.canonical_key.as_str();
    match proposal.kind {
        LearningKind::UserPreference => {
            proposal.scope == LearningScope::User && has_key_prefix(key, "preference.")
        }
        LearningKind::UserCorrection => {
            proposal.scope == LearningScope::User && has_key_prefix(key, "correction.")
        }
        LearningKind::RelationshipContext => {
            proposal.scope == LearningScope::User && has_key_prefix(key, "relationship.")
        }
        LearningKind::ProjectFact => {
            proposal.scope == LearningScope::Project && has_key_prefix(key, "project.")
        }
        LearningKind::Procedure => {
            proposal.scope == LearningScope::Project && has_key_prefix(key, "procedure.")
        }
        LearningKind::Forget => match proposal.scope {
            LearningScope::User => ["preference.", "correction.", "relationship."]
                .iter()
                .any(|prefix| has_key_prefix(key, prefix)),
            LearningScope::Project => ["project.", "procedure."]
                .iter()
                .any(|prefix| has_key_prefix(key, prefix)),
        },
    }
}

fn has_key_prefix(key: &str, prefix: &str) -> bool {
    key.strip_prefix(prefix)
        .is_some_and(|suffix| !suffix.is_empty())
}

pub(super) fn auto_promotion_key_allowed(kind: LearningKind, key: &str) -> bool {
    let aligned = match kind {
        LearningKind::UserPreference => has_key_prefix(key, "preference."),
        LearningKind::UserCorrection => has_key_prefix(key, "correction."),
        LearningKind::ProjectFact
        | LearningKind::Procedure
        | LearningKind::RelationshipContext
        | LearningKind::Forget => false,
    };
    aligned && !contains_sensitive_pattern(key)
}

fn evidence_is_explicit(evidence: &str, kind: LearningKind) -> bool {
    let lower = evidence.to_ascii_lowercase();
    let patterns: &[&str] = match kind {
        LearningKind::UserPreference => &[
            "i prefer",
            "i want",
            "i like",
            "my preference",
            "please ",
            "always ",
            "never ",
            "do not ",
            "don't ",
            "keep ",
            "use ",
        ],
        LearningKind::UserCorrection => &[
            "actually",
            "i meant",
            "that's wrong",
            "that is wrong",
            "correction",
            "instead",
            "do not ",
            "don't ",
            "no,",
        ],
        LearningKind::Forget => &[
            "forget ",
            "forget that",
            "delete ",
            "remove ",
            "do not remember",
            "don't remember",
        ],
        LearningKind::ProjectFact | LearningKind::Procedure | LearningKind::RelationshipContext => {
            return false
        }
    };
    patterns.iter().any(|pattern| lower.contains(pattern))
}

fn reject(reason: &str) -> LearningDecision {
    LearningDecision {
        status: LearningCandidateStatus::Rejected,
        reason: reason.to_string(),
    }
}

fn contains_sensitive_pattern(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    // This is intentionally a bounded defense-in-depth denylist, not a claim
    // to recognize every credential format. The reviewer must also label
    // sensitive material, and exact evidence validation still applies.
    let bounded_phrases = [
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
        "-----begin",
        "private key-----",
    ];
    bounded_phrases
        .iter()
        .any(|pattern| lower.contains(pattern))
        || lower
            .split(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-' && character != '_'
            })
            .any(|token| {
                ["ghp_", "github_pat_", "xoxb-", "sk-"]
                    .iter()
                    .any(|prefix| token.starts_with(prefix))
            })
}

#[cfg(test)]
mod tests {
    use super::{LearningPolicy, AUTO_ACCEPT_CONFIDENCE};
    use crate::agent::learning::{LearningProposal, LearningScope};
    use crate::storage::{LearningCandidateStatus, LearningKind, LearningSensitivity};

    fn proposal() -> LearningProposal {
        LearningProposal {
            canonical_key: "preference.progress_updates".to_string(),
            kind: LearningKind::UserPreference,
            scope: LearningScope::User,
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
    fn model_supplied_explicit_flag_is_not_enough_without_explicit_evidence() {
        let mut inferred = proposal();
        inferred.evidence_excerpt = "Concise progress updates are useful.".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&inferred).status,
            LearningCandidateStatus::Pending
        );
    }

    #[test]
    fn project_scoped_preferences_are_rejected_as_mismatched() {
        let mut project = proposal();
        project.scope = LearningScope::Project;
        assert_eq!(
            LearningPolicy::evaluate(&project).status,
            LearningCandidateStatus::Rejected
        );
    }

    #[test]
    fn kind_scope_and_key_prefix_must_align() {
        let mut correction = proposal();
        correction.kind = LearningKind::UserCorrection;
        assert_eq!(
            LearningPolicy::evaluate(&correction).status,
            LearningCandidateStatus::Rejected
        );
        correction.canonical_key = "correction.progress_updates".to_string();
        correction.evidence_excerpt = "Actually, keep progress updates concise.".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&correction).status,
            LearningCandidateStatus::AutoAccepted
        );

        let mut protected = proposal();
        protected.canonical_key = "identity.display_name".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&protected).status,
            LearningCandidateStatus::Rejected
        );
    }

    #[test]
    fn common_secret_signatures_are_rejected() {
        for secret in [
            "github_pat_example",
            "ghp_example",
            "xoxb-example",
            "sk-example",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
        ] {
            let mut sensitive = proposal();
            sensitive.content = format!("Remember {secret}");
            assert_eq!(
                LearningPolicy::evaluate(&sensitive).status,
                LearningCandidateStatus::Rejected
            );
        }

        let mut ordinary_hyphenated_word = proposal();
        ordinary_hyphenated_word.content =
            "The user prefers task-based progress updates.".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&ordinary_hyphenated_word).status,
            LearningCandidateStatus::AutoAccepted
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
        forget.evidence_excerpt = "Please forget that preference.".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&forget).status,
            LearningCandidateStatus::Tombstoned
        );
    }

    #[test]
    fn inferred_forget_stays_pending() {
        let mut forget = proposal();
        forget.kind = LearningKind::Forget;
        forget.explicit = false;
        forget.evidence_excerpt = "Maybe that preference no longer applies.".to_string();
        assert_eq!(
            LearningPolicy::evaluate(&forget).status,
            LearningCandidateStatus::Pending
        );
    }
}
