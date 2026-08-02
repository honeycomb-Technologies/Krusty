//! The heart of proper caching for multi-harness use.
//!
//! Design goals (copied from real-world usage of the official client):
//! - Multiple processes (official `grok` + Mitsuro + other agents) must not corrupt auth.json.
//! - Fast path: in-memory token so we don't hit disk on every model call.
//! - Proactive refresh so a token doesn't die in the middle of a long agent turn.
//! - Atomic writes + advisory locks.
//! - Full round-tripping of the rich metadata the official CLI stores.

use crate::config::AuthConfig;
use crate::error::{AuthError, Result};
use crate::token::{AuthEntry, AuthToken};
use chrono::{Duration, Utc};
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct AuthStore {
    path: PathBuf,
    config: AuthConfig,
    /// Hot in-memory cache. Protected by RwLock so many tasks can read a valid token
    /// while a single refresh (which may do network + disk) happens.
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    current: Option<(String, AuthEntry)>, // (issuer_key, entry)
}

impl AuthStore {
    pub async fn new(path: PathBuf, config: AuthConfig) -> Result<Self> {
        let store = Self {
            path,
            config,
            inner: Arc::new(RwLock::new(Inner::default())),
        };
        // Initial load (best effort)
        let _ = store.load_from_disk().await;
        Ok(store)
    }

    /// Load (or create) the current valid token, refreshing if necessary.
    /// This is the main method callers should use before every authenticated request.
    pub async fn ensure_fresh(&self) -> Result<AuthToken> {
        // Fast path under read lock
        {
            let guard = self.inner.read().await;
            if let Some((key, entry)) = &guard.current {
                if !entry.is_expired(self.config.early_invalidation_buffer) {
                    return Ok(AuthToken::from((key.clone(), entry.clone())));
                }
            }
        }

        // Slow path: may need refresh or full re-auth
        self.refresh_or_reauth().await
    }

    async fn refresh_or_reauth(&self) -> Result<AuthToken> {
        let mut guard = self.inner.write().await;

        // Re-check under write lock (another task may have refreshed)
        if let Some((key, entry)) = &guard.current {
            if !entry.is_expired(self.config.early_invalidation_buffer) {
                return Ok(AuthToken::from((key.clone(), entry.clone())));
            }
        }

        // Try to load fresh from disk (another process may have logged in or refreshed)
        if let Ok((key, entry)) = self.load_from_disk_locked().await {
            if !entry.is_expired(self.config.early_invalidation_buffer) {
                guard.current = Some((key.clone(), entry.clone()));
                return Ok(AuthToken::from((key, entry)));
            }
            // We have an expired entry on disk that has a refresh_token → try refresh
            if let Some(refresh) = &entry.refresh_token {
                match self.perform_refresh(&key, &entry, refresh).await {
                    Ok(new_entry) => {
                        self.save_to_disk_locked(&key, &new_entry).await?;
                        guard.current = Some((key.clone(), new_entry.clone()));
                        return Ok(AuthToken::from((key, new_entry)));
                    }
                    Err(e) => {
                        tracing::warn!("refresh using stored refresh_token failed: {}", e);
                    }
                }
            }
        }

        // No valid token and refresh didn't work → we need to (re)authenticate.
        // For a library we usually return an error here and let the caller decide
        // whether to call an interactive login or fall back to XAI_API_KEY.
        if self.config.api_key.is_some() {
            // Direct API key path (no refresh possible)
            let entry = AuthEntry {
                access_token: self.config.api_key.clone().unwrap(),
                auth_mode: Some("api_key".to_string()),
                ..Default::default()
            };
            let issuer_key = "api_key".to_string();
            self.save_to_disk_locked(&issuer_key, &entry).await?;
            guard.current = Some((issuer_key.clone(), entry.clone()));
            return Ok(AuthToken::from((issuer_key, entry)));
        }

        Err(AuthError::NoCredentials)
    }

    /// Perform an OIDC refresh using the stored refresh_token.
    async fn perform_refresh(
        &self,
        issuer_key: &str,
        old_entry: &AuthEntry,
        refresh_token: &str,
    ) -> Result<AuthEntry> {
        // We need the token endpoint. For xAI we can either hardcode or do discovery.
        // The official client uses the OIDC issuer, so we do discovery.
        let issuer = old_entry
            .oidc_issuer
            .as_deref()
            .unwrap_or("https://auth.x.ai");

        let discovery = crate::oidc::discover(issuer).await?;
        let token_endpoint =
            discovery
                .token_endpoint
                .ok_or_else(|| AuthError::DiscoveryFailed {
                    issuer: issuer.to_string(),
                    msg: "no token_endpoint in discovery".into(),
                })?;

        let client_id = old_entry
            .oidc_client_id
            .clone()
            .or_else(|| self.config.oidc_client_id.clone())
            .ok_or_else(|| AuthError::Config("no client_id for refresh".into()))?;

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &client_id),
        ];

        let resp: serde_json::Value = reqwest::Client::new()
            .post(token_endpoint)
            .form(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let new_access = resp["access_token"]
            .as_str()
            .ok_or_else(|| AuthError::RefreshFailed("no access_token in refresh response".into()))?
            .to_string();

        let new_refresh = resp["refresh_token"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| old_entry.refresh_token.clone());

        let expires_in = resp["expires_in"].as_i64().unwrap_or(3600);
        let new_expires = Utc::now() + Duration::seconds(expires_in);

        let mut new_entry = old_entry.clone();
        new_entry.access_token = new_access;
        new_entry.refresh_token = new_refresh;
        new_entry.expires_at = Some(new_expires);

        tracing::info!(
            "refreshed token for {} (new exp {:?})",
            issuer_key,
            new_entry.expires_at
        );

        Ok(new_entry)
    }

    /// Load the entire auth.json (with lock) and pick the best entry for our config.
    async fn load_from_disk(&self) -> Result<(String, AuthEntry)> {
        self.load_from_disk_locked().await
    }

    async fn load_from_disk_locked(&self) -> Result<(String, AuthEntry)> {
        let path = &self.path;
        if !path.exists() {
            return Err(AuthError::NoCredentials);
        }

        // Take a shared lock for reading
        let file = OpenOptions::new().read(true).open(path)?;
        // Best effort lock; on some FS this may not be supported.
        let _guard = FileLockGuard::new_shared(&file)?;

        let mut contents = String::new();
        {
            let mut reader = (&file).take(1_000_000); // safety
            reader.read_to_string(&mut contents)?;
        }
        drop(_guard);

        let map: HashMap<String, AuthEntry> = serde_json::from_str(&contents)?;

        // Choose the entry that matches our configured OIDC issuer/client if possible,
        // otherwise the first one that looks like an xAI session.
        let preferred = format!(
            "{}::{}",
            self.config
                .oidc_issuer
                .as_deref()
                .unwrap_or("https://auth.x.ai"),
            self.config.oidc_client_id.as_deref().unwrap_or_default()
        );

        if let Some(entry) = map.get(&preferred) {
            return Ok((preferred, entry.clone()));
        }

        // Fallback: any entry that has a refresh_token or looks like xAI
        for (k, v) in &map {
            if v.refresh_token.is_some() || k.contains("auth.x.ai") {
                return Ok((k.clone(), v.clone()));
            }
        }

        // Last resort: first entry
        if let Some((k, v)) = map.into_iter().next() {
            return Ok((k, v));
        }

        Err(AuthError::NoCredentials)
    }

    async fn save_to_disk_locked(&self, issuer_key: &str, entry: &AuthEntry) -> Result<()> {
        let path = &self.path;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Read existing so we can merge (preserve other issuers)
        let mut map: HashMap<String, AuthEntry> = if path.exists() {
            let file = OpenOptions::new().read(true).open(path)?;
            let _guard = FileLockGuard::new_exclusive(&file)?; // upgrade to exclusive for write
            let mut s = String::new();
            {
                let mut reader = (&file).take(2_000_000);
                reader.read_to_string(&mut s)?;
            }
            drop(_guard);
            serde_json::from_str(&s).unwrap_or_default()
        } else {
            HashMap::new()
        };

        map.insert(issuer_key.to_string(), entry.clone());

        // Atomic write: write to .tmp then rename
        let tmp = path.with_extension("json.tmp");
        {
            let tmp_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;

            let _guard = FileLockGuard::new_exclusive(&tmp_file)?;
            let pretty = serde_json::to_string_pretty(&map)?;
            let mut writer = &tmp_file;
            writer.write_all(pretty.as_bytes())?;
            tmp_file.sync_all()?;
            drop(_guard);
        }

        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Force a re-login flow (used by `grok-auth login` style commands).
    pub async fn force_login(&self) -> Result<AuthToken> {
        // For now we delegate to the login module. The store just receives the result.
        let entry = crate::login::run_interactive_login(&self.config).await?;
        let issuer_key = self.preferred_issuer_key();
        self.save_to_disk_locked(&issuer_key, &entry).await?;

        let mut guard = self.inner.write().await;
        guard.current = Some((issuer_key.clone(), entry.clone()));
        Ok(AuthToken::from((issuer_key, entry)))
    }

    fn preferred_issuer_key(&self) -> String {
        format!(
            "{}::{}",
            self.config
                .oidc_issuer
                .as_deref()
                .unwrap_or("https://auth.x.ai"),
            self.config.oidc_client_id.as_deref().unwrap_or("default")
        )
    }
}

/// Small RAII guard for fs2 locks.
struct FileLockGuard<'a> {
    file: &'a File,
}

impl<'a> FileLockGuard<'a> {
    fn new_shared(file: &'a File) -> Result<Self> {
        file.lock_shared()
            .map_err(|e| AuthError::Lock(e.to_string()))?;
        Ok(Self { file })
    }

    fn new_exclusive(file: &'a File) -> Result<Self> {
        file.lock_exclusive()
            .map_err(|e| AuthError::Lock(e.to_string()))?;
        Ok(Self { file })
    }
}

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
