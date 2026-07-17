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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryNamespace {
    /// Durable knowledge shared by the owner's Mako surfaces.
    #[default]
    Shared,
    /// Knowledge specific to the primary Mako presence.
    Mako,
    /// Knowledge specific to one named crew member.
    Crew,
}

impl MemoryNamespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Mako => "mako",
            Self::Crew => "crew",
        }
    }
}

impl FromStr for MemoryNamespace {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "shared" => Ok(Self::Shared),
            "mako" => Ok(Self::Mako),
            "crew" => Ok(Self::Crew),
            _ => Err(format!("Unknown memory namespace: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    #[default]
    Active,
    Superseded,
    Deleted,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Deleted => "deleted",
        }
    }

    pub fn is_active(self) -> bool {
        self == Self::Active
    }
}

impl FromStr for MemoryStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "superseded" => Ok(Self::Superseded),
            "deleted" => Ok(Self::Deleted),
            _ => Err(format!("Unknown memory status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    #[default]
    Legacy,
    User,
    Agent,
    Tool,
    Import,
    Compaction,
    System,
}

impl MemorySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::User => "user",
            Self::Agent => "agent",
            Self::Tool => "tool",
            Self::Import => "import",
            Self::Compaction => "compaction",
            Self::System => "system",
        }
    }
}

impl FromStr for MemorySource {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "legacy" => Ok(Self::Legacy),
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            "tool" => Ok(Self::Tool),
            "import" => Ok(Self::Import),
            "compaction" => Ok(Self::Compaction),
            "system" => Ok(Self::System),
            _ => Err(format!("Unknown memory source: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySensitivity {
    #[default]
    Normal,
    Sensitive,
    Secret,
}

impl MemorySensitivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Sensitive => "sensitive",
            Self::Secret => "secret",
        }
    }
}

impl FromStr for MemorySensitivity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "normal" => Ok(Self::Normal),
            "sensitive" => Ok(Self::Sensitive),
            "secret" => Ok(Self::Secret),
            _ => Err(format!("Unknown memory sensitivity: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRevisionEvent {
    Created,
    Updated,
    Superseded,
    Deleted,
}

impl MemoryRevisionEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Superseded => "superseded",
            Self::Deleted => "deleted",
        }
    }
}

impl FromStr for MemoryRevisionEvent {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "created" => Ok(Self::Created),
            "updated" => Ok(Self::Updated),
            "superseded" => Ok(Self::Superseded),
            "deleted" => Ok(Self::Deleted),
            _ => Err(format!("Unknown memory revision event: {value}")),
        }
    }
}

pub const COMPACTION_FLUSH_TITLE_PREFIX: &str = "Compaction flush #";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMemory {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub project_dir: Option<String>,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub canonical_key: Option<String>,
    #[serde(default)]
    pub namespace: MemoryNamespace,
    #[serde(default)]
    pub namespace_id: Option<String>,
    #[serde(default)]
    pub status: MemoryStatus,
    #[serde(default)]
    pub source: MemorySource,
    #[serde(default)]
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub source_message_id: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub sensitivity: MemorySensitivity,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub supersedes_id: Option<String>,
    #[serde(default)]
    pub last_accessed_at: Option<String>,
    #[serde(default)]
    pub access_count: i64,
}

fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalMemoryInput {
    pub memory_type: MemoryType,
    pub canonical_key: String,
    pub title: String,
    pub content: String,
    pub project_dir: Option<String>,
    pub user_id: Option<String>,
    pub namespace: MemoryNamespace,
    pub namespace_id: Option<String>,
    pub source: MemorySource,
    pub source_session_id: Option<String>,
    pub source_message_id: Option<String>,
    pub confidence: f64,
    pub sensitivity: MemorySensitivity,
    pub pinned: bool,
}

impl CanonicalMemoryInput {
    pub fn new(
        memory_type: MemoryType,
        canonical_key: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            memory_type,
            canonical_key: canonical_key.into(),
            title: title.into(),
            content: content.into(),
            project_dir: None,
            user_id: None,
            namespace: MemoryNamespace::Shared,
            namespace_id: None,
            source: MemorySource::Agent,
            source_session_id: None,
            source_message_id: None,
            confidence: default_confidence(),
            sensitivity: MemorySensitivity::Normal,
            pinned: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMemoryRevision {
    pub id: String,
    pub memory_id: String,
    pub revision: i64,
    pub event: MemoryRevisionEvent,
    pub snapshot: AgentMemory,
    pub created_at: String,
}

pub fn is_compaction_flush_memory(memory: &AgentMemory) -> bool {
    memory.memory_type == MemoryType::Project
        && memory.title.starts_with(COMPACTION_FLUSH_TITLE_PREFIX)
}
