use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MobileDiagnosticRun {
    pub id: String,
    pub user_id: Option<String>,
    pub installation_id: String,
    pub app_version: String,
    pub build_number: String,
    pub platform: String,
    pub os_version: String,
    pub device_class: String,
    pub capture_level: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub status: String,
    pub event_count: usize,
    pub dropped_event_count: usize,
    pub byte_count: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MobileDiagnosticEvent {
    pub run_id: String,
    pub sequence: u64,
    pub occurred_at_ms: i64,
    pub monotonic_ms: f64,
    pub category: String,
    pub name: String,
    pub duration_ms: Option<f64>,
    pub severity: String,
    pub attributes: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct MobileDiagnosticRunInput<'a> {
    pub id: &'a str,
    pub user_id: Option<&'a str>,
    pub installation_id: &'a str,
    pub app_version: &'a str,
    pub build_number: &'a str,
    pub platform: &'a str,
    pub os_version: &'a str,
    pub device_class: &'a str,
    pub capture_level: &'a str,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub completed: bool,
    pub dropped_event_count: usize,
}

#[derive(Debug, Clone)]
pub struct MobileDiagnosticEventInput<'a> {
    pub sequence: u64,
    pub occurred_at_ms: i64,
    pub monotonic_ms: f64,
    pub category: &'a str,
    pub name: &'a str,
    pub duration_ms: Option<f64>,
    pub severity: &'a str,
    pub attributes_json: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MobileDiagnosticNativePayload {
    pub run_id: String,
    pub payload_id: String,
    pub kind: String,
    pub received_at_ms: i64,
    pub payload: serde_json::Value,
    pub byte_count: usize,
}

#[derive(Debug, Clone)]
pub struct MobileDiagnosticNativePayloadInput<'a> {
    pub payload_id: &'a str,
    pub kind: &'a str,
    pub received_at_ms: i64,
    pub payload_json: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MobileDiagnosticCategoryCount {
    pub category: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MobileDiagnosticReport {
    pub run: MobileDiagnosticRun,
    pub categories: Vec<MobileDiagnosticCategoryCount>,
    pub long_task_count: usize,
    pub max_long_task_ms: Option<f64>,
    pub heartbeat_stall_count: usize,
    pub max_heartbeat_drift_ms: Option<f64>,
    pub webview_termination_count: usize,
    pub error_count: usize,
    pub native_payload_count: usize,
    pub recent_events: Vec<MobileDiagnosticEvent>,
}
