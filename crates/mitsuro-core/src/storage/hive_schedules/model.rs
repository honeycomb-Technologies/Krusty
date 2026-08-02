use serde::{Deserialize, Serialize};

use crate::ai::models::ModelKey;
use crate::hive::{DstPolicy, MisfireConfig, RecurrenceV1, RetryPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveScheduleStatus {
    Enabled,
    Paused,
    Completed,
    Cancelled,
}

impl HiveScheduleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "enabled" => Some(Self::Enabled),
            "paused" => Some(Self::Paused),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (
                    Self::Enabled,
                    Self::Paused | Self::Completed | Self::Cancelled
                ) | (Self::Paused, Self::Enabled | Self::Cancelled)
            )
    }
}

impl std::fmt::Display for HiveScheduleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    Skip,
    QueueOne,
    Allow,
}

impl OverlapPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::QueueOne => "queue_one",
            Self::Allow => "allow",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "skip" => Some(Self::Skip),
            "queue_one" => Some(Self::QueueOne),
            "allow" => Some(Self::Allow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveSchedule {
    pub id: String,
    pub controller_id: String,
    pub title: String,
    pub summary: String,
    pub objective: String,
    pub recurrence: RecurrenceV1,
    pub timezone: String,
    pub dst_policy: DstPolicy,
    pub next_fire_at: Option<String>,
    pub last_scheduled_for: Option<String>,
    pub status: HiveScheduleStatus,
    pub priority: i32,
    pub project_dir: Option<String>,
    pub model: Option<String>,
    /// Exact provider/auth/transport identity frozen for future occurrences.
    #[serde(default)]
    pub model_key: Option<ModelKey>,
    /// Catalog revision observed when `model_key` was selected.
    #[serde(default)]
    pub model_catalog_revision: Option<String>,
    pub crew_slug: Option<String>,
    pub misfire: MisfireConfig,
    pub overlap_policy: OverlapPolicy,
    pub retry: RetryPolicy,
    pub revision: u64,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A user-scoped schedule plus the owning Hive session required by mutation
/// routes. The controller UUID alone is intentionally not a routing key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnedHiveSchedule {
    #[serde(flatten)]
    pub schedule: HiveSchedule,
    pub controller_session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveScheduleOccurrenceStatus {
    Pending,
    Queued,
    Skipped,
    Coalesced,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl HiveScheduleOccurrenceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Queued => "queued",
            Self::Skipped => "skipped",
            Self::Coalesced => "coalesced",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "queued" => Some(Self::Queued),
            "skipped" => Some(Self::Skipped),
            "coalesced" => Some(Self::Coalesced),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

impl std::fmt::Display for HiveScheduleOccurrenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveScheduleOccurrence {
    pub id: String,
    pub schedule_id: String,
    pub scheduled_for: String,
    pub run_id: Option<String>,
    pub status: HiveScheduleOccurrenceStatus,
    pub decision_reason: Option<String>,
    pub coalesced_count: u32,
    pub created_at: String,
    pub updated_at: String,
}
