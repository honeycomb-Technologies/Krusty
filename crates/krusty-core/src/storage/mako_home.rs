use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, NaiveDateTime};
use tracing::warn;

use super::{
    AutonomousTaskStore, DelegatedRunStore, MakoRuntimeState, MakoRuntimeStateStatus, SessionInfo,
    TaskStatus,
};
use crate::agent::DelegatedRunStage;
use crate::paths;

const SOUL_CANDIDATES: &[&str] = &[paths::MAKO_SOUL_FILE, "mako_soul.md", "SOUL.md", "soul.md"];
const IDENTITY_CANDIDATES: &[&str] = &[
    paths::MAKO_IDENTITY_FILE,
    "mako_identity.md",
    "IDENTITY.md",
    "identity.md",
];
const HEARTBEAT_CANDIDATES: &[&str] = &[
    paths::MAKO_HEARTBEAT_FILE,
    "mako_heartbeat.md",
    "HEARTBEAT.md",
    "heartbeat.md",
];
const MEMORY_CANDIDATES: &[&str] = &[
    paths::MAKO_MEMORY_FILE,
    "mako_memory.md",
    "MEMORY.md",
    "memory.md",
];
const CHANNELS_CANDIDATES: &[&str] = &[
    paths::MAKO_CHANNELS_FILE,
    "mako_channels.md",
    "CHANNELS.md",
    "channels.md",
];
const CREW_IDENTITY_CANDIDATES: &[&str] = &[
    "IDENTITY.md",
    "identity.md",
    "CREW_IDENTITY.md",
    "crew_identity.md",
    paths::MAKO_IDENTITY_FILE,
];
const CREW_SOUL_CANDIDATES: &[&str] = &[
    "SOUL.md",
    "soul.md",
    "CREW_SOUL.md",
    "crew_soul.md",
    paths::MAKO_SOUL_FILE,
];
const CREW_MEMORY_CANDIDATES: &[&str] = &[
    "MEMORY.md",
    "memory.md",
    "CREW_MEMORY.md",
    "crew_memory.md",
    paths::MAKO_MEMORY_FILE,
];
const DEFAULT_CREW_SLUGS: &[&str] = &["builder", "researcher", "reviewer"];

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

    fn default_content(self) -> &'static str {
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

    fn default_content(self, slug: &str) -> String {
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

impl MakoHomeProfile {
    pub fn load() -> Self {
        Self::load_from(&paths::mako_dir())
    }

    pub fn load_from(mako_home: &Path) -> Self {
        Self {
            soul: load_named_document(mako_home, SOUL_CANDIDATES, "Mako soul"),
            identity: load_named_document(mako_home, IDENTITY_CANDIDATES, "Mako identity"),
            heartbeat: load_named_document(mako_home, HEARTBEAT_CANDIDATES, "Mako heartbeat"),
            memory: load_named_document(mako_home, MEMORY_CANDIDATES, "Mako memory"),
            channels: load_named_document(mako_home, CHANNELS_CANDIDATES, "Mako channels"),
            crew: load_crew_profiles(&mako_home.join("crew")),
        }
    }

    pub fn context_layers(&self) -> Vec<MakoContextLayer> {
        let mut layers = Vec::new();
        push_layer(&mut layers, "SOUL", self.soul.clone());
        push_layer(&mut layers, "IDENTITY", self.identity.clone());
        push_layer(&mut layers, "HEARTBEAT", self.heartbeat.clone());
        push_layer(&mut layers, "MEMORY", self.memory.clone());
        push_layer(&mut layers, "CHANNELS", self.channels.clone());
        layers
    }
}

pub fn bootstrap_mako_home(mako_home: &Path) -> std::io::Result<MakoBootstrapResult> {
    fs::create_dir_all(mako_home)?;

    let mut created_files = Vec::new();
    for kind in [
        MakoHomeDocumentKind::Soul,
        MakoHomeDocumentKind::Identity,
        MakoHomeDocumentKind::Heartbeat,
        MakoHomeDocumentKind::Memory,
        MakoHomeDocumentKind::Channels,
    ] {
        if create_document_if_missing(
            mako_home,
            kind.preferred_file_name(),
            kind.default_content(),
        )? {
            created_files.push(kind.preferred_file_name().to_string());
        }
    }

    for slug in DEFAULT_CREW_SLUGS {
        let crew_dir = mako_home.join("crew").join(slug);
        fs::create_dir_all(&crew_dir)?;
        for kind in [
            MakoCrewDocumentKind::Identity,
            MakoCrewDocumentKind::Soul,
            MakoCrewDocumentKind::Memory,
        ] {
            let file_name = kind.preferred_file_name();
            if create_document_if_missing(&crew_dir, file_name, &kind.default_content(slug))? {
                created_files.push(format!("crew/{slug}/{file_name}"));
            }
        }
    }

    Ok(MakoBootstrapResult {
        created_files,
        profile: MakoHomeProfile::load_from(mako_home),
    })
}

pub fn write_mako_home_document(
    mako_home: &Path,
    kind: MakoHomeDocumentKind,
    content: &str,
) -> std::io::Result<MakoHomeDocument> {
    fs::create_dir_all(mako_home)?;
    write_document(mako_home.join(kind.preferred_file_name()), content)
}

pub fn write_mako_crew_document(
    mako_home: &Path,
    slug: &str,
    kind: MakoCrewDocumentKind,
    content: &str,
) -> std::io::Result<MakoHomeDocument> {
    let crew_dir = mako_home.join("crew").join(slug);
    fs::create_dir_all(&crew_dir)?;
    write_document(crew_dir.join(kind.preferred_file_name()), content)
}

pub fn is_valid_crew_slug(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

pub fn summarize_crew_runtime(
    profile: &MakoHomeProfile,
    sessions: &[SessionInfo],
    runtime_states: &HashMap<String, MakoRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<Vec<MakoCrewRuntimeSummary>> {
    let known_slugs = profile
        .crew
        .iter()
        .map(|member| normalize_agent_key(&member.slug))
        .collect::<HashSet<_>>();
    let mut summaries = BTreeMap::<String, MakoCrewRuntimeSummary>::new();

    for member in &profile.crew {
        summaries.insert(
            normalize_agent_key(&member.slug),
            MakoCrewRuntimeSummary {
                slug: member.slug.clone(),
                known_to_home: true,
                ..Default::default()
            },
        );
    }

    for session in sessions {
        if let Some(runtime_state) = runtime_states.get(session.id.as_str()) {
            if let Some(crew_slug) = runtime_state.crew_slug.as_deref() {
                let key = normalize_agent_key(crew_slug);
                if !key.is_empty() {
                    let summary = summaries.entry(key.clone()).or_insert_with(|| {
                        new_runtime_summary(crew_slug, known_slugs.contains(&key))
                    });
                    summary.recent_run_count += 1;
                    match runtime_state.status {
                        MakoRuntimeStateStatus::Running
                        | MakoRuntimeStateStatus::AwaitingInput
                        | MakoRuntimeStateStatus::Paused => {
                            summary.active_run_count += 1;
                        }
                        MakoRuntimeStateStatus::Error => {
                            summary.failed_run_count += 1;
                        }
                        _ => {}
                    }
                    record_latest_activity(
                        &mut summary.latest_activity_at,
                        runtime_state.updated_at.as_str(),
                    );
                }
            }
        }

        for task in task_store.list_tasks(&session.id)? {
            let Some(owner) = task.owner.as_deref() else {
                continue;
            };
            let key = normalize_agent_key(owner);
            if key.is_empty() {
                continue;
            }
            let summary = summaries
                .entry(key.clone())
                .or_insert_with(|| new_runtime_summary(owner, known_slugs.contains(&key)));

            match task.status {
                TaskStatus::Pending => summary.queued_task_count += 1,
                TaskStatus::InProgress => summary.active_task_count += 1,
                TaskStatus::Completed => summary.completed_task_count += 1,
                TaskStatus::Failed => summary.failed_task_count += 1,
            }

            let task_activity = task
                .completed_at
                .as_deref()
                .unwrap_or(task.updated_at.as_str());
            record_latest_activity(&mut summary.latest_activity_at, task_activity);
        }

        for run in delegated_store.list_runs_for_session(&session.id, 100)? {
            let Some(snapshot) = run.snapshot.as_ref() else {
                continue;
            };
            for agent in &snapshot.agents {
                let key = normalize_agent_key(&agent.agent_name);
                if key.is_empty() {
                    continue;
                }
                let summary = summaries.entry(key.clone()).or_insert_with(|| {
                    new_runtime_summary(&agent.agent_name, known_slugs.contains(&key))
                });

                summary.recent_run_count += 1;
                if matches!(
                    run.stage,
                    DelegatedRunStage::Created
                        | DelegatedRunStage::Running
                        | DelegatedRunStage::Synthesizing
                ) || matches!(agent.status.as_str(), "running" | "pending")
                {
                    summary.active_run_count += 1;
                }
                if matches!(
                    run.stage,
                    DelegatedRunStage::Failed | DelegatedRunStage::Degraded
                ) || agent.status.eq_ignore_ascii_case("failed")
                {
                    summary.failed_run_count += 1;
                }
                record_latest_activity(
                    &mut summary.latest_activity_at,
                    &run.updated_at.to_rfc3339(),
                );
            }
        }
    }

    let mut values = summaries.into_values().collect::<Vec<_>>();
    for summary in &mut values {
        summary.status = resolve_runtime_status(summary);
    }
    values.sort_by(
        |left, right| match (left.known_to_home, right.known_to_home) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => left.slug.cmp(&right.slug),
        },
    );
    Ok(values)
}

fn create_document_if_missing(dir: &Path, file_name: &str, content: &str) -> std::io::Result<bool> {
    let path = dir.join(file_name);
    if path.exists() {
        return Ok(false);
    }
    write_document(path, content)?;
    Ok(true)
}

fn write_document(path: PathBuf, content: &str) -> std::io::Result<MakoHomeDocument> {
    let trimmed = content.trim();
    fs::write(&path, trimmed)?;
    Ok(MakoHomeDocument {
        file_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        content: trimmed.to_string(),
    })
}

fn new_runtime_summary(slug: &str, known_to_home: bool) -> MakoCrewRuntimeSummary {
    MakoCrewRuntimeSummary {
        slug: slug.trim().to_string(),
        known_to_home,
        ..Default::default()
    }
}

fn resolve_runtime_status(summary: &MakoCrewRuntimeSummary) -> MakoCrewRuntimeStatus {
    if summary.active_run_count > 0 || summary.active_task_count > 0 {
        MakoCrewRuntimeStatus::Running
    } else if summary.failed_run_count > 0 || summary.failed_task_count > 0 {
        MakoCrewRuntimeStatus::Degraded
    } else if summary.queued_task_count > 0 {
        MakoCrewRuntimeStatus::Waiting
    } else {
        MakoCrewRuntimeStatus::Idle
    }
}

fn record_latest_activity(current: &mut Option<String>, candidate: &str) {
    if candidate.trim().is_empty() {
        return;
    }
    match current {
        Some(existing) => {
            if timestamp_sort_key(candidate) > timestamp_sort_key(existing) {
                *existing = candidate.to_string();
            }
        }
        None => *current = Some(candidate.to_string()),
    }
}

fn timestamp_sort_key(value: &str) -> i64 {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return parsed.timestamp();
    }
    if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return parsed.and_utc().timestamp();
    }
    i64::MIN
}

fn push_layer(
    layers: &mut Vec<MakoContextLayer>,
    kind: &'static str,
    document: Option<MakoHomeDocument>,
) {
    if let Some(document) = document {
        layers.push(MakoContextLayer { kind, document });
    }
}

fn normalize_agent_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn load_crew_profiles(crew_root: &Path) -> Vec<MakoCrewProfile> {
    let entries = match fs::read_dir(crew_root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut dirs = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then_some(path)
        })
        .collect::<Vec<_>>();
    dirs.sort();

    dirs.into_iter()
        .filter_map(|dir| {
            let slug = dir
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())?;

            let profile = MakoCrewProfile {
                slug,
                identity: load_named_document(&dir, CREW_IDENTITY_CANDIDATES, "Mako crew identity"),
                soul: load_named_document(&dir, CREW_SOUL_CANDIDATES, "Mako crew soul"),
                memory: load_named_document(&dir, CREW_MEMORY_CANDIDATES, "Mako crew memory"),
            };

            if profile.identity.is_none() && profile.soul.is_none() && profile.memory.is_none() {
                None
            } else {
                Some(profile)
            }
        })
        .collect()
}

fn load_named_document(
    base_dir: &Path,
    candidates: &[&str],
    context: &'static str,
) -> Option<MakoHomeDocument> {
    let path = discover_named_file(base_dir, candidates)?;
    load_document(&path, context)
}

fn discover_named_file(base_dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|name| base_dir.join(name))
        .find(|path| path.is_file())
}

fn load_document(path: &Path, context: &'static str) -> Option<MakoHomeDocument> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            warn!(context, path = %path.display(), error = %error, "Failed to read Mako home document");
            return None;
        }
    };

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(MakoHomeDocument {
        file_name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        content: trimmed.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        bootstrap_mako_home, is_valid_crew_slug, summarize_crew_runtime, write_mako_crew_document,
        write_mako_home_document, MakoCrewDocumentKind, MakoCrewRuntimeStatus,
        MakoHomeDocumentKind, MakoHomeProfile,
    };
    use crate::agent::DelegatedRunStage;
    use crate::paths;
    use crate::storage::{
        AutonomousTaskStore, Database, DelegatedRunAgentSnapshot, DelegatedRunRole,
        DelegatedRunScope, DelegatedRunSnapshot, DelegatedRunStartInput, DelegatedRunStore,
        SessionManager, SessionType, WorkspaceMode,
    };
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn load_mako_home_prefers_branded_top_level_files() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join(paths::MAKO_SOUL_FILE), "Soul.").unwrap();
        fs::write(temp.path().join(paths::MAKO_IDENTITY_FILE), "Identity.").unwrap();

        let profile = MakoHomeProfile::load_from(temp.path());

        assert_eq!(profile.soul.unwrap().file_name, paths::MAKO_SOUL_FILE);
        assert_eq!(
            profile.identity.unwrap().file_name,
            paths::MAKO_IDENTITY_FILE
        );
    }

    #[test]
    fn load_mako_home_falls_back_to_legacy_generic_files() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("SOUL.md"), "Soul.").unwrap();
        fs::write(temp.path().join("CHANNELS.md"), "Channels.").unwrap();

        let profile = MakoHomeProfile::load_from(temp.path());

        assert_eq!(profile.soul.unwrap().file_name, "SOUL.md");
        assert_eq!(profile.channels.unwrap().file_name, "CHANNELS.md");
    }

    #[test]
    fn load_mako_home_discovers_sorted_crew_profiles() {
        let temp = TempDir::new().unwrap();
        let reviewer = temp.path().join("crew").join("reviewer");
        let builder = temp.path().join("crew").join("builder");
        fs::create_dir_all(&reviewer).unwrap();
        fs::create_dir_all(&builder).unwrap();
        fs::write(reviewer.join("IDENTITY.md"), "Reviewer").unwrap();
        fs::write(builder.join("SOUL.md"), "Builder soul").unwrap();

        let profile = MakoHomeProfile::load_from(temp.path());
        let slugs = profile
            .crew
            .iter()
            .map(|member| member.slug.clone())
            .collect::<Vec<_>>();

        assert_eq!(slugs, vec!["builder".to_string(), "reviewer".to_string()]);
    }

    #[test]
    fn bootstrap_mako_home_creates_branded_files_and_default_crew() {
        let temp = TempDir::new().unwrap();

        let result = bootstrap_mako_home(temp.path()).unwrap();

        assert!(result
            .created_files
            .iter()
            .any(|path| path == paths::MAKO_SOUL_FILE));
        assert!(result
            .created_files
            .iter()
            .any(|path| path == "crew/builder/IDENTITY.md"));
        assert_eq!(result.profile.crew.len(), 3);
    }

    #[test]
    fn write_document_helpers_use_preferred_file_names() {
        let temp = TempDir::new().unwrap();

        let home_doc =
            write_mako_home_document(temp.path(), MakoHomeDocumentKind::Identity, "Mako").unwrap();
        let crew_doc = write_mako_crew_document(
            temp.path(),
            "researcher",
            MakoCrewDocumentKind::Soul,
            "Read widely",
        )
        .unwrap();

        assert_eq!(home_doc.file_name, paths::MAKO_IDENTITY_FILE);
        assert_eq!(crew_doc.file_name, "SOUL.md");
        assert!(temp.path().join(paths::MAKO_IDENTITY_FILE).is_file());
        assert!(temp
            .path()
            .join("crew")
            .join("researcher")
            .join("SOUL.md")
            .is_file());
    }

    #[test]
    fn crew_slug_validation_is_conservative() {
        assert!(is_valid_crew_slug("reviewer"));
        assert!(is_valid_crew_slug("ops_1"));
        assert!(!is_valid_crew_slug("Reviewer"));
        assert!(!is_valid_crew_slug("../evil"));
        assert!(!is_valid_crew_slug(""));
    }

    #[test]
    fn summarize_crew_runtime_merges_profile_tasks_and_delegated_runs() {
        let temp = TempDir::new().unwrap();
        bootstrap_mako_home(temp.path()).unwrap();
        let db_path = temp.path().join("krusty.db");
        let db = Database::new(&db_path).unwrap();
        db.conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                ("alice", "alice@example.com", "free"),
            )
            .unwrap();
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let session_id = session_manager
            .create_session_for_user_with_config(
                "Mako",
                None,
                Some(temp.path().to_string_lossy().as_ref()),
                Some(temp.path().to_string_lossy().as_ref()),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Mako,
            )
            .unwrap();

        let task_store = AutonomousTaskStore::new(Database::new(&db_path).unwrap());
        let delegated_store = DelegatedRunStore::new(Database::new(&db_path).unwrap());
        let task_id = task_store
            .create_task(&session_id, "Build", "Implement feature", &[])
            .unwrap();
        task_store.claim_task(&task_id, "builder").unwrap();

        delegated_store
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: "run-1".to_string(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: None,
                role: DelegatedRunRole::Build,
                stage: DelegatedRunStage::Running,
                provider: None,
                model: None,
                resumable: false,
                resumed_from_run_id: None,
                target_scope: vec![DelegatedRunScope {
                    label: "repo".to_string(),
                    path: temp.path().to_string_lossy().to_string(),
                    kind: "dir".to_string(),
                }],
            })
            .unwrap();
        delegated_store
            .update_snapshot(
                "run-1",
                DelegatedRunStage::Running,
                &DelegatedRunSnapshot {
                    stage: DelegatedRunStage::Running,
                    agents: vec![DelegatedRunAgentSnapshot {
                        task_id: "task-1".to_string(),
                        agent_name: "builder".to_string(),
                        status: "running".to_string(),
                        tool_count: 1,
                        tokens: 10,
                        current_action: None,
                        completion_summary: None,
                        lines_added: 0,
                        lines_removed: 0,
                        completed_plan_task: None,
                    }],
                },
            )
            .unwrap();
        delegated_store
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: "run-2".to_string(),
                parent_session_id: session_id.clone(),
                parent_tool_call_id: None,
                role: DelegatedRunRole::Verifier,
                stage: DelegatedRunStage::Failed,
                provider: None,
                model: None,
                resumable: false,
                resumed_from_run_id: None,
                target_scope: vec![DelegatedRunScope {
                    label: "repo".to_string(),
                    path: temp.path().to_string_lossy().to_string(),
                    kind: "dir".to_string(),
                }],
            })
            .unwrap();
        delegated_store
            .update_snapshot(
                "run-2",
                DelegatedRunStage::Failed,
                &DelegatedRunSnapshot {
                    stage: DelegatedRunStage::Failed,
                    agents: vec![DelegatedRunAgentSnapshot {
                        task_id: "task-2".to_string(),
                        agent_name: "reviewer".to_string(),
                        status: "failed".to_string(),
                        tool_count: 1,
                        tokens: 10,
                        current_action: None,
                        completion_summary: None,
                        lines_added: 0,
                        lines_removed: 0,
                        completed_plan_task: None,
                    }],
                },
            )
            .unwrap();

        let profile = MakoHomeProfile::load_from(temp.path());
        let sessions = vec![session_manager.get_session(&session_id).unwrap().unwrap()];
        let summary = summarize_crew_runtime(
            &profile,
            &sessions,
            &HashMap::new(),
            &task_store,
            &delegated_store,
        )
        .unwrap();

        let builder = summary
            .iter()
            .find(|member| member.slug == "builder")
            .unwrap();
        assert_eq!(builder.status, MakoCrewRuntimeStatus::Running);
        assert_eq!(builder.active_task_count, 1);
        assert_eq!(builder.active_run_count, 1);

        let reviewer = summary
            .iter()
            .find(|member| member.slug == "reviewer")
            .unwrap();
        assert_eq!(reviewer.status, MakoCrewRuntimeStatus::Degraded);
        assert_eq!(reviewer.failed_run_count, 1);
        assert!(summary.iter().any(|member| member.slug == "researcher"));
    }
}
