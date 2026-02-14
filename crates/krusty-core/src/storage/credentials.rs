//! Multi-provider credential storage
//!
//! Stores API keys for each provider in a JSON file.
//! Also provides unified auth resolution that checks both API keys and OAuth tokens.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::ai::providers::ProviderId;
use crate::auth::{try_refresh_oauth_token_blocking, OAuthTokenStore};
use crate::paths;

/// Storage for API keys indexed by provider
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CredentialStore {
    /// API keys by provider storage key
    #[serde(flatten)]
    keys: HashMap<String, String>,
}

impl CredentialStore {
    /// Get the credentials file path
    fn path() -> PathBuf {
        paths::config_dir().join("tokens").join("credentials.json")
    }

    /// Get credentials file path for a user's home directory
    pub fn path_for_home(home_dir: &std::path::Path) -> PathBuf {
        home_dir
            .join(".krusty")
            .join("tokens")
            .join("credentials.json")
    }

    /// Load credentials from disk
    pub fn load() -> Result<Self> {
        let path = Self::path();
        Self::load_from_path(&path)
    }

    /// Load credentials from a specific path
    pub fn load_from_path(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path)?;
        let store: CredentialStore = serde_json::from_str(&contents)?;
        Ok(store)
    }

    /// Load credentials for a user's home directory
    pub fn load_for_home(home_dir: &std::path::Path) -> Result<Self> {
        let path = Self::path_for_home(home_dir);
        Self::load_from_path(&path)
    }

    /// Save credentials to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        self.save_to_path(&path)
    }

    /// Save credentials to a specific path
    ///
    /// Uses atomic write-to-temp-file-then-rename pattern to prevent corruption.
    /// On Unix systems, sets file permissions to 0600 (user read/write only).
    /// On Windows, logs a warning that credentials may be accessible to other users.
    ///
    /// # Security
    /// - Atomic write: writes to temp file, then renames over original
    /// - Unix: Sets restrictive 0600 permissions (owner read/write only)
    /// - Windows: No granular permission control, logs warning
    ///
    /// # Errors
    /// Returns error if directory creation, temp file write, or rename fails.
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Create a temporary file in the same directory for atomic rename
        let temp_path = path.with_extension("tmp");
        let contents = serde_json::to_string_pretty(self)?;

        // Write to temp file first
        fs::write(&temp_path, contents)?;

        // Set restrictive permissions on temp file before renaming (Unix)
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

        // Atomically replace the original file
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

    /// Save credentials for a user's home directory
    pub fn save_for_home(&self, home_dir: &std::path::Path) -> Result<()> {
        let path = Self::path_for_home(home_dir);
        self.save_to_path(&path)
    }

    /// Get API key for a provider
    pub fn get(&self, provider: &ProviderId) -> Option<&String> {
        self.keys.get(provider.storage_key())
    }

    /// Set API key for a provider
    pub fn set(&mut self, provider: ProviderId, key: String) {
        self.keys.insert(provider.storage_key().to_string(), key);
    }

    /// Check if a provider has a stored API key
    pub fn has_key(&self, provider: &ProviderId) -> bool {
        self.keys.contains_key(provider.storage_key())
    }

    /// Get all providers with stored API keys
    pub fn configured_providers(&self) -> Vec<ProviderId> {
        ProviderId::all()
            .iter()
            .filter(|p| self.has_key(p))
            .copied()
            .collect()
    }

    /// Remove API key for a provider
    pub fn remove(&mut self, provider: &ProviderId) {
        self.keys.remove(provider.storage_key());
    }

    /// Get authentication credential (API key or OAuth token) for a provider
    ///
    /// This checks API keys first, then falls back to OAuth tokens.
    /// Returns the credential string suitable for use in Authorization headers.
    pub fn get_auth(&self, provider: &ProviderId) -> Option<String> {
        // Try API key first
        if let Some(key) = self.get(provider) {
            return Some(key.clone());
        }

        // Try OAuth token for providers that support it
        if provider.supports_oauth() {
            if let Ok(oauth_store) = OAuthTokenStore::load() {
                if let Some(token) = oauth_store.get(provider) {
                    if !token.is_expired() {
                        return Some(token.access_token.clone());
                    }
                    // Token expired — attempt refresh
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

    /// Check if a provider has any valid authentication configured
    ///
    /// Checks both API keys and OAuth tokens.
    pub fn has_auth(&self, provider: &ProviderId) -> bool {
        self.get_auth(provider).is_some()
    }

    /// Get all providers with any authentication configured
    ///
    /// Includes providers with API keys or valid OAuth tokens.
    pub fn providers_with_auth(&self) -> Vec<ProviderId> {
        let mut providers: Vec<ProviderId> = ProviderId::all()
            .iter()
            .filter(|p| self.has_auth(p))
            .copied()
            .collect();
        providers.sort_by_key(|p| {
            // Keep the same order as ProviderId::all()
            ProviderId::all().iter().position(|x| x == p).unwrap_or(999)
        });
        providers
    }
}

/// Storage for the active provider selection
pub struct ActiveProviderStore;

impl ActiveProviderStore {
    /// Get the active provider file path
    fn path() -> PathBuf {
        paths::config_dir()
            .join("tokens")
            .join("active_provider.json")
    }

    /// Load active provider from disk
    pub fn load() -> ProviderId {
        let path = Self::path();
        if !path.exists() {
            return ProviderId::default();
        }
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save active provider to disk
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
