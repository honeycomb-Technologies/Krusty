use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApnsDevice {
    pub id: String,
    pub user_id: Option<String>,
    pub device_token: String,
    pub bundle_id: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_failure_reason: Option<String>,
    pub failure_count: i64,
}
