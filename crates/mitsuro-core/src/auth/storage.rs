//! OAuth token storage
//!
//! Stores OAuth tokens in ~/.mitsuro/tokens/oauth.json with secure permissions.
//! Uses atomic replacement plus an advisory lockfile so the CLI and server can
//! safely share the store.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::Result;
use fs2::FileExt;
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

/// Serializes read-modify-write operations against the shared token file.
///
/// Callers that need to persist a mutation must use the atomic helpers below
/// instead of composing `load` + `set`/`remove` + `save`, which can lose a
/// concurrent provider update. A file lock provides the equivalent guarantee
/// across Mitsuro processes.
static TOKEN_STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Holds the provider-scoped advisory lock for the complete refresh exchange.
/// Dropping the guard closes the file and releases the cross-process lock.
pub(crate) struct OAuthRefreshLock {
    _file: File,
}

impl OAuthTokenStore {
    /// Get the OAuth tokens file path
    fn path() -> PathBuf {
        paths::tokens_dir().join("oauth.json")
    }

    fn lock_store() -> Result<MutexGuard<'static, ()>> {
        TOKEN_STORE_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("OAuth token store lock was poisoned"))
    }

    fn open_lock_file(path: &Path) -> Result<File> {
        Self::open_named_lock_file(&path.with_extension("lock"))
    }

    fn open_named_lock_file(lock_path: &Path) -> Result<File> {
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = lock_file.metadata()?.permissions();
            permissions.set_mode(0o600);
            let _ = lock_file.set_permissions(permissions);
        }

        Ok(lock_file)
    }

    fn provider_refresh_lock_path(provider: ProviderId) -> PathBuf {
        paths::tokens_dir().join(format!("oauth.{}.refresh.lock", provider.storage_key()))
    }

    fn acquire_provider_refresh_lock_at(path: &Path) -> Result<OAuthRefreshLock> {
        let file = Self::open_named_lock_file(path)?;
        FileExt::lock_exclusive(&file)?;
        Ok(OAuthRefreshLock { _file: file })
    }

    fn lock_file_shared(path: &Path) -> Result<File> {
        let lock_file = Self::open_lock_file(path)?;
        FileExt::lock_shared(&lock_file)?;
        Ok(lock_file)
    }

    fn lock_file_exclusive(path: &Path) -> Result<File> {
        let lock_file = Self::open_lock_file(path)?;
        FileExt::lock_exclusive(&lock_file)?;
        Ok(lock_file)
    }

    fn load_from_disk(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    fn save_to_disk(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // A process-specific temporary path prevents unrelated Mitsuro
        // processes from writing the same temporary file concurrently.
        let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&temp_path, contents)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&temp_path) {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o600);
                let _ = fs::set_permissions(&temp_path, permissions);
            }
        }

        fs::rename(&temp_path, path)?;
        Ok(())
    }

    fn update_persisted_at<R>(
        path: &Path,
        mutate: impl FnOnce(&mut Self) -> (R, bool),
    ) -> Result<R> {
        let _guard = Self::lock_store()?;
        let _file_lock = Self::lock_file_exclusive(path)?;
        let mut store = Self::load_from_disk(path)?;
        let (result, changed) = mutate(&mut store);

        if changed {
            store.save_to_disk(path)?;
        }

        Ok(result)
    }

    fn set_persisted_at(path: &Path, provider: ProviderId, token: OAuthTokenData) -> Result<()> {
        Self::update_persisted_at(path, move |store| {
            store.set(provider, token);
            ((), true)
        })
    }

    fn remove_persisted_at(path: &Path, provider: &ProviderId) -> Result<bool> {
        Self::update_persisted_at(path, |store| {
            let removed = store.tokens.remove(provider.storage_key()).is_some();
            (removed, removed)
        })
    }

    fn remove_persisted_if_refresh_token_matches_at(
        path: &Path,
        provider: &ProviderId,
        expected_refresh_token: &str,
    ) -> Result<bool> {
        Self::update_persisted_at(path, |store| {
            let matches = store
                .get(provider)
                .and_then(|token| token.refresh_token.as_deref())
                == Some(expected_refresh_token);
            if matches {
                store.tokens.remove(provider.storage_key());
            }
            (matches, matches)
        })
    }

    fn replace_persisted_if_refresh_token_matches_at(
        path: &Path,
        provider: &ProviderId,
        expected_refresh_token: &str,
        replacement: OAuthTokenData,
    ) -> Result<bool> {
        Self::update_persisted_at(path, move |store| {
            let matches = store
                .get(provider)
                .and_then(|token| token.refresh_token.as_deref())
                == Some(expected_refresh_token);
            if matches {
                store.set(*provider, replacement);
            }
            (matches, matches)
        })
    }

    /// Load the latest OAuth tokens under a shared cross-process lock.
    pub fn load() -> Result<Self> {
        let _guard = Self::lock_store()?;
        let path = Self::path();
        let _file_lock = Self::lock_file_shared(&path)?;
        Self::load_from_disk(&path)
    }

    /// Save this complete snapshot with secure permissions (atomic write).
    ///
    /// Production read-modify-write paths should use `set_persisted`,
    /// `remove_persisted`, or one of the refresh-token CAS helpers so a stale
    /// snapshot cannot overwrite another process's provider update.
    pub fn save(&self) -> Result<()> {
        let _guard = Self::lock_store()?;
        let path = Self::path();
        let _file_lock = Self::lock_file_exclusive(&path)?;
        self.save_to_disk(&path)
    }

    /// Atomically store one provider's token without overwriting concurrent
    /// updates for other providers.
    pub fn set_persisted(provider: ProviderId, token: OAuthTokenData) -> Result<()> {
        Self::set_persisted_at(&Self::path(), provider, token)
    }

    /// Atomically remove one provider's token.
    pub fn remove_persisted(provider: &ProviderId) -> Result<bool> {
        Self::remove_persisted_at(&Self::path(), provider)
    }

    /// Remove a rejected credential only if it is still the exact refresh
    /// token used by the failed request. A concurrent successful rotation is
    /// therefore preserved.
    pub fn remove_persisted_if_refresh_token_matches(
        provider: &ProviderId,
        expected_refresh_token: &str,
    ) -> Result<bool> {
        Self::remove_persisted_if_refresh_token_matches_at(
            &Self::path(),
            provider,
            expected_refresh_token,
        )
    }

    /// Atomically replace a refreshed credential only if the store still
    /// contains the exact refresh token used by the request.
    pub fn replace_persisted_if_refresh_token_matches(
        provider: &ProviderId,
        expected_refresh_token: &str,
        replacement: OAuthTokenData,
    ) -> Result<bool> {
        Self::replace_persisted_if_refresh_token_matches_at(
            &Self::path(),
            provider,
            expected_refresh_token,
            replacement,
        )
    }

    /// Serialize one provider's full refresh exchange across Mitsuro processes.
    /// The blocking advisory-lock acquisition runs off the async executor.
    pub(crate) async fn lock_provider_refresh(provider: ProviderId) -> Result<OAuthRefreshLock> {
        let path = Self::provider_refresh_lock_path(provider);
        tokio::task::spawn_blocking(move || Self::acquire_provider_refresh_lock_at(&path))
            .await
            .map_err(|error| anyhow::anyhow!("OAuth refresh lock task failed: {error}"))?
    }

    /// Get OAuth token for a provider
    pub fn get(&self, provider: &ProviderId) -> Option<&OAuthTokenData> {
        self.tokens.get(provider.storage_key())
    }

    /// Set a token in this in-memory snapshot without persisting it.
    pub fn set(&mut self, provider: ProviderId, token: OAuthTokenData) {
        self.tokens
            .insert(provider.storage_key().to_string(), token);
    }

    /// Remove a token from this in-memory snapshot without persisting it.
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

    fn create_test_token_with_refresh(access_token: &str, refresh_token: &str) -> OAuthTokenData {
        let mut token = create_test_token();
        token.access_token = access_token.to_string();
        token.refresh_token = Some(refresh_token.to_string());
        token
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

    #[test]
    fn serialized_provider_updates_do_not_lose_each_other() {
        use std::sync::{Arc, Barrier};

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("oauth.json");
        let barrier = Arc::new(Barrier::new(3));

        let openai_path = path.clone();
        let openai_barrier = Arc::clone(&barrier);
        let openai = std::thread::spawn(move || {
            openai_barrier.wait();
            OAuthTokenStore::set_persisted_at(
                &openai_path,
                ProviderId::OpenAI,
                create_test_token_with_refresh("openai-access", "openai-refresh"),
            )
            .unwrap();
        });

        let anthropic_path = path.clone();
        let anthropic_barrier = Arc::clone(&barrier);
        let anthropic = std::thread::spawn(move || {
            anthropic_barrier.wait();
            OAuthTokenStore::set_persisted_at(
                &anthropic_path,
                ProviderId::Anthropic,
                create_test_token_with_refresh("anthropic-access", "anthropic-refresh"),
            )
            .unwrap();
        });

        barrier.wait();
        openai.join().unwrap();
        anthropic.join().unwrap();

        let stored = OAuthTokenStore::load_from_disk(&path).unwrap();
        assert_eq!(
            stored.get(&ProviderId::OpenAI).unwrap().access_token,
            "openai-access"
        );
        assert_eq!(
            stored.get(&ProviderId::Anthropic).unwrap().access_token,
            "anthropic-access"
        );
    }

    #[test]
    fn rejected_refresh_token_cas_preserves_a_concurrent_rotation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("oauth.json");

        OAuthTokenStore::set_persisted_at(
            &path,
            ProviderId::OpenAI,
            create_test_token_with_refresh("old-access", "old-refresh"),
        )
        .unwrap();
        OAuthTokenStore::set_persisted_at(
            &path,
            ProviderId::OpenAI,
            create_test_token_with_refresh("rotated-access", "rotated-refresh"),
        )
        .unwrap();

        let removed = OAuthTokenStore::remove_persisted_if_refresh_token_matches_at(
            &path,
            &ProviderId::OpenAI,
            "old-refresh",
        )
        .unwrap();
        assert!(!removed);
        assert_eq!(
            OAuthTokenStore::load_from_disk(&path)
                .unwrap()
                .get(&ProviderId::OpenAI)
                .unwrap()
                .access_token,
            "rotated-access"
        );

        let removed = OAuthTokenStore::remove_persisted_if_refresh_token_matches_at(
            &path,
            &ProviderId::OpenAI,
            "rotated-refresh",
        )
        .unwrap();
        assert!(removed);
        assert!(OAuthTokenStore::load_from_disk(&path)
            .unwrap()
            .get(&ProviderId::OpenAI)
            .is_none());
    }

    #[test]
    fn refresh_success_cas_does_not_overwrite_a_concurrent_rotation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("oauth.json");

        OAuthTokenStore::set_persisted_at(
            &path,
            ProviderId::OpenAI,
            create_test_token_with_refresh("rotated-access", "rotated-refresh"),
        )
        .unwrap();

        let replaced = OAuthTokenStore::replace_persisted_if_refresh_token_matches_at(
            &path,
            &ProviderId::OpenAI,
            "old-refresh",
            create_test_token_with_refresh("stale-access", "stale-refresh"),
        )
        .unwrap();

        assert!(!replaced);
        let stored = OAuthTokenStore::load_from_disk(&path).unwrap();
        let token = stored.get(&ProviderId::OpenAI).unwrap();
        assert_eq!(token.access_token, "rotated-access");
        assert_eq!(token.refresh_token.as_deref(), Some("rotated-refresh"));
    }

    #[test]
    fn provider_refresh_lock_serializes_same_provider_exchange() {
        use std::sync::mpsc;
        use std::time::Duration;

        let temp_dir = tempfile::tempdir().unwrap();
        let lock_path = temp_dir.path().join("oauth.openai.refresh.lock");
        let first_lock_path = lock_path.clone();
        let (first_acquired_tx, first_acquired_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();

        let first = std::thread::spawn(move || {
            let _guard =
                OAuthTokenStore::acquire_provider_refresh_lock_at(&first_lock_path).unwrap();
            first_acquired_tx.send(()).unwrap();
            release_first_rx.recv().unwrap();
        });
        first_acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first refresh should acquire the provider lock");

        let second_lock_path = lock_path;
        let (second_acquired_tx, second_acquired_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            let _guard =
                OAuthTokenStore::acquire_provider_refresh_lock_at(&second_lock_path).unwrap();
            second_acquired_tx.send(()).unwrap();
        });

        assert!(
            second_acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "a concurrent refresh must wait for the same provider lock"
        );
        release_first_tx.send(()).unwrap();
        second_acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second refresh should proceed after the first guard drops");

        first.join().unwrap();
        second.join().unwrap();
    }
}
