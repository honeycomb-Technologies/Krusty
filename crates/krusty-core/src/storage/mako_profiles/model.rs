use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::paths;

const LOCAL_PROFILE_ID: &str = "local";

/// Stable owner of one Mako identity profile.
///
/// User profile ids are derived from the complete user id. Hashing keeps raw
/// external identifiers out of internal primary keys while remaining stable
/// across workspaces, server restarts, and future daemon extraction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MakoProfileOwner {
    Local,
    User(String),
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MakoProfileOwnerError {
    #[error("Mako profile user id must not be empty")]
    EmptyUserId,
}

impl MakoProfileOwner {
    pub fn local() -> Self {
        Self::Local
    }

    pub fn user(user_id: impl Into<String>) -> Result<Self, MakoProfileOwnerError> {
        let user_id = user_id.into();
        let trimmed = user_id.trim();
        if trimmed.is_empty() {
            return Err(MakoProfileOwnerError::EmptyUserId);
        }
        Ok(Self::User(trimmed.to_string()))
    }

    pub fn from_user_id(user_id: Option<&str>) -> Result<Self, MakoProfileOwnerError> {
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
pub enum MakoProfileDocumentKind {
    Soul,
    Identity,
    User,
    Heartbeat,
    Channels,
}

impl MakoProfileDocumentKind {
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
            Self::Soul => paths::MAKO_SOUL_FILE,
            Self::Identity => paths::MAKO_IDENTITY_FILE,
            Self::User => paths::MAKO_USER_FILE,
            Self::Heartbeat => paths::MAKO_HEARTBEAT_FILE,
            Self::Channels => paths::MAKO_CHANNELS_FILE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MakoCrewProfileDocumentKind {
    Identity,
    Soul,
}

impl MakoCrewProfileDocumentKind {
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
pub struct MakoProfileDocument<K> {
    pub kind: K,
    pub content: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MakoCrewProfileSnapshot {
    pub slug: String,
    pub revision: i64,
    pub identity: Option<MakoProfileDocument<MakoCrewProfileDocumentKind>>,
    pub soul: Option<MakoProfileDocument<MakoCrewProfileDocumentKind>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoProfileSnapshot {
    pub profile_id: String,
    pub user_id: Option<String>,
    pub revision: i64,
    pub soul: Option<MakoProfileDocument<MakoProfileDocumentKind>>,
    pub identity: Option<MakoProfileDocument<MakoProfileDocumentKind>>,
    pub user: Option<MakoProfileDocument<MakoProfileDocumentKind>>,
    pub heartbeat: Option<MakoProfileDocument<MakoProfileDocumentKind>>,
    pub channels: Option<MakoProfileDocument<MakoProfileDocumentKind>>,
    pub crew: Vec<MakoCrewProfileSnapshot>,
}

impl MakoProfileSnapshot {
    pub fn document(
        &self,
        kind: MakoProfileDocumentKind,
    ) -> Option<&MakoProfileDocument<MakoProfileDocumentKind>> {
        match kind {
            MakoProfileDocumentKind::Soul => self.soul.as_ref(),
            MakoProfileDocumentKind::Identity => self.identity.as_ref(),
            MakoProfileDocumentKind::User => self.user.as_ref(),
            MakoProfileDocumentKind::Heartbeat => self.heartbeat.as_ref(),
            MakoProfileDocumentKind::Channels => self.channels.as_ref(),
        }
    }

    pub fn crew_member(&self, slug: &str) -> Option<&MakoCrewProfileSnapshot> {
        self.crew.iter().find(|member| member.slug == slug)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoCrewProfileSeed {
    pub slug: String,
    pub documents: Vec<(MakoCrewProfileDocumentKind, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MakoProfileSeed {
    pub documents: Vec<(MakoProfileDocumentKind, String)>,
    pub crew: Vec<MakoCrewProfileSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoProfileMergeResult {
    pub snapshot: MakoProfileSnapshot,
    pub inserted_documents: Vec<MakoProfileDocumentKind>,
    pub inserted_crew_documents: Vec<(String, MakoCrewProfileDocumentKind)>,
}
