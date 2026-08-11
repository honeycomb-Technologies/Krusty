//! Small, privacy-safe persistence for native desktop attachment state.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mitsuro_desktop_backend::{BackendKind, BackendSessionId};

const CURRENT_VERSION: u32 = 6;
const STATE_FILE: &str = "gpui-desktop-state.json";
const MAX_PINNED_SESSIONS_PER_BACKEND: usize = 200;

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
