use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerConversationInputState {
    Staged,
    Materialized,
}

impl WorkerConversationInputState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Materialized => "materialized",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "staged" => Some(Self::Staged),
            "materialized" => Some(Self::Materialized),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerConversationInput {
    pub id: String,
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub request_id: String,
    pub accepted_while_run_id: String,
    pub body: String,
    pub state: WorkerConversationInputState,
    pub canonical_message_id: Option<i64>,
    pub assigned_run_id: Option<String>,
    pub accepted_at: String,
    pub materialized_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StageWorkerConversationInput {
    pub id: String,
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub request_id: String,
    pub accepted_while_run_id: String,
    pub body: String,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageWorkerConversationInputResult {
    Inserted(WorkerConversationInput),
    Existing(WorkerConversationInput),
}
