use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: DateTime<Utc>,
    pub token_count: Option<usize>,
    /// Parent session ID for linked sessions (pinch)
    pub parent_session_id: Option<String>,
    /// Working directory for this session
    pub working_dir: Option<String>,
    /// Explicit active project directory, when different from the session root.
    pub project_dir: Option<String>,
    /// Whether the session is operating without a project or within an explicit project.
    pub workspace_mode: WorkspaceMode,
    /// High-level session surface type.
    pub session_type: SessionType,
    /// User ID for multi-tenant isolation
    pub user_id: Option<String>,
    /// Current work mode for this session
    pub work_mode: WorkMode,
    /// Model selected for this session
    pub model: Option<String>,
    /// Optional target branch selected for this session
    pub target_branch: Option<String>,
}

/// Session type for high-level product surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionType {
    Chat,
    #[default]
    Code,
    Mako,
}

impl fmt::Display for SessionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionType::Chat => write!(f, "chat"),
            SessionType::Code => write!(f, "code"),
            SessionType::Mako => write!(f, "mako"),
        }
    }
}

impl FromStr for SessionType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "chat" => Ok(SessionType::Chat),
            "code" => Ok(SessionType::Code),
            "mako" => Ok(SessionType::Mako),
            other => Err(format!("Unknown session type: {}", other)),
        }
    }
}

/// Session workspace mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    #[default]
    Neutral,
    Selected,
    Created,
}

impl fmt::Display for WorkspaceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceMode::Neutral => write!(f, "neutral"),
            WorkspaceMode::Selected => write!(f, "selected"),
            WorkspaceMode::Created => write!(f, "created"),
        }
    }
}

impl FromStr for WorkspaceMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "neutral" => Ok(WorkspaceMode::Neutral),
            "selected" => Ok(WorkspaceMode::Selected),
            "created" => Ok(WorkspaceMode::Created),
            other => Err(format!("Unknown workspace mode: {}", other)),
        }
    }
}

/// Session work mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkMode {
    #[default]
    Build,
    Plan,
}

impl fmt::Display for WorkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkMode::Build => write!(f, "build"),
            WorkMode::Plan => write!(f, "plan"),
        }
    }
}

impl FromStr for WorkMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "build" => Ok(WorkMode::Build),
            "plan" => Ok(WorkMode::Plan),
            other => Err(format!("Unknown work mode: {}", other)),
        }
    }
}
