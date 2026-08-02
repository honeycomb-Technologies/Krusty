use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ai::providers::ProviderId;
use crate::auth::{try_refresh_oauth_token_blocking, OAuthTokenStore};
use crate::paths;

/// Storage for API keys indexed by provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStore {
    #[serde(flatten)]
    keys: HashMap<String, String>,
}

impl CredentialStore {
    pub(crate) fn path() -> PathBuf {
        paths::config_dir().join("tokens").join("credentials.json")
    }

    pub fn path_for_home(home_dir: &Path) -> PathBuf {
        home_dir
            .join(".mitsuro")
            .join("tokens")
            .join("credentials.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path)?;
        let store: CredentialStore = serde_json::from_str(&contents)?;
        Ok(store)
    }

    pub fn load_for_home(home_dir: &Path) -> Result<Self> {
        let path = Self::path_for_home(home_dir);
        Self::load_from_path(&path)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        self.save_to_path(&path)
    }

    /// Uses atomic write-to-temp-file-then-rename to prevent corruption.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let temp_path = path.with_extension("tmp");
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&temp_path, contents)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&temp_path) {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o600);
                fs::set_permissions(&temp_path, permissions)
                    .map_err(|e| anyhow::anyhow!("Failed to set secure file permissions: {}", e))?;
                tracing::debug!("Set 0o600 permissions on credentials temp file");
            } else {
                tracing::warn!(
                    "Could not get metadata for credentials temp file, permissions not set"
                );
            }
        }

        fs::rename(&temp_path, path)?;

        #[cfg(windows)]
        {
            tracing::warn!(
                "Windows: File permissions not set - credentials may be accessible to other users"
            );
        }

        tracing::debug!("Credentials saved atomically to {:?}", path);
        Ok(())
    }

    pub fn save_for_home(&self, home_dir: &Path) -> Result<()> {
        let path = Self::path_for_home(home_dir);
        self.save_to_path(&path)
    }

    pub fn get(&self, provider: &ProviderId) -> Option<&String> {
        self.keys.get(provider.storage_key())
    }

    pub fn set(&mut self, provider: ProviderId, key: String) {
        self.keys.insert(provider.storage_key().to_string(), key);
    }

    pub fn has_key(&self, provider: &ProviderId) -> bool {
        self.keys.contains_key(provider.storage_key())
    }

    pub fn configured_providers(&self) -> Vec<ProviderId> {
        ProviderId::all()
            .iter()
            .filter(|p| self.has_key(p))
            .copied()
            .collect()
    }

    pub fn remove(&mut self, provider: &ProviderId) {
        self.keys.remove(provider.storage_key());
    }

    /// Read a secret owned by another core subsystem from the shared
    /// credential boundary. Callers must use a collision-resistant,
    /// namespaced key and must never surface the value in diagnostics.
    pub(crate) fn get_scoped_secret(&self, key: &str) -> Option<&String> {
        self.keys.get(key)
    }

    /// Store a secret owned by another core subsystem in the same atomic,
    /// owner-only credential file used for provider credentials.
    pub(crate) fn set_scoped_secret(&mut self, key: String, value: String) {
        self.keys.insert(key, value);
    }

    pub(crate) fn remove_scoped_secret(&mut self, key: &str) {
        self.keys.remove(key);
    }

    /// Enumerate opaque subsystem-owned keys without exposing their values.
    /// This lets a subsystem garbage-collect credentials for resources that
    /// were removed from configuration while preserving other providers.
    pub(crate) fn scoped_secret_keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.keys
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// This checks API keys first, then falls back to OAuth tokens.
    /// Grok is X-subscription only and resolves from the shared `~/.grok/auth.json` store.
    pub fn get_auth(&self, provider: &ProviderId) -> Option<String> {
        if *provider == ProviderId::Grok {
            return crate::auth::resolve_grok_auth(self).credential;
        }

        if let Some(key) = self.get(provider) {
            return Some(key.clone());
        }

        if provider.supports_oauth() {
            if let Ok(oauth_store) = OAuthTokenStore::load() {
                if let Some(token) = oauth_store.get(provider) {
                    if !token.is_expired() {
                        return Some(token.access_token.clone());
                    }
                    if let Some(refreshed) = token
                        .refresh_token
                        .as_ref()
                        .and_then(|_| try_refresh_oauth_token_blocking(*provider))
                    {
                        return Some(refreshed.access_token);
                    }
                }
            }
        }

        None
    }

    pub fn has_auth(&self, provider: &ProviderId) -> bool {
        self.get_auth(provider).is_some()
    }

    pub fn providers_with_auth(&self) -> Vec<ProviderId> {
        let mut providers: Vec<ProviderId> = ProviderId::all()
            .iter()
            .filter(|p| self.has_auth(p))
            .copied()
            .collect();
        providers.sort_by_key(|p| ProviderId::all().iter().position(|x| x == p).unwrap_or(999));
        providers
    }
}
