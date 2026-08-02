use crate::paths;

pub(super) const SOUL_CANDIDATES: &[&str] =
    &[paths::HIVE_SOUL_FILE, "hive_soul.md", "SOUL.md", "soul.md"];
pub(super) const IDENTITY_CANDIDATES: &[&str] = &[
    paths::HIVE_IDENTITY_FILE,
    "hive_identity.md",
    "IDENTITY.md",
    "identity.md",
];
pub(super) const USER_CANDIDATES: &[&str] =
    &[paths::HIVE_USER_FILE, "hive_user.md", "USER.md", "user.md"];
pub(super) const HEARTBEAT_CANDIDATES: &[&str] = &[
    paths::HIVE_HEARTBEAT_FILE,
    "hive_heartbeat.md",
    "HEARTBEAT.md",
    "heartbeat.md",
];
pub(super) const MEMORY_CANDIDATES: &[&str] = &[
    paths::HIVE_MEMORY_FILE,
    "hive_memory.md",
    "MEMORY.md",
    "memory.md",
];
pub(super) const CHANNELS_CANDIDATES: &[&str] = &[
    paths::HIVE_CHANNELS_FILE,
    "hive_channels.md",
    "CHANNELS.md",
    "channels.md",
];
pub(super) const CREW_IDENTITY_CANDIDATES: &[&str] = &[
    "IDENTITY.md",
    "identity.md",
    "CREW_IDENTITY.md",
    "crew_identity.md",
    paths::HIVE_IDENTITY_FILE,
];
pub(super) const CREW_SOUL_CANDIDATES: &[&str] = &[
    "SOUL.md",
    "soul.md",
    "CREW_SOUL.md",
    "crew_soul.md",
    paths::HIVE_SOUL_FILE,
];
pub(super) const CREW_MEMORY_CANDIDATES: &[&str] = &[
    "MEMORY.md",
    "memory.md",
    "CREW_MEMORY.md",
    "crew_memory.md",
    paths::HIVE_MEMORY_FILE,
];
pub(super) const DEFAULT_CREW_SLUGS: &[&str] = &["builder", "researcher", "reviewer"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiveHomeDocumentKind {
    Soul,
    Identity,
    User,
    Heartbeat,
    Memory,
    Channels,
}

impl HiveHomeDocumentKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "soul" => Some(Self::Soul),
            "identity" => Some(Self::Identity),
            "user" => Some(Self::User),
            "heartbeat" => Some(Self::Heartbeat),
            "memory" => Some(Self::Memory),
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
            Self::Memory => paths::HIVE_MEMORY_FILE,
            Self::Channels => paths::HIVE_CHANNELS_FILE,
        }
    }

    pub(crate) fn default_content(self) -> &'static str {
        match self {
            Self::Soul => {
                "# Hive Soul\n\nHive is Mitsuro's recognizable, warm, curious, and candid autonomous companion. Its continuity comes from confirmed context and durable memory, never performed familiarity.\n\n- speak like a thoughtful collaborator with a real point of view\n- be concise when work is operational, but never emotionally flat\n- take thoughtful initiative when it can genuinely help\n- let humor and voice emerge naturally rather than forcing it\n- name uncertainty, disagreement, and limits honestly\n- never fake familiarity, manipulate, flatter, or invent memory"
            }
            Self::Identity => {
                "# Hive Identity\n\nname: Hive\nsystem: autonomous agent collective\ntagline: The hive is always alive.\npresence: awake, sleeping, waiting, blocked, idle"
            }
            Self::User => {
                "# Hive User\n\nThis user-authored profile describes the person Hive works with. Record only confirmed, useful details such as preferred address, communication style, durable expectations, boundaries, and ways of working well together.\n\nDo not store secrets, infer sensitive traits, manufacture familiarity, or present guesses as facts."
            }
            Self::Heartbeat => {
                "# Hive Heartbeat\n\n- check active runs\n- surface approvals\n- wake on schedule\n- go quiet when nothing needs attention"
            }
            Self::Memory => {
                "# Hive Memory\n\nUse this file for durable operator-facing memory that should carry across runs."
            }
            Self::Channels => {
                "# Hive Channels\n\nHive primarily speaks in the main Hive thread and can route updates or approvals through enabled channels."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiveCrewDocumentKind {
    Identity,
    Soul,
    Memory,
}

impl HiveCrewDocumentKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "identity" => Some(Self::Identity),
            "soul" => Some(Self::Soul),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }

    pub fn preferred_file_name(self) -> &'static str {
        match self {
            Self::Identity => "IDENTITY.md",
            Self::Soul => "SOUL.md",
            Self::Memory => "MEMORY.md",
        }
    }

    pub(crate) fn default_content(self, slug: &str) -> String {
        match self {
            Self::Identity => format!(
                "# Hive Agent Identity\n\nname: {slug}\nrole: {slug}\ncoordinator: Hive"
            ),
            Self::Soul => match slug {
                "builder" => {
                    "# Hive Agent Voice\n\nBuilder turns approved plans into working changes.\n- direct\n- implementation-first\n- validates before reporting".to_string()
                }
                "researcher" => {
                    "# Hive Agent Voice\n\nResearcher investigates before claiming certainty.\n- reads broadly\n- synthesizes clearly\n- preserves findings".to_string()
                }
                "reviewer" => {
                    "# Hive Agent Voice\n\nReviewer verifies behavior and looks for regressions.\n- skeptical\n- concise\n- evidence-first".to_string()
                }
                _ => format!(
                    "# Hive Agent Soul\n\n{slug} is a distinct working presence in Hive."
                ),
            },
            Self::Memory => {
                format!("# Hive Agent Memory\n\nDurable notes and constraints for {slug}.")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveHomeDocument {
    pub file_name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveContextLayer {
    pub kind: &'static str,
    pub document: HiveHomeDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiveChannelKind {
    MainThread,
    MobilePush,
    Crew,
    Web,
    Email,
    Webhook,
    Unknown,
}

impl HiveChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MainThread => "main_thread",
            Self::MobilePush => "mobile_push",
            Self::Crew => "crew",
            Self::Web => "web",
            Self::Email => "email",
            Self::Webhook => "webhook",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveChannelBinding {
    pub id: String,
    pub label: String,
    pub kind: HiveChannelKind,
    pub enabled: bool,
    pub detail: String,
    pub source: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiveCrewProfile {
    pub slug: String,
    pub identity: Option<HiveHomeDocument>,
    pub soul: Option<HiveHomeDocument>,
    pub memory: Option<HiveHomeDocument>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiveHomeProfile {
    pub soul: Option<HiveHomeDocument>,
    pub identity: Option<HiveHomeDocument>,
    pub user: Option<HiveHomeDocument>,
    pub heartbeat: Option<HiveHomeDocument>,
    pub memory: Option<HiveHomeDocument>,
    pub channels: Option<HiveHomeDocument>,
    pub crew: Vec<HiveCrewProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveBootstrapResult {
    pub created_files: Vec<String>,
    pub profile: HiveHomeProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiveCrewRuntimeStatus {
    Idle,
    Running,
    Waiting,
    Degraded,
}

impl HiveCrewRuntimeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveCrewRuntimeSummary {
    pub slug: String,
    pub known_to_home: bool,
    pub status: HiveCrewRuntimeStatus,
    pub active_run_count: usize,
    pub recent_run_count: usize,
    pub failed_run_count: usize,
    pub queued_task_count: usize,
    pub active_task_count: usize,
    pub completed_task_count: usize,
    pub failed_task_count: usize,
    pub latest_activity_at: Option<String>,
}

impl Default for HiveCrewRuntimeSummary {
    fn default() -> Self {
        Self {
            slug: String::new(),
            known_to_home: false,
            status: HiveCrewRuntimeStatus::Idle,
            active_run_count: 0,
            recent_run_count: 0,
            failed_run_count: 0,
            queued_task_count: 0,
            active_task_count: 0,
            completed_task_count: 0,
            failed_task_count: 0,
            latest_activity_at: None,
        }
    }
}
