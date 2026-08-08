//! Small, privacy-safe persistence for native desktop attachment state.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mitsuro_desktop_backend::{BackendKind, BackendSessionId};

const CURRENT_VERSION: u32 = 1;
const STATE_FILE: &str = "gpui-desktop-state.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DesktopPreferences {
    pub version: u32,
    pub selected_backend: Option<BackendKind>,
    pub selected_session: Option<BackendSessionId>,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            selected_backend: None,
            selected_session: None,
        }
    }
}

impl DesktopPreferences {
    pub fn load_default() -> io::Result<Self> {
        Self::load(&default_path())
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
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
        if self
            .selected_session
            .as_ref()
            .is_some_and(|session| session.backend != backend)
        {
            self.selected_session = None;
        }
    }

    pub fn remember_session(&mut self, session: BackendSessionId) {
        self.selected_backend = Some(session.backend);
        self.selected_session = Some(session);
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
    fn changing_backend_drops_a_foreign_session_attachment() {
        let mut state = DesktopPreferences::default();
        state.remember_session(BackendSessionId::new(BackendKind::MitsuroHttp, "session-9"));
        state.remember_backend(BackendKind::CodexStdio);
        assert!(state.selected_session.is_none());
        assert_eq!(state.selected_backend, Some(BackendKind::CodexStdio));
    }
}
