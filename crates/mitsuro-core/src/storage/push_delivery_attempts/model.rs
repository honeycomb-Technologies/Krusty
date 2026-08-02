use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushDeliveryAttempt {
    pub id: String,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub endpoint_hash: String,
    pub provider_host: String,
    pub event_type: String,
    pub outcome: String,
    pub http_status: Option<i64>,
    pub error_message: Option<String>,
    pub latency_ms: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PushDeliverySummary {
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_failure_reason: Option<String>,
    pub recent_failures_24h: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PushDeliveryAttemptInput<'a> {
    pub user_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub endpoint: &'a str,
    pub event_type: &'a str,
    pub outcome: &'a str,
    pub http_status: Option<u16>,
    pub error_message: Option<&'a str>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AttemptOutcomeFilter {
    Any,
    Success,
    Failure,
}
