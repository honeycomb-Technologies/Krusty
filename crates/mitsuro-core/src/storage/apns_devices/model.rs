use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApnsDevice {
    pub id: String,
    pub user_id: Option<String>,
    pub device_token: String,
    pub bundle_id: String,
    pub notification_level: String,
    pub environment: String,
    pub last_registered_at: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_failure_reason: Option<String>,
    pub failure_count: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct ApnsDeviceRegistration<'a> {
    pub user_id: Option<&'a str>,
    pub device_token: &'a str,
    pub bundle_id: &'a str,
    pub notification_level: &'a str,
    pub environment: &'a str,
}
