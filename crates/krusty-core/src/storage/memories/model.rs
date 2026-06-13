use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Role, expertise, preferences, goals
    User,
    /// Corrections and confirmed approaches
    Feedback,
    /// Ongoing work, deadlines, decisions
    Project,
    /// External system pointers (issue trackers, dashboards)
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "feedback" => Ok(Self::Feedback),
            "project" => Ok(Self::Project),
            "reference" => Ok(Self::Reference),
            _ => Err(format!("Unknown memory type: {s}")),
        }
    }
}

pub const COMPACTION_FLUSH_TITLE_PREFIX: &str = "Compaction flush #";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMemory {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub project_dir: Option<String>,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn is_compaction_flush_memory(memory: &AgentMemory) -> bool {
    memory.memory_type == MemoryType::Project
        && memory.title.starts_with(COMPACTION_FLUSH_TITLE_PREFIX)
}
