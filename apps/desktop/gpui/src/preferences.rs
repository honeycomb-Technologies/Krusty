//! Small, privacy-safe persistence for native desktop attachment state.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mitsuro_desktop_backend::{BackendKind, BackendSessionId};

const CURRENT_VERSION: u32 = 3;
const STATE_FILE: &str = "gpui-desktop-state.json";

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
        state.settings_toggles.insert("reduce_motion".into(), true);
        state.settings_choices.insert("theme".into(), "Dark".into());
        state.save(&path).expect("save");

        let restored = DesktopPreferences::load(&path).expect("load");
        assert_eq!(restored.settings_toggles, state.settings_toggles);
        assert_eq!(restored.settings_choices, state.settings_choices);
    }
}
