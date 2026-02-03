//! OAuth token storage
//!
//! Stores OAuth tokens in ~/.krusty/tokens/oauth.json with secure permissions.
//! Uses in-memory caching to reduce per-call I/O.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use super::types::OAuthTokenData;
use crate::ai::providers::ProviderId;
use crate::paths;

/// Storage for OAuth tokens indexed by provider
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OAuthTokenStore {
    /// OAuth tokens by provider storage key
    #[serde(flatten)]
    tokens: HashMap<String, OAuthTokenData>,
}

/// In-memory cache for OAuth token store
///
/// Reduces disk I/O for frequent token lookups.
/// Cache is invalidated when tokens are modified (save/remove).
static TOKEN_CACHE: Lazy<Mutex<Option<OAuthTokenStore>>> = Lazy::new(|| Mutex::new(None));

impl OAuthTokenStore {
    /// Get the OAuth tokens file path
    fn path() -> PathBuf {
        paths::tokens_dir().join("oauth.json")
    }

    /// Load OAuth tokens from disk (with caching)
    pub fn load() -> Result<Self> {
        // Check cache first
        if let Ok(cache) = TOKEN_CACHE.try_lock() {
            if let Some(cached) = cache.as_ref() {
                tracing::debug!("Using cached OAuth token store");
                return Ok(cached.clone());
            }
        }

        // Cache miss or lock contention - load from disk
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)?;
        let store: OAuthTokenStore = serde_json::from_str(&contents)?;

        // Update cache
        if let Ok(mut cache) = TOKEN_CACHE.try_lock() {
            *cache = Some(store.clone());
        }

        Ok(store)
    }

    /// Save OAuth tokens to disk with secure permissions (atomic write)
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
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
                let _ = fs::set_permissions(&temp_path, permissions);
            }
        }

        // Atomically replace the original file
        fs::rename(&temp_path, path)?;

        // Update cache after successful save
        if let Ok(mut cache) = TOKEN_CACHE.try_lock() {
            *cache = Some(self.clone());
            tracing::debug!("OAuth token cache updated after save");
        }

        Ok(())
    }

    /// Get OAuth token for a provider
    pub fn get(&self, provider: &ProviderId) -> Option<&OAuthTokenData> {
        self.tokens.get(provider.storage_key())
    }

    /// Set OAuth token for a provider
    pub fn set(&mut self, provider: ProviderId, token: OAuthTokenData) {
        self.tokens
            .insert(provider.storage_key().to_string(), token);
    }

    /// Remove OAuth token for a provider
    pub fn remove(&mut self, provider: &ProviderId) {
        self.tokens.remove(provider.storage_key());
    }

    /// Check if a provider has a stored OAuth token
    pub fn has_token(&self, provider: &ProviderId) -> bool {
        self.tokens.contains_key(provider.storage_key())
    }

    /// Check if a provider's token needs refresh
    pub fn needs_refresh(&self, provider: &ProviderId, refresh_days: u64) -> bool {
        self.get(provider)
            .map(|t| t.needs_refresh(refresh_days))
            .unwrap_or(false)
    }

    /// Get all providers with stored OAuth tokens
    pub fn configured_providers(&self) -> Vec<ProviderId> {
        ProviderId::all()
            .iter()
            .filter(|p| self.has_token(p))
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_token() -> OAuthTokenData {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        OAuthTokenData {
            access_token: "test_access_token".to_string(),
            refresh_token: Some("test_refresh_token".to_string()),
            id_token: None,
            expires_at: Some(now + 3600),
            last_refresh: now,
            account_id: Some("test_account".to_string()),
        }
    }

    #[test]
    fn test_token_store_operations() {
        let mut store = OAuthTokenStore::default();
        let token = create_test_token();

        // Initially no token
        assert!(!store.has_token(&ProviderId::OpenAI));
        assert!(store.get(&ProviderId::OpenAI).is_none());

        // Set token
        store.set(ProviderId::OpenAI, token);
        assert!(store.has_token(&ProviderId::OpenAI));
        assert_eq!(
            store.get(&ProviderId::OpenAI).unwrap().access_token,
            "test_access_token"
        );

        // Remove token
        store.remove(&ProviderId::OpenAI);
        assert!(!store.has_token(&ProviderId::OpenAI));
    }

    #[test]
    fn test_serialization() {
        let mut store = OAuthTokenStore::default();
        store.set(ProviderId::OpenAI, create_test_token());

        let json = serde_json::to_string(&store).unwrap();
        let restored: OAuthTokenStore = serde_json::from_str(&json).unwrap();

        assert!(restored.has_token(&ProviderId::OpenAI));
    }
}
