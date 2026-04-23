use crate::paths;

pub(super) const SOUL_CANDIDATES: &[&str] =
    &[paths::MAKO_SOUL_FILE, "mako_soul.md", "SOUL.md", "soul.md"];
pub(super) const IDENTITY_CANDIDATES: &[&str] = &[
    paths::MAKO_IDENTITY_FILE,
    "mako_identity.md",
    "IDENTITY.md",
    "identity.md",
];
pub(super) const HEARTBEAT_CANDIDATES: &[&str] = &[
    paths::MAKO_HEARTBEAT_FILE,
    "mako_heartbeat.md",
    "HEARTBEAT.md",
    "heartbeat.md",
];
pub(super) const MEMORY_CANDIDATES: &[&str] = &[
    paths::MAKO_MEMORY_FILE,
    "mako_memory.md",
    "MEMORY.md",
    "memory.md",
];
pub(super) const CHANNELS_CANDIDATES: &[&str] = &[
    paths::MAKO_CHANNELS_FILE,
    "mako_channels.md",
    "CHANNELS.md",
    "channels.md",
];
pub(super) const CREW_IDENTITY_CANDIDATES: &[&str] = &[
    "IDENTITY.md",
    "identity.md",
    "CREW_IDENTITY.md",
    "crew_identity.md",
    paths::MAKO_IDENTITY_FILE,
];
pub(super) const CREW_SOUL_CANDIDATES: &[&str] = &[
    "SOUL.md",
    "soul.md",
    "CREW_SOUL.md",
    "crew_soul.md",
    paths::MAKO_SOUL_FILE,
];
pub(super) const CREW_MEMORY_CANDIDATES: &[&str] = &[
    "MEMORY.md",
    "memory.md",
    "CREW_MEMORY.md",
    "crew_memory.md",
    paths::MAKO_MEMORY_FILE,
];
pub(super) const DEFAULT_CREW_SLUGS: &[&str] = &["builder", "researcher", "reviewer"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakoHomeDocumentKind {
    Soul,
    Identity,
    Heartbeat,
    Memory,
    Channels,
}

impl MakoHomeDocumentKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "soul" => Some(Self::Soul),
            "identity" => Some(Self::Identity),
            "heartbeat" => Some(Self::Heartbeat),
            "memory" => Some(Self::Memory),
            "channels" => Some(Self::Channels),
            _ => None,
        }
    }

    pub fn preferred_file_name(self) -> &'static str {
        match self {
            Self::Soul => paths::MAKO_SOUL_FILE,
            Self::Identity => paths::MAKO_IDENTITY_FILE,
            Self::Heartbeat => paths::MAKO_HEARTBEAT_FILE,
            Self::Memory => paths::MAKO_MEMORY_FILE,
            Self::Channels => paths::MAKO_CHANNELS_FILE,
        }
    }

    pub(super) fn default_content(self) -> &'static str {
        match self {
            Self::Soul => {
                "# Mako Soul\n\nMako is Krusty's always-on companion.\n- concise\n- calm\n- watchful\n- proactive when it matters\n- never noisy"
            }
            Self::Identity => {
                "# Mako Identity\n\nname: Mako\ncreature: mako shark\ntagline: Always Swimming.\npresence: awake, sleeping, waiting, blocked, idle"
            }
            Self::Heartbeat => {
                "# Mako Heartbeat\n\n- check active runs\n- surface approvals\n- wake on schedule\n- go quiet when nothing needs attention"
            }
            Self::Memory => {
                "# Mako Memory\n\nUse this file for durable operator-facing memory that should carry across runs."
            }
            Self::Channels => {
                "# Mako Channels\n\nMako primarily speaks in the main Mako thread, and can route updates or approvals through enabled channels."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakoCrewDocumentKind {
    Identity,
    Soul,
    Memory,
}

impl MakoCrewDocumentKind {
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

    pub(super) fn default_content(self, slug: &str) -> String {
        match self {
            Self::Identity => format!(
                "# Crew Identity\n\nname: {slug}\nrole: {slug}\ncoordinator: Mako"
            ),
            Self::Soul => match slug {
                "builder" => {
                    "# Crew Soul\n\nBuilder turns approved plans into working changes.\n- direct\n- implementation-first\n- validates before reporting".to_string()
                }
                "researcher" => {
                    "# Crew Soul\n\nResearcher investigates before claiming certainty.\n- reads broadly\n- synthesizes clearly\n- preserves findings".to_string()
                }
                "reviewer" => {
                    "# Crew Soul\n\nReviewer verifies behavior and looks for regressions.\n- skeptical\n- concise\n- evidence-first".to_string()
                }
                _ => format!(
                    "# Crew Soul\n\n{slug} is a distinct working presence in Mako's crew."
                ),
            },
            Self::Memory => {
                format!("# Crew Memory\n\nDurable notes and constraints for {slug}.")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoHomeDocument {
    pub file_name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoContextLayer {
    pub kind: &'static str,
    pub document: MakoHomeDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakoChannelKind {
    MainThread,
    MobilePush,
    Crew,
    Web,
    Email,
    Webhook,
    Unknown,
}

impl MakoChannelKind {
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
pub struct MakoChannelBinding {
    pub id: String,
    pub label: String,
    pub kind: MakoChannelKind,
    pub enabled: bool,
    pub detail: String,
    pub source: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MakoCrewProfile {
    pub slug: String,
    pub identity: Option<MakoHomeDocument>,
    pub soul: Option<MakoHomeDocument>,
    pub memory: Option<MakoHomeDocument>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MakoHomeProfile {
    pub soul: Option<MakoHomeDocument>,
    pub identity: Option<MakoHomeDocument>,
    pub heartbeat: Option<MakoHomeDocument>,
    pub memory: Option<MakoHomeDocument>,
    pub channels: Option<MakoHomeDocument>,
    pub crew: Vec<MakoCrewProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoBootstrapResult {
    pub created_files: Vec<String>,
    pub profile: MakoHomeProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakoCrewRuntimeStatus {
    Idle,
    Running,
    Waiting,
    Degraded,
}

impl MakoCrewRuntimeStatus {
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
pub struct MakoCrewRuntimeSummary {
    pub slug: String,
    pub known_to_home: bool,
    pub status: MakoCrewRuntimeStatus,
    pub active_run_count: usize,
    pub recent_run_count: usize,
    pub failed_run_count: usize,
    pub queued_task_count: usize,
    pub active_task_count: usize,
    pub completed_task_count: usize,
    pub failed_task_count: usize,
    pub latest_activity_at: Option<String>,
}

impl Default for MakoCrewRuntimeSummary {
    fn default() -> Self {
        Self {
            slug: String::new(),
            known_to_home: false,
            status: MakoCrewRuntimeStatus::Idle,
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
