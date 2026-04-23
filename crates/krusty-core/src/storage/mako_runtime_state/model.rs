use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MakoRuntimeStateStatus {
    Idle,
    Running,
    Sleeping,
    AwaitingInput,
    Paused,
    Error,
    Cancelled,
}

impl MakoRuntimeStateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::AwaitingInput => "awaiting_input",
            Self::Paused => "paused",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "idle" => Some(Self::Idle),
            "running" => Some(Self::Running),
            "sleeping" => Some(Self::Sleeping),
            "awaiting_input" => Some(Self::AwaitingInput),
            "paused" => Some(Self::Paused),
            "error" => Some(Self::Error),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

impl std::fmt::Display for MakoRuntimeStateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MakoRunPriority {
    Low,
    Normal,
    High,
}

impl MakoRunPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

impl Default for MakoRunPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl std::fmt::Display for MakoRunPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MakoRuntimeState {
    pub session_id: String,
    pub status: MakoRuntimeStateStatus,
    pub next_wake_at: Option<String>,
    pub sleep_reason: Option<String>,
    pub last_error: Option<String>,
    pub current_run_id: Option<String>,
    pub last_wake_reason: Option<String>,
    pub crew_slug: Option<String>,
    pub priority: MakoRunPriority,
    pub updated_at: String,
}

impl MakoRuntimeState {
    pub(crate) fn new_empty(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            status: MakoRuntimeStateStatus::Idle,
            next_wake_at: None,
            sleep_reason: None,
            last_error: None,
            current_run_id: None,
            last_wake_reason: None,
            crew_slug: None,
            priority: MakoRunPriority::Normal,
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}
