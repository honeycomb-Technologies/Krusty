use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakoControllerEventType {
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

impl MakoControllerEventType {
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

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "controller_created" => Some(Self::ControllerCreated),
            "controller_started" => Some(Self::ControllerStarted),
            "controller_paused" => Some(Self::ControllerPaused),
            "controller_disabled" => Some(Self::ControllerDisabled),
            "schedule_created" => Some(Self::ScheduleCreated),
            "schedule_updated" => Some(Self::ScheduleUpdated),
            "schedule_paused" => Some(Self::SchedulePaused),
            "schedule_resumed" => Some(Self::ScheduleResumed),
            "schedule_cancelled" => Some(Self::ScheduleCancelled),
            "occurrence_materialized" => Some(Self::OccurrenceMaterialized),
            "occurrence_skipped" => Some(Self::OccurrenceSkipped),
            "run_queued" => Some(Self::RunQueued),
            "run_leased" => Some(Self::RunLeased),
            "run_started" => Some(Self::RunStarted),
            "run_sleeping" => Some(Self::RunSleeping),
            "run_retry_scheduled" => Some(Self::RunRetryScheduled),
            "run_awaiting_input" => Some(Self::RunAwaitingInput),
            "run_completed" => Some(Self::RunCompleted),
            "run_failed" => Some(Self::RunFailed),
            "run_cancelled" => Some(Self::RunCancelled),
            "run_dead_lettered" => Some(Self::RunDeadLettered),
            "recovery_required" => Some(Self::RecoveryRequired),
            _ => None,
        }
    }
}

impl std::fmt::Display for MakoControllerEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMakoControllerEvent {
    pub controller_id: String,
    pub event_type: MakoControllerEventType,
    pub run_id: Option<String>,
    pub schedule_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakoControllerEvent {
    pub id: i64,
    pub controller_id: String,
    pub sequence: u64,
    pub event_type: MakoControllerEventType,
    pub run_id: Option<String>,
    pub schedule_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub payload: Value,
    pub created_at: String,
}
