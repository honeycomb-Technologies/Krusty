use serde::{Deserialize, Serialize};

use crate::ai::models::ModelKey;
use crate::tools::registry::PermissionMode;

/// Lifecycle status of a Hive Worker identity.
///
/// Archiving frees the Worker's slug for reuse (the active-scope unique index
/// ignores archived rows) without destroying its history or documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HiveWorkerStatus {
    #[default]
    Active,
    Paused,
    Archived,
}

impl HiveWorkerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Archived => "archived",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

impl std::fmt::Display for HiveWorkerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a Worker wakes up outside direct user messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HiveWorkerAutonomy {
    #[default]
    Manual,
    Scheduled,
    AlwaysOn,
}

impl HiveWorkerAutonomy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::AlwaysOn => "always_on",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(Self::Manual),
            "scheduled" => Some(Self::Scheduled),
            "always_on" => Some(Self::AlwaysOn),
            _ => None,
        }
    }
}

impl std::fmt::Display for HiveWorkerAutonomy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Persona documents attached to one Worker, mirroring the crew document
/// kinds so backfilled crew content keeps its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveWorkerDocumentKind {
    Identity,
    Soul,
}

impl HiveWorkerDocumentKind {
    pub const ALL: [Self; 2] = [Self::Identity, Self::Soul];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Soul => "soul",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "identity" => Some(Self::Identity),
            "soul" => Some(Self::Soul),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveWorkerDocument {
    pub kind: HiveWorkerDocumentKind,
    pub content: String,
    pub updated_at: String,
}

/// A durable Hive Worker identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveWorker {
    pub id: String,
    /// Exact owner; NULL means the local single-tenant profile.
    pub user_id: Option<String>,
    pub slug: String,
    pub display_name: String,
    pub avatar_color: Option<String>,
    pub model: Option<String>,
    /// Exact provider/auth/transport identity frozen for this Worker.
    #[serde(default)]
    pub model_key: Option<ModelKey>,
    /// Catalog revision observed when `model_key` was selected.
    #[serde(default)]
    pub model_catalog_revision: Option<String>,
    pub permission_mode: PermissionMode,
    pub autonomy: HiveWorkerAutonomy,
    pub heartbeat_interval_secs: Option<u32>,
    pub status: HiveWorkerStatus,
    /// The Worker's private DM session; its controller is the Worker's
    /// serialized execution lane.
    pub dm_session_id: Option<String>,
    /// Memory namespace id; defaults to the slug so backfilled crew
    /// memories (namespace 'crew', namespace_id = slug) stay reachable.
    pub memory_namespace_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating a Worker. `display_name` defaults to a titled form of
/// the slug and `memory_namespace_id` defaults to the slug for crew
/// compatibility.
#[derive(Debug, Clone, Default)]
pub struct NewHiveWorker {
    pub user_id: Option<String>,
    pub slug: String,
    pub display_name: Option<String>,
    pub avatar_color: Option<String>,
    pub model: Option<String>,
    pub model_key: Option<ModelKey>,
    pub model_catalog_revision: Option<String>,
    pub permission_mode: PermissionMode,
    pub autonomy: HiveWorkerAutonomy,
    pub heartbeat_interval_secs: Option<u32>,
    pub dm_session_id: Option<String>,
    pub memory_namespace_id: Option<String>,
}

impl NewHiveWorker {
    pub fn new(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            ..Self::default()
        }
    }
}

/// Full overwrite of the profile-editable surface of a Worker. Callers load
/// the current Worker, adjust fields, and submit the complete set; identity
/// fields (id, owner, slug, status, autonomy, DM binding) are updated through
/// their dedicated store methods.
#[derive(Debug, Clone)]
pub struct HiveWorkerProfileUpdate {
    pub display_name: String,
    pub avatar_color: Option<String>,
    pub model: Option<String>,
    pub model_key: Option<ModelKey>,
    pub model_catalog_revision: Option<String>,
    pub permission_mode: PermissionMode,
}

/// Derive a human-friendly default display name from a Worker slug
/// (`code-reviewer` becomes `Code Reviewer`).
pub fn display_name_from_slug(slug: &str) -> String {
    let mut display_name = String::with_capacity(slug.len());
    for word in slug.split(['-', '_']).filter(|word| !word.is_empty()) {
        if !display_name.is_empty() {
            display_name.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            display_name.extend(first.to_uppercase());
            display_name.push_str(chars.as_str());
        }
    }
    if display_name.is_empty() {
        slug.to_string()
    } else {
        display_name
    }
}
