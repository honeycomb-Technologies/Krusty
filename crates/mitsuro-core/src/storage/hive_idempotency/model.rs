use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    pub scope_key: String,
    pub operation: String,
    pub key: String,
    pub request_hash: String,
    pub resource_id: Option<String>,
    pub response: Option<Value>,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdempotencyClaim {
    /// This caller owns execution and may complete the key.
    Claimed(IdempotencyRecord),
    /// An equivalent request is currently executing.
    InProgress(IdempotencyRecord),
    /// An equivalent request already completed; replay this response.
    Replay(IdempotencyRecord),
    /// The key was reused for a different logical request.
    Conflict { existing_request_hash: String },
}
