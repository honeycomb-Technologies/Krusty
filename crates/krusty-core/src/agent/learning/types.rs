use serde::{Deserialize, Serialize};

use crate::storage::{LearningKind, LearningSensitivity};

/// The exact persistence scope proposed by the restricted reviewer.
///
/// This is deliberately small: the reviewer cannot target another user,
/// another project, a crew identity, or any filesystem-backed profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningScope {
    User,
    Project,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningProposal {
    pub canonical_key: String,
    pub kind: LearningKind,
    pub scope: LearningScope,
    pub content: String,
    pub evidence_message_id: i64,
    pub evidence_excerpt: String,
    pub explicit: bool,
    pub confidence: f64,
    pub sensitivity: LearningSensitivity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningReviewerOutput {
    #[serde(default)]
    pub proposals: Vec<LearningProposal>,
}
