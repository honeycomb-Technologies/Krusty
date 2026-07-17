use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mako::MakoRunStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakoRunKind {
    Dispatch,
    Scheduled,
    ControllerChild,
    LegacyResume,
}

impl MakoRunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Scheduled => "scheduled",
            Self::ControllerChild => "controller_child",
            Self::LegacyResume => "legacy_resume",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "dispatch" => Some(Self::Dispatch),
            "scheduled" => Some(Self::Scheduled),
            "controller_child" => Some(Self::ControllerChild),
            "legacy_resume" => Some(Self::LegacyResume),
            _ => None,
        }
    }
}

impl std::fmt::Display for MakoRunKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakoRun {
    pub id: String,
    pub controller_id: String,
    pub session_id: Option<String>,
    pub schedule_id: Option<String>,
    pub occurrence_id: Option<String>,
    pub kind: MakoRunKind,
    pub objective: String,
    pub config: Value,
    pub status: MakoRunStatus,
    pub priority: i32,
    pub concurrency_key: Option<String>,
    pub scheduled_for: Option<String>,
    pub available_at: String,
    pub wake_at: Option<String>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub lease_owner: Option<String>,
    pub lease_token: Option<String>,
    pub lease_epoch: Option<u64>,
    pub lease_expires_at: Option<String>,
    pub heartbeat_at: Option<String>,
    pub last_stop_reason: Option<String>,
    pub last_error: Option<String>,
    pub outcome: Option<Value>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakoRunAttemptOutcome {
    Leased,
    Succeeded,
    Failed,
    RetryScheduled,
    Sleeping,
    AwaitingInput,
    RecoveryRequired,
    Cancelled,
    Abandoned,
    DeadLetter,
}

impl MakoRunAttemptOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Leased => "leased",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::RetryScheduled => "retry_scheduled",
            Self::Sleeping => "sleeping",
            Self::AwaitingInput => "awaiting_input",
            Self::RecoveryRequired => "recovery_required",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
            Self::DeadLetter => "dead_letter",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "leased" => Some(Self::Leased),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "retry_scheduled" => Some(Self::RetryScheduled),
            "sleeping" => Some(Self::Sleeping),
            "awaiting_input" => Some(Self::AwaitingInput),
            "recovery_required" => Some(Self::RecoveryRequired),
            "cancelled" => Some(Self::Cancelled),
            "abandoned" => Some(Self::Abandoned),
            "dead_letter" => Some(Self::DeadLetter),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MakoRunAttempt {
    pub id: String,
    pub run_id: String,
    pub attempt_no: u32,
    pub worker_id: String,
    pub lease_token: String,
    pub lease_epoch: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub outcome: MakoRunAttemptOutcome,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    pub retry_at: Option<String>,
    pub trace_sequence_start: Option<i64>,
    pub trace_sequence_end: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ClaimRunRequest {
    pub worker_id: String,
    pub lease_epoch: u64,
    pub now: DateTime<Utc>,
    pub lease_duration: Duration,
    pub global_concurrency_limit: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedMakoRun {
    pub run: MakoRun,
    pub attempt_id: String,
    pub attempt_no: u32,
    pub lease_token: String,
}

#[derive(Debug, Clone)]
pub struct RunCompletion {
    pub target_status: MakoRunStatus,
    pub now: DateTime<Utc>,
    pub available_at: Option<DateTime<Utc>>,
    pub wake_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    pub outcome: Option<Value>,
    pub trace_sequence_end: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LeaseReconciliation {
    pub requeued_unstarted: usize,
    pub recovery_required: usize,
}
