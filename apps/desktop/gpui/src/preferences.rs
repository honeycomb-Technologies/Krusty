//! Small, privacy-safe persistence for native desktop attachment state.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mitsuro_desktop_backend::{BackendKind, BackendSessionId};

const CURRENT_VERSION: u32 = 8;
const STATE_FILE: &str = "gpui-desktop-state.json";
const MAX_PINNED_SESSIONS_PER_BACKEND: usize = 200;
const MAX_LOCAL_PROJECTS: usize = 50;
const MAX_PROJECT_ROOTS: usize = 8;
const MAX_PROJECT_MEMBERSHIPS_PER_BACKEND: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopProject {
    /// Stable desktop-host identity. The server never interprets this value.
    pub id: String,
    pub name: String,
    /// Canonical absolute folders that provide the project's real workspace context.
    pub root_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DesktopPreferences {
    pub version: u32,
    pub selected_backend: Option<BackendKind>,
    /// Compatibility mirror for the active backend and v1 preference files.
    pub selected_session: Option<BackendSessionId>,
    #[serde(default)]
    pub sessions_by_backend: HashMap<BackendKind, BackendSessionId>,
    /// Desktop-local settings values. These are intentionally separate from
    /// Mitsuro/Codex server configuration and contain no credentials.
    #[serde(default)]
    pub settings_toggles: HashMap<String, bool>,
    #[serde(default)]
    pub settings_choices: HashMap<String, String>,
    #[serde(default)]
    pub models_by_backend: HashMap<BackendKind, String>,
    /// Last reasoning effort selected for each backend/model pair. Values come
    /// only from that model's live advertised capability list.
    #[serde(default)]
    pub reasoning_by_model: HashMap<String, String>,
    /// Fast is remembered only for an exact backend/model capability identity.
    #[serde(default)]
    pub fast_by_model: HashMap<String, bool>,
    /// Plan/Default(Build) selection is scoped to the active backend.
    #[serde(default)]
    pub plan_by_backend: HashMap<BackendKind, bool>,
    /// Ordered desktop-local pinned session ids, scoped to their backend.
    /// Codex desktop owns this in its native host rather than app-server, so
    /// this intentionally remains separate from server-owned thread metadata.
    #[serde(default)]
    pub pinned_sessions_by_backend: HashMap<BackendKind, Vec<String>>,
    /// Native-host projects shared across backends. They organize real sessions by
    /// authoritative working directory and never cache server-owned thread content.
    #[serde(default)]
    pub local_projects: Vec<DesktopProject>,
    /// Explicit native-host membership overrides keyed by backend and durable
    /// server session id. Values are local project ids, never thread content.
    #[serde(default)]
    pub project_assignments_by_backend: HashMap<BackendKind, HashMap<String, String>>,
    /// Durable sessions explicitly removed from cwd-derived project grouping.
    /// This mirrors the reference host's separate projectless-thread identity set.
    #[serde(default)]
    pub projectless_sessions_by_backend: HashMap<BackendKind, Vec<String>>,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            selected_backend: None,
            selected_session: None,
            sessions_by_backend: HashMap::new(),
            settings_toggles: HashMap::new(),
            settings_choices: HashMap::new(),
            models_by_backend: HashMap::new(),
            reasoning_by_model: HashMap::new(),
            fast_by_model: HashMap::new(),
            plan_by_backend: HashMap::new(),
            pinned_sessions_by_backend: HashMap::new(),
            local_projects: Vec::new(),
            project_assignments_by_backend: HashMap::new(),
            projectless_sessions_by_backend: HashMap::new(),
        }
    }
}

impl DesktopPreferences {
    pub fn load_default() -> io::Result<Self> {
        Self::load(&default_path())
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let mut preferences: Self =
                    serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                if let Some(session) = preferences.selected_session.clone() {
                    preferences
                        .sessions_by_backend
                        .entry(session.backend)
                        .or_insert(session);
                }
                if let Some(backend) = preferences.selected_backend {
                    preferences.selected_session =
                        preferences.sessions_by_backend.get(&backend).cloned();
                }
                for ids in preferences.pinned_sessions_by_backend.values_mut() {
                    let mut seen = std::collections::HashSet::new();
                    ids.retain(|id| !id.trim().is_empty() && seen.insert(id.clone()));
                    ids.truncate(MAX_PINNED_SESSIONS_PER_BACKEND);
                }
                preferences
                    .pinned_sessions_by_backend
                    .retain(|_, ids| !ids.is_empty());
                sanitize_local_projects(&mut preferences.local_projects);
                sanitize_project_memberships(&mut preferences);
                preferences.version = CURRENT_VERSION;
                Ok(preferences)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(&default_path())
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "preference path has no parent")
        })?;
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(".{STATE_FILE}.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        fs::write(&temp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(temp, path)
    }

    pub fn remember_backend(&mut self, backend: BackendKind) {
        self.selected_backend = Some(backend);
        self.selected_session = self.sessions_by_backend.get(&backend).cloned();
    }

    pub fn remember_session(&mut self, session: BackendSessionId) {
        self.selected_backend = Some(session.backend);
        self.selected_session = Some(session.clone());
        self.sessions_by_backend.insert(session.backend, session);
    }

    pub fn remember_model(&mut self, backend: BackendKind, model_id: String) {
        self.models_by_backend.insert(backend, model_id);
    }

    pub fn remember_reasoning(&mut self, backend: BackendKind, model_id: &str, effort: String) {
        self.reasoning_by_model
            .insert(reasoning_key(backend, model_id), effort);
    }

    pub fn reasoning_for(&self, backend: BackendKind, model_id: &str) -> Option<&str> {
        self.reasoning_by_model
            .get(&reasoning_key(backend, model_id))
            .map(String::as_str)
    }

    pub fn remember_fast(&mut self, backend: BackendKind, model_id: &str, enabled: bool) {
        self.fast_by_model
            .insert(reasoning_key(backend, model_id), enabled);
    }

    pub fn fast_for(&self, backend: BackendKind, model_id: &str) -> Option<bool> {
        self.fast_by_model
            .get(&reasoning_key(backend, model_id))
            .copied()
    }

    pub fn remember_plan_mode(&mut self, backend: BackendKind, enabled: bool) {
        self.plan_by_backend.insert(backend, enabled);
    }

    pub fn plan_mode_for(&self, backend: BackendKind) -> bool {
        self.plan_by_backend.get(&backend).copied().unwrap_or(false)
    }

    pub fn is_session_pinned(&self, backend: BackendKind, session_id: &str) -> bool {
        self.pinned_sessions_by_backend
            .get(&backend)
            .is_some_and(|ids| ids.iter().any(|id| id == session_id))
    }

    pub fn pinned_session_rank(&self, backend: BackendKind, session_id: &str) -> Option<usize> {
        self.pinned_sessions_by_backend
            .get(&backend)?
            .iter()
            .position(|id| id == session_id)
    }

    pub fn set_session_pinned(&mut self, backend: BackendKind, session_id: String, pinned: bool) {
        if session_id.trim().is_empty() {
            return;
        }
        let ids = self.pinned_sessions_by_backend.entry(backend).or_default();
        ids.retain(|id| id != &session_id);
        if pinned {
            ids.insert(0, session_id);
            ids.truncate(MAX_PINNED_SESSIONS_PER_BACKEND);
        }
        if ids.is_empty() {
            self.pinned_sessions_by_backend.remove(&backend);
        }
    }

    pub fn project(&self, project_id: &str) -> Option<&DesktopProject> {
        self.local_projects
            .iter()
            .find(|project| project.id == project_id)
    }

    pub fn project_for_path(&self, path: &str) -> Option<&DesktopProject> {
        let path = Path::new(path);
        self.local_projects
            .iter()
            .flat_map(|project| {
                project
                    .root_paths
                    .iter()
                    .map(move |root| (project, Path::new(root)))
            })
            .filter(|(_, root)| path == *root || path.starts_with(root))
            .max_by_key(|(_, root)| root.components().count())
            .map(|(project, _)| project)
    }

    pub fn project_for_session(
        &self,
        session: &BackendSessionId,
        working_dir: Option<&str>,
    ) -> Option<&DesktopProject> {
        if session.backend == BackendKind::Fixture
            || self
                .projectless_sessions_by_backend
                .get(&session.backend)
                .is_some_and(|ids| ids.iter().any(|id| id == &session.raw))
        {
            return None;
        }
        if let Some(project_id) = self
            .project_assignments_by_backend
            .get(&session.backend)
            .and_then(|assignments| assignments.get(&session.raw))
        {
            if let Some(project) = self.project(project_id) {
                return Some(project);
            }
        }
        working_dir.and_then(|path| self.project_for_path(path))
    }

    pub fn set_session_project(
        &mut self,
        session: &BackendSessionId,
        working_dir: Option<&str>,
        project_id: Option<&str>,
    ) -> bool {
        if session.backend == BackendKind::Fixture || session.raw.trim().is_empty() {
            return false;
        }
        if let Some(project_id) = project_id {
            if self.project(project_id).is_none() {
                return false;
            }
        }

        if let Some(assignments) = self
            .project_assignments_by_backend
            .get_mut(&session.backend)
        {
            assignments.remove(&session.raw);
            if assignments.is_empty() {
                self.project_assignments_by_backend.remove(&session.backend);
            }
        }
        if let Some(ids) = self
            .projectless_sessions_by_backend
            .get_mut(&session.backend)
        {
            ids.retain(|id| id != &session.raw);
            if ids.is_empty() {
                self.projectless_sessions_by_backend
                    .remove(&session.backend);
            }
        }

        match project_id {
            Some(project_id)
                if self
                    .project_for_path(working_dir.unwrap_or_default())
                    .is_none_or(|project| project.id != project_id) =>
            {
                let assignments = self
                    .project_assignments_by_backend
                    .entry(session.backend)
                    .or_default();
                assignments.insert(session.raw.clone(), project_id.to_owned());
                bound_project_assignments(assignments);
            }
            Some(_) => {}
            None => {
                let ids = self
                    .projectless_sessions_by_backend
                    .entry(session.backend)
                    .or_default();
                ids.insert(0, session.raw.clone());
                dedupe_and_bound_session_ids(ids);
            }
        }
        true
    }

    pub fn add_project(&mut self, project: DesktopProject) {
        if project.id.trim().is_empty()
            || project.name.trim().is_empty()
            || project.root_paths.is_empty()
            || project
                .root_paths
                .iter()
                .any(|root| !Path::new(root).is_absolute())
        {
            return;
        }
        self.local_projects.retain(|existing| {
            existing.id != project.id
                && !existing
                    .root_paths
                    .iter()
                    .any(|root| project.root_paths.contains(root))
        });
        self.local_projects.insert(0, project);
        sanitize_local_projects(&mut self.local_projects);
    }

    pub fn remove_project(&mut self, project_id: &str) {
        self.local_projects
            .retain(|project| project.id != project_id);
        for assignments in self.project_assignments_by_backend.values_mut() {
            assignments.retain(|_, assigned_project_id| assigned_project_id != project_id);
        }
        self.project_assignments_by_backend
            .retain(|_, assignments| !assignments.is_empty());
    }
}

fn sanitize_local_projects(projects: &mut Vec<DesktopProject>) {
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_roots = std::collections::HashSet::new();
    projects.retain_mut(|project| {
        project.id = project.id.trim().to_owned();
        project.name = project.name.trim().to_owned();
        let mut roots_in_project = std::collections::HashSet::new();
        project.root_paths.retain(|root| {
            Path::new(root).is_absolute()
                && roots_in_project.insert(root.clone())
                && !seen_roots.contains(root)
        });
        project.root_paths.truncate(MAX_PROJECT_ROOTS);
        if project.id.is_empty()
            || project.name.is_empty()
            || project.root_paths.is_empty()
            || !seen_ids.insert(project.id.clone())
        {
            return false;
        }
        seen_roots.extend(project.root_paths.iter().cloned());
        true
    });
    projects.truncate(MAX_LOCAL_PROJECTS);
}

fn sanitize_project_memberships(preferences: &mut DesktopPreferences) {
    let valid_projects = preferences
        .local_projects
        .iter()
        .map(|project| project.id.clone())
        .collect::<std::collections::HashSet<_>>();
    preferences
        .project_assignments_by_backend
        .retain(|backend, assignments| {
            if *backend == BackendKind::Fixture {
                return false;
            }
            assignments.retain(|session_id, project_id| {
                !session_id.trim().is_empty() && valid_projects.contains(project_id)
            });
            bound_project_assignments(assignments);
            !assignments.is_empty()
        });
    preferences
        .projectless_sessions_by_backend
        .retain(|backend, ids| {
            if *backend == BackendKind::Fixture {
                return false;
            }
            dedupe_and_bound_session_ids(ids);
            if let Some(assignments) = preferences.project_assignments_by_backend.get_mut(backend) {
                for id in ids.iter() {
                    assignments.remove(id);
                }
            }
            !ids.is_empty()
        });
    preferences
        .project_assignments_by_backend
        .retain(|_, assignments| !assignments.is_empty());
}

fn bound_project_assignments(assignments: &mut HashMap<String, String>) {
    if assignments.len() <= MAX_PROJECT_MEMBERSHIPS_PER_BACKEND {
        return;
    }
    let mut session_ids = assignments.keys().cloned().collect::<Vec<_>>();
    session_ids.sort();
    for session_id in session_ids
        .into_iter()
        .skip(MAX_PROJECT_MEMBERSHIPS_PER_BACKEND)
    {
        assignments.remove(&session_id);
    }
}

fn dedupe_and_bound_session_ids(ids: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| !id.trim().is_empty() && seen.insert(id.clone()));
    ids.truncate(MAX_PROJECT_MEMBERSHIPS_PER_BACKEND);
}

fn reasoning_key(backend: BackendKind, model_id: &str) -> String {
    format!("{}:{model_id}", backend.id())
}

fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MITSURO_GPUI_STATE_PATH") {
        return PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mitsuro")
        .join(STATE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_backend_qualified_session_without_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state.json");
        let mut state = DesktopPreferences::default();
        state.remember_session(BackendSessionId::new(BackendKind::MitsuroHttp, "session-9"));
        state.save(&path).expect("save");
        let restored = DesktopPreferences::load(&path).expect("load");
        assert_eq!(restored, state);
        let raw = fs::read_to_string(path).expect("read state");
        assert!(raw.contains("session-9"));
        assert!(!raw.contains("token"));
    }

    #[test]
    fn changing_backend_restores_each_backends_last_session() {
        let mut state = DesktopPreferences::default();
        state.remember_session(BackendSessionId::new(BackendKind::MitsuroHttp, "session-9"));
        state.remember_backend(BackendKind::CodexStdio);
        assert!(state.selected_session.is_none());
        state.remember_session(BackendSessionId::new(BackendKind::CodexStdio, "thread-4"));
        state.remember_backend(BackendKind::MitsuroHttp);
        assert_eq!(
            state
                .selected_session
                .as_ref()
                .map(|session| session.raw.as_str()),
            Some("session-9")
        );
        assert_eq!(state.sessions_by_backend.len(), 2);
    }

    #[test]
    fn model_selection_is_namespaced_by_backend() {
        let mut state = DesktopPreferences::default();
        state.remember_model(BackendKind::MitsuroHttp, "grok-4.5".into());
        state.remember_model(BackendKind::CodexStdio, "gpt-5.6".into());
        assert_eq!(
            state.models_by_backend.get(&BackendKind::MitsuroHttp),
            Some(&"grok-4.5".to_owned())
        );
        assert_eq!(
            state.models_by_backend.get(&BackendKind::CodexStdio),
            Some(&"gpt-5.6".to_owned())
        );
    }

    #[test]
    fn reasoning_selection_is_namespaced_by_backend_and_model() {
        let mut state = DesktopPreferences::default();
        state.remember_reasoning(BackendKind::MitsuroHttp, "grok-4.5", "max".into());
        state.remember_reasoning(BackendKind::CodexStdio, "gpt-5.6", "high".into());
        state.remember_reasoning(BackendKind::CodexStdio, "gpt-5.5", "medium".into());

        assert_eq!(
            state.reasoning_for(BackendKind::MitsuroHttp, "grok-4.5"),
            Some("max")
        );
        assert_eq!(
            state.reasoning_for(BackendKind::CodexStdio, "gpt-5.6"),
            Some("high")
        );
        assert_eq!(
            state.reasoning_for(BackendKind::CodexStdio, "grok-4.5"),
            None
        );
    }

    #[test]
    fn speed_and_plan_selections_are_scoped_to_their_contract_identity() {
        let mut state = DesktopPreferences::default();
        state.remember_fast(BackendKind::MitsuroHttp, "gpt-5.6-luna", true);
        state.remember_fast(BackendKind::CodexStdio, "gpt-5.6-luna", false);
        state.remember_plan_mode(BackendKind::MitsuroHttp, true);

        assert_eq!(
            state.fast_for(BackendKind::MitsuroHttp, "gpt-5.6-luna"),
            Some(true)
        );
        assert_eq!(
            state.fast_for(BackendKind::CodexStdio, "gpt-5.6-luna"),
            Some(false)
        );
        assert!(state.plan_mode_for(BackendKind::MitsuroHttp));
        assert!(!state.plan_mode_for(BackendKind::CodexStdio));
    }

    #[test]
    fn pinned_sessions_are_ordered_bounded_and_backend_scoped() {
        let mut state = DesktopPreferences::default();
        state.set_session_pinned(BackendKind::CodexStdio, "thread-1".into(), true);
        state.set_session_pinned(BackendKind::CodexStdio, "thread-2".into(), true);
        state.set_session_pinned(BackendKind::MitsuroHttp, "thread-1".into(), true);
        state.set_session_pinned(BackendKind::MitsuroHttp, "   ".into(), true);

        assert_eq!(
            state
                .pinned_sessions_by_backend
                .get(&BackendKind::CodexStdio),
            Some(&vec!["thread-2".to_owned(), "thread-1".to_owned()])
        );
        assert_eq!(
            state.pinned_session_rank(BackendKind::CodexStdio, "thread-2"),
            Some(0)
        );
        assert!(state.is_session_pinned(BackendKind::MitsuroHttp, "thread-1"));

        state.set_session_pinned(BackendKind::CodexStdio, "thread-2".into(), false);
        assert!(!state.is_session_pinned(BackendKind::CodexStdio, "thread-2"));
        assert!(state.is_session_pinned(BackendKind::CodexStdio, "thread-1"));

        for index in 0..=MAX_PINNED_SESSIONS_PER_BACKEND {
            state.set_session_pinned(
                BackendKind::CodexStdio,
                format!("bounded-thread-{index}"),
                true,
            );
        }
        let codex = state
            .pinned_sessions_by_backend
            .get(&BackendKind::CodexStdio)
            .expect("Codex pins");
        assert_eq!(codex.len(), MAX_PINNED_SESSIONS_PER_BACKEND);
        assert_eq!(
            codex.first().map(String::as_str),
            Some("bounded-thread-200")
        );
        assert_eq!(codex.last().map(String::as_str), Some("bounded-thread-1"));
    }

    #[test]
    fn loading_sanitizes_pinned_session_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state.json");
        fs::write(
            &path,
            r#"{
                "version": 5,
                "pinned_sessions_by_backend": {
                    "codex-stdio": ["thread-1", "", "thread-1", "thread-2"]
                }
            }"#,
        )
        .expect("write preferences");

        let restored = DesktopPreferences::load(&path).expect("load preferences");
        assert_eq!(restored.version, CURRENT_VERSION);
        assert_eq!(
            restored
                .pinned_sessions_by_backend
                .get(&BackendKind::CodexStdio),
            Some(&vec!["thread-1".to_owned(), "thread-2".to_owned()])
        );
    }

    #[test]
    fn local_projects_use_real_absolute_roots_and_most_specific_membership() {
        let mut state = DesktopPreferences::default();
        state.add_project(DesktopProject {
            id: "parent".into(),
            name: "Workspace".into(),
            root_paths: vec!["/workspace".into()],
        });
        state.add_project(DesktopProject {
            id: "nested".into(),
            name: "Mitsuro".into(),
            root_paths: vec!["/workspace/Mitsuro".into()],
        });
        state.add_project(DesktopProject {
            id: "invalid".into(),
            name: "Relative".into(),
            root_paths: vec!["relative/path".into()],
        });

        assert!(state.project("invalid").is_none());
        assert_eq!(
            state
                .project_for_path("/workspace/Mitsuro/apps/desktop")
                .map(|project| project.id.as_str()),
            Some("nested")
        );
        assert_eq!(
            state
                .project_for_path("/workspace/Other")
                .map(|project| project.id.as_str()),
            Some("parent")
        );
        assert!(state.project_for_path("/workspace-other").is_none());
    }

    #[test]
    fn explicit_project_membership_overrides_cwd_and_is_backend_scoped() {
        let mut state = DesktopPreferences::default();
        state.add_project(DesktopProject {
            id: "project-a".into(),
            name: "Project A".into(),
            root_paths: vec!["/workspace/a".into()],
        });
        state.add_project(DesktopProject {
            id: "project-b".into(),
            name: "Project B".into(),
            root_paths: vec!["/workspace/b".into()],
        });
        let mitsuro = BackendSessionId::new(BackendKind::MitsuroHttp, "same-id");
        let codex = BackendSessionId::new(BackendKind::CodexStdio, "same-id");

        assert_eq!(
            state
                .project_for_session(&mitsuro, Some("/workspace/a/src"))
                .map(|project| project.id.as_str()),
            Some("project-a")
        );
        assert!(state.set_session_project(&mitsuro, Some("/workspace/a/src"), Some("project-b")));
        assert_eq!(
            state
                .project_for_session(&mitsuro, Some("/workspace/a/src"))
                .map(|project| project.id.as_str()),
            Some("project-b")
        );
        assert_eq!(
            state
                .project_for_session(&codex, Some("/workspace/a/src"))
                .map(|project| project.id.as_str()),
            Some("project-a")
        );

        assert!(state.set_session_project(&codex, Some("/workspace/a/src"), None));
        assert!(state
            .project_for_session(&codex, Some("/workspace/a/src"))
            .is_none());

        assert!(state.set_session_project(&mitsuro, Some("/workspace/a/src"), Some("project-a")));
        assert_eq!(
            state
                .project_for_session(&mitsuro, Some("/workspace/a/src"))
                .map(|project| project.id.as_str()),
            Some("project-a")
        );
        assert!(!state
            .project_assignments_by_backend
            .contains_key(&BackendKind::MitsuroHttp));

        assert!(state.set_session_project(&mitsuro, Some("/workspace/a/src"), Some("project-b")));
        state.remove_project("project-b");
        assert!(state.project("project-b").is_none());
        assert!(!state
            .project_assignments_by_backend
            .contains_key(&BackendKind::MitsuroHttp));
        assert_eq!(
            state
                .project_for_session(&mitsuro, Some("/workspace/a/src"))
                .map(|project| project.id.as_str()),
            Some("project-a")
        );
    }

    #[test]
    fn loading_sanitizes_project_membership_and_projectless_wins() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state.json");
        fs::write(
            &path,
            r#"{
                "version": 7,
                "local_projects": [
                    {"id":"project-a","name":"Project A","rootPaths":["/workspace/a"]}
                ],
                "project_assignments_by_backend": {
                    "codex-stdio": {
                        "thread-1": "project-a",
                        "thread-2": "missing-project",
                        "": "project-a"
                    },
                    "fixture": {"fixture-thread": "project-a"}
                },
                "projectless_sessions_by_backend": {
                    "codex-stdio": ["thread-1", "thread-1", ""],
                    "fixture": ["fixture-thread"]
                }
            }"#,
        )
        .expect("write preferences");

        let restored = DesktopPreferences::load(&path).expect("load preferences");
        assert_eq!(restored.version, CURRENT_VERSION);
        assert!(!restored
            .project_assignments_by_backend
            .contains_key(&BackendKind::CodexStdio));
        assert_eq!(
            restored
                .projectless_sessions_by_backend
                .get(&BackendKind::CodexStdio),
            Some(&vec!["thread-1".to_owned()])
        );
        assert!(!restored
            .projectless_sessions_by_backend
            .contains_key(&BackendKind::Fixture));
    }

    #[test]
    fn loading_sanitizes_duplicate_and_invalid_local_projects() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state.json");
        fs::write(
            &path,
            r#"{
                "version": 6,
                "local_projects": [
                    {"id":" first ","name":" First ","rootPaths":["/workspace","/workspace"]},
                    {"id":"second","name":"Second","rootPaths":["/workspace"]},
                    {"id":"relative","name":"Relative","rootPaths":["tmp/project"]}
                ]
            }"#,
        )
        .expect("write preferences");

        let restored = DesktopPreferences::load(&path).expect("load preferences");
        assert_eq!(restored.version, CURRENT_VERSION);
        assert_eq!(restored.local_projects.len(), 1);
        assert_eq!(restored.local_projects[0].id, "first");
        assert_eq!(restored.local_projects[0].name, "First");
        assert_eq!(restored.local_projects[0].root_paths, ["/workspace"]);
    }

    #[test]
    fn loads_v1_session_into_the_per_backend_map() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state.json");
        fs::write(
            &path,
            r#"{
                "version": 1,
                "selected_backend": "mitsuro-http",
                "selected_session": {"backend":"mitsuro-http","raw":"legacy-session"}
            }"#,
        )
        .expect("write v1 state");
        let restored = DesktopPreferences::load(&path).expect("load v1 state");
        assert_eq!(restored.version, CURRENT_VERSION);
        assert_eq!(
            restored
                .sessions_by_backend
                .get(&BackendKind::MitsuroHttp)
                .map(|session| session.raw.as_str()),
            Some("legacy-session")
        );
    }

    #[test]
    fn round_trips_privacy_safe_local_settings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("state.json");
        let mut state = DesktopPreferences::default();
        state
            .settings_toggles
            .insert("profile_show_name".into(), false);
        state
            .settings_choices
            .insert("send_shortcut".into(), "Ctrl+Enter".into());
        state.save(&path).expect("save");

        let restored = DesktopPreferences::load(&path).expect("load");
        assert_eq!(restored.settings_toggles, state.settings_toggles);
        assert_eq!(restored.settings_choices, state.settings_choices);
    }
}
