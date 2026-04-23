use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::ai::providers::ProviderId;
use crate::paths;

/// Storage for the active provider selection.
pub struct ActiveProviderStore;

impl ActiveProviderStore {
    fn path() -> PathBuf {
        paths::config_dir()
            .join("tokens")
            .join("active_provider.json")
    }

    pub fn load() -> Option<ProviderId> {
        let path = Self::path();
        if !path.exists() {
            return None;
        }
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn save(provider: ProviderId) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string(&provider)?;
        fs::write(&path, contents)?;
        Ok(())
    }
}
