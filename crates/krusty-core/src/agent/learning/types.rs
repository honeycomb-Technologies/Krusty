use serde::{Deserialize, Serialize};

use crate::storage::{LearningKind, LearningSensitivity};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningProposal {
    pub canonical_key: String,
    pub kind: LearningKind,
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
