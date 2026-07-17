use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningKind {
    UserPreference,
    UserCorrection,
    ProjectFact,
    Procedure,
    RelationshipContext,
    Forget,
}

impl fmt::Display for LearningKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UserPreference => "user_preference",
            Self::UserCorrection => "user_correction",
            Self::ProjectFact => "project_fact",
            Self::Procedure => "procedure",
            Self::RelationshipContext => "relationship_context",
            Self::Forget => "forget",
        })
    }
}

impl FromStr for LearningKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user_preference" => Ok(Self::UserPreference),
            "user_correction" => Ok(Self::UserCorrection),
            "project_fact" => Ok(Self::ProjectFact),
            "procedure" => Ok(Self::Procedure),
            "relationship_context" => Ok(Self::RelationshipContext),
            "forget" => Ok(Self::Forget),
            other => Err(format!("unknown learning kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningSensitivity {
    Normal,
    Sensitive,
    Prohibited,
}

impl fmt::Display for LearningSensitivity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Normal => "normal",
            Self::Sensitive => "sensitive",
            Self::Prohibited => "prohibited",
        })
    }
}

impl FromStr for LearningSensitivity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "normal" => Ok(Self::Normal),
            "sensitive" => Ok(Self::Sensitive),
            "prohibited" => Ok(Self::Prohibited),
            other => Err(format!("unknown learning sensitivity: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningCandidateStatus {
    Pending,
    Accepted,
    AutoAccepted,
    Rejected,
    Tombstoned,
}

impl fmt::Display for LearningCandidateStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::AutoAccepted => "auto_accepted",
            Self::Rejected => "rejected",
            Self::Tombstoned => "tombstoned",
        })
    }
}

impl FromStr for LearningCandidateStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "auto_accepted" => Ok(Self::AutoAccepted),
            "rejected" => Ok(Self::Rejected),
            "tombstoned" => Ok(Self::Tombstoned),
            other => Err(format!("unknown learning status: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningCandidateInput {
    pub user_id: Option<String>,
    pub project_dir: Option<String>,
    pub canonical_key: String,
    pub kind: LearningKind,
    pub proposed_content: String,
    pub evidence_session_id: String,
    pub evidence_message_id: i64,
    pub evidence_excerpt: String,
    pub explicit: bool,
    pub confidence: f64,
    pub sensitivity: LearningSensitivity,
    pub status: LearningCandidateStatus,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningCandidate {
    pub id: String,
    pub user_id: Option<String>,
    pub project_dir: Option<String>,
    pub canonical_key: String,
    pub kind: LearningKind,
    pub proposed_content: String,
    pub evidence_session_id: String,
    pub evidence_message_id: i64,
    pub evidence_excerpt: String,
    pub explicit: bool,
    pub confidence: f64,
    pub sensitivity: LearningSensitivity,
    pub status: LearningCandidateStatus,
    pub reason: String,
    pub created_at: String,
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningThroughState {
    pub session_id: String,
    pub through_message_id: i64,
    pub status: String,
    pub model: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}
