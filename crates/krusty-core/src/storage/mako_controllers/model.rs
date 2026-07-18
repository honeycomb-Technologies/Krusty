use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakoControllerStatus {
    Active,
    Paused,
    Disabled,
}

impl MakoControllerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl std::fmt::Display for MakoControllerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MakoController {
    pub id: String,
    pub scope_key: String,
    pub user_id: Option<String>,
    pub session_id: String,
    pub status: MakoControllerStatus,
    pub timezone: String,
    pub max_concurrent_runs: u32,
    pub created_at: String,
    pub updated_at: String,
}
