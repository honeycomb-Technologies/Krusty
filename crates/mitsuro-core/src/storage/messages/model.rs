use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessageRecord {
    pub id: i64,
    pub role: String,
    pub content_json: String,
    pub created_at: String,
}
