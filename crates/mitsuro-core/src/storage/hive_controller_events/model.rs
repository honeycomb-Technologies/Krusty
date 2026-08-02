use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveControllerEventType {
    ControllerCreated,
    ControllerStarted,
    ControllerPaused,
    ControllerDisabled,
    ScheduleCreated,
    ScheduleUpdated,
    SchedulePaused,
    ScheduleResumed,
    ScheduleCancelled,
    OccurrenceMaterialized,
    OccurrenceSkipped,
    RunQueued,
    RunLeased,
    RunStarted,
    RunSleeping,
    RunRetryScheduled,
    RunAwaitingInput,
    RunCompleted,
    RunFailed,
    RunCancelled,
    RunDeadLettered,
    RecoveryRequired,
}

impl HiveControllerEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ControllerCreated => "controller_created",
            Self::ControllerStarted => "controller_started",
            Self::ControllerPaused => "controller_paused",
            Self::ControllerDisabled => "controller_disabled",
            Self::ScheduleCreated => "schedule_created",
            Self::ScheduleUpdated => "schedule_updated",
            Self::SchedulePaused => "schedule_paused",
            Self::ScheduleResumed => "schedule_resumed",
            Self::ScheduleCancelled => "schedule_cancelled",
            Self::OccurrenceMaterialized => "occurrence_materialized",
            Self::OccurrenceSkipped => "occurrence_skipped",
            Self::RunQueued => "run_queued",
            Self::RunLeased => "run_leased",
            Self::RunStarted => "run_started",
            Self::RunSleeping => "run_sleeping",
            Self::RunRetryScheduled => "run_retry_scheduled",
            Self::RunAwaitingInput => "run_awaiting_input",
            Self::RunCompleted => "run_completed",
            Self::RunFailed => "run_failed",
            Self::RunCancelled => "run_cancelled",
            Self::RunDeadLettered => "run_dead_lettered",
            Self::RecoveryRequired => "recovery_required",
        }
    }
}

impl std::fmt::Display for HiveControllerEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewHiveControllerEvent {
    pub controller_id: String,
    pub event_type: HiveControllerEventType,
    pub run_id: Option<String>,
    pub schedule_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveControllerEvent {
    pub id: i64,
    pub controller_id: String,
    pub sequence: u64,
    /// Durable runtime event name. Unlike write-side controller lifecycle
    /// events, execution extensions are intentionally open-ended, so reads
    /// must preserve unknown names instead of failing deserialization.
    pub event_type: String,
    pub run_id: Option<String>,
    pub schedule_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
    pub created_at: String,
}
