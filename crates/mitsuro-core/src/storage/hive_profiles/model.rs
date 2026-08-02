use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::paths;

const LOCAL_PROFILE_ID: &str = "local";

/// Stable owner of one Hive identity profile.
///
/// User profile ids are derived from the complete user id. Hashing keeps raw
/// external identifiers out of internal primary keys while remaining stable
/// across workspaces, server restarts, and future daemon extraction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HiveProfileOwner {
    Local,
    User(String),
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum HiveProfileOwnerError {
    #[error("Hive profile user id must not be empty")]
    EmptyUserId,
}

impl HiveProfileOwner {
    pub fn local() -> Self {
        Self::Local
    }

    pub fn user(user_id: impl Into<String>) -> Result<Self, HiveProfileOwnerError> {
        let user_id = user_id.into();
        let trimmed = user_id.trim();
        if trimmed.is_empty() {
            return Err(HiveProfileOwnerError::EmptyUserId);
        }
        Ok(Self::User(trimmed.to_string()))
    }

    pub fn from_user_id(user_id: Option<&str>) -> Result<Self, HiveProfileOwnerError> {
        match user_id {
            Some(user_id) => Self::user(user_id),
            None => Ok(Self::Local),
        }
    }

    pub fn profile_id(&self) -> String {
        match self {
            Self::Local => LOCAL_PROFILE_ID.to_string(),
            Self::User(user_id) => {
                let digest = Sha256::digest(user_id.as_bytes());
                format!("user:{digest:x}")
            }
        }
    }

    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::Local => None,
            Self::User(user_id) => Some(user_id),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HiveProfileDocumentKind {
    Soul,
    Identity,
    User,
    Heartbeat,
    Channels,
}

impl HiveProfileDocumentKind {
    pub const ALL: [Self; 5] = [
        Self::Soul,
        Self::Identity,
        Self::User,
        Self::Heartbeat,
        Self::Channels,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Soul => "soul",
            Self::Identity => "identity",
            Self::User => "user",
            Self::Heartbeat => "heartbeat",
            Self::Channels => "channels",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "soul" => Some(Self::Soul),
            "identity" => Some(Self::Identity),
            "user" => Some(Self::User),
            "heartbeat" => Some(Self::Heartbeat),
            "channels" => Some(Self::Channels),
            _ => None,
        }
    }

    pub fn preferred_file_name(self) -> &'static str {
        match self {
            Self::Soul => paths::HIVE_SOUL_FILE,
            Self::Identity => paths::HIVE_IDENTITY_FILE,
            Self::User => paths::HIVE_USER_FILE,
            Self::Heartbeat => paths::HIVE_HEARTBEAT_FILE,
            Self::Channels => paths::HIVE_CHANNELS_FILE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HiveCrewProfileDocumentKind {
    Identity,
    Soul,
}

impl HiveCrewProfileDocumentKind {
    pub const ALL: [Self; 2] = [Self::Identity, Self::Soul];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Soul => "soul",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "identity" => Some(Self::Identity),
            "soul" => Some(Self::Soul),
            _ => None,
        }
    }

    pub fn preferred_file_name(self) -> &'static str {
        match self {
            Self::Identity => "IDENTITY.md",
            Self::Soul => "SOUL.md",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveProfileDocument<K> {
    pub kind: K,
    pub content: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiveCrewProfileSnapshot {
    pub slug: String,
    pub revision: i64,
    pub identity: Option<HiveProfileDocument<HiveCrewProfileDocumentKind>>,
    pub soul: Option<HiveProfileDocument<HiveCrewProfileDocumentKind>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveProfileSnapshot {
    pub profile_id: String,
    pub user_id: Option<String>,
    pub revision: i64,
    pub soul: Option<HiveProfileDocument<HiveProfileDocumentKind>>,
    pub identity: Option<HiveProfileDocument<HiveProfileDocumentKind>>,
    pub user: Option<HiveProfileDocument<HiveProfileDocumentKind>>,
    pub heartbeat: Option<HiveProfileDocument<HiveProfileDocumentKind>>,
    pub channels: Option<HiveProfileDocument<HiveProfileDocumentKind>>,
    pub crew: Vec<HiveCrewProfileSnapshot>,
}

impl HiveProfileSnapshot {
    pub fn document(
        &self,
        kind: HiveProfileDocumentKind,
    ) -> Option<&HiveProfileDocument<HiveProfileDocumentKind>> {
        match kind {
            HiveProfileDocumentKind::Soul => self.soul.as_ref(),
            HiveProfileDocumentKind::Identity => self.identity.as_ref(),
            HiveProfileDocumentKind::User => self.user.as_ref(),
            HiveProfileDocumentKind::Heartbeat => self.heartbeat.as_ref(),
            HiveProfileDocumentKind::Channels => self.channels.as_ref(),
        }
    }

    pub fn crew_member(&self, slug: &str) -> Option<&HiveCrewProfileSnapshot> {
        self.crew.iter().find(|member| member.slug == slug)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveCrewProfileSeed {
    pub slug: String,
    pub documents: Vec<(HiveCrewProfileDocumentKind, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiveProfileSeed {
    pub documents: Vec<(HiveProfileDocumentKind, String)>,
    pub crew: Vec<HiveCrewProfileSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveProfileMergeResult {
    pub snapshot: HiveProfileSnapshot,
    pub inserted_documents: Vec<HiveProfileDocumentKind>,
    pub inserted_crew_documents: Vec<(String, HiveCrewProfileDocumentKind)>,
}
