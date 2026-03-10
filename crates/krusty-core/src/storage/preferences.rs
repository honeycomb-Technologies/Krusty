//! User preferences storage

use anyhow::Result;
use rusqlite::params;

use crate::ai::models::{
    dynamic_model_cache_ttl, model_catalog_fingerprint, DynamicModelCacheMetadata, ModelMetadata,
};
use crate::ai::providers::ProviderId;
use crate::tools::git_identity::GitIdentity;

use super::{database::Database, unix_timestamp};

/// User preferences manager
pub struct Preferences {
    db: Database,
    user_id: Option<String>,
}

impl Preferences {
    /// Create preferences manager with existing database (single-tenant mode)
    pub fn new(db: Database) -> Self {
        Self { db, user_id: None }
    }

    /// Create preferences manager for a specific user (multi-tenant mode)
    pub fn for_user(db: Database, user_id: &str) -> Self {
        Self {
            db,
            user_id: Some(user_id.to_string()),
        }
    }

    /// Get a preference value
    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(uid) = self.user_id.as_deref() {
            self.db
                .conn()
                .query_row(
                    "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = ?2",
                    params![uid, key],
                    |row| row.get(0),
                )
                .ok()
        } else {
            self.db
                .conn()
                .query_row(
                    "SELECT value FROM user_preferences WHERE user_id IS NULL AND key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .ok()
        }
    }

    /// Set a preference value
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let user_id = self.user_id.as_deref();
        self.db.conn().execute(
            "INSERT INTO user_preferences (key, value, updated_at, user_id)
             VALUES (?1, ?2, strftime('%s', 'now'), ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = strftime('%s', 'now'), user_id = ?3",
            params![key, value, user_id],
        )?;
        Ok(())
    }

    /// Delete a preference
    pub fn delete(&self, key: &str) -> Result<()> {
        if let Some(uid) = self.user_id.as_deref() {
            self.db.conn().execute(
                "DELETE FROM user_preferences WHERE user_id = ?1 AND key = ?2",
                params![uid, key],
            )?;
        } else {
            self.db.conn().execute(
                "DELETE FROM user_preferences WHERE user_id IS NULL AND key = ?1",
                [key],
            )?;
        }

        Ok(())
    }

    /// Get theme name (defaults to "krusty")
    pub fn get_theme(&self) -> String {
        self.get("theme").unwrap_or_else(|| "krusty".to_string())
    }

    /// Save theme name
    pub fn set_theme(&self, theme: &str) -> Result<()> {
        self.set("theme", theme)
    }

    /// Get recently used model IDs
    pub fn get_recent_models(&self) -> Vec<String> {
        self.get("recent_models")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save recently used model IDs
    pub fn set_recent_models(&self, models: &[String]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set("recent_models", &json)
    }

    /// Add a model to recent list (moves to front if exists)
    pub fn add_recent_model(&self, model_id: &str) -> Result<()> {
        let mut recent = self.get_recent_models();

        // Remove if exists
        recent.retain(|id| id != model_id);

        // Add at front
        recent.insert(0, model_id.to_string());

        // Keep max 10
        recent.truncate(10);

        self.set_recent_models(&recent)
    }

    /// Get current model ID (None if never set)
    pub fn get_current_model(&self) -> Option<String> {
        self.get("current_model")
    }

    /// Save current model ID
    pub fn set_current_model(&self, model_id: &str) -> Result<()> {
        self.set("current_model", model_id)
    }

    /// Get cached OpenRouter models
    pub fn get_cached_openrouter_models(&self) -> Option<Vec<ModelMetadata>> {
        self.get_cached_models(ProviderId::OpenRouter)
    }

    /// Cache OpenRouter models
    pub fn cache_openrouter_models(&self, models: &[ModelMetadata]) -> Result<()> {
        self.cache_models(ProviderId::OpenRouter, models)
    }

    /// Check if OpenRouter cache is stale (>24 hours old)
    pub fn is_openrouter_cache_stale(&self) -> bool {
        self.is_model_cache_stale(ProviderId::OpenRouter)
    }

    /// Get cached models for any dynamic provider.
    pub fn get_cached_models(&self, provider: ProviderId) -> Option<Vec<ModelMetadata>> {
        self.get(&format!("{}_models_cache", provider.storage_key()))
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Get cache policy metadata for a dynamic provider model catalog.
    pub fn get_model_cache_metadata(
        &self,
        provider: ProviderId,
    ) -> Option<DynamicModelCacheMetadata> {
        self.get(&format!("{}_models_cache_meta", provider.storage_key()))
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Cache models for a dynamic provider.
    pub fn cache_models(&self, provider: ProviderId, models: &[ModelMetadata]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set(&format!("{}_models_cache", provider.storage_key()), &json)?;
        let metadata = DynamicModelCacheMetadata {
            fetched_at: unix_timestamp(),
            ttl_seconds: dynamic_model_cache_ttl(provider),
            model_count: models.len(),
            fingerprint: model_catalog_fingerprint(models),
        };
        self.set(
            &format!("{}_models_cache_meta", provider.storage_key()),
            &serde_json::to_string(&metadata)?,
        )?;
        self.set(
            &format!("{}_models_cached_at", provider.storage_key()),
            &metadata.fetched_at.to_string(),
        )
    }

    /// Get persisted custom/manual models for a provider.
    pub fn get_custom_models(&self, provider: ProviderId) -> Vec<ModelMetadata> {
        self.get(&format!("{}_custom_models", provider.storage_key()))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist the full custom/manual model list for a provider.
    pub fn set_custom_models(&self, provider: ProviderId, models: &[ModelMetadata]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set(&format!("{}_custom_models", provider.storage_key()), &json)
    }

    /// Save or replace a custom/manual model for its provider.
    pub fn save_custom_model(&self, model: &ModelMetadata) -> Result<()> {
        let mut models = self.get_custom_models(model.provider);
        models.retain(|existing| existing.id != model.id);
        models.insert(0, model.clone());
        models.truncate(32);
        self.set_custom_models(model.provider, &models)
    }

    /// Check if a provider's cached model list is stale (>24 hours old).
    pub fn is_model_cache_stale(&self, provider: ProviderId) -> bool {
        if let Some(metadata) = self.get_model_cache_metadata(provider) {
            if metadata.fetched_at == 0
                || metadata.ttl_seconds == 0
                || (unix_timestamp() - metadata.fetched_at) > metadata.ttl_seconds
            {
                return true;
            }

            let Some(cached_models) = self.get_cached_models(provider) else {
                return true;
            };

            return metadata.model_count != cached_models.len()
                || metadata.fingerprint != model_catalog_fingerprint(&cached_models);
        }

        let cached_at: u64 = self
            .get(&format!("{}_models_cached_at", provider.storage_key()))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        cached_at == 0 || (unix_timestamp() - cached_at) > 86400
    }

    /// Get active plugin ID
    pub fn get_active_plugin(&self) -> Option<String> {
        self.get("active_plugin")
    }

    /// Save active plugin ID
    pub fn set_active_plugin(&self, plugin_id: &str) -> Result<()> {
        self.set("active_plugin", plugin_id)
    }

    /// Get git identity configuration (defaults to CoAuthor mode)
    pub fn get_git_identity(&self) -> GitIdentity {
        self.get("git_identity")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save git identity configuration
    pub fn set_git_identity(&self, identity: &GitIdentity) -> Result<()> {
        let json = serde_json::to_string(identity)?;
        self.set("git_identity", &json)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::storage::database::Database;

    fn create_preferences() -> (Preferences, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("prefs.db");
        let db = Database::new(&db_path).expect("database");
        (Preferences::new(db), temp_dir)
    }

    #[test]
    fn caches_dynamic_model_metadata_with_provider_ttl() {
        let (prefs, _temp) = create_preferences();
        let models = vec![ModelMetadata::new(
            "gpt-5-codex",
            "GPT-5 Codex",
            ProviderId::OpenAI,
        )];

        prefs.cache_models(ProviderId::OpenAI, &models).unwrap();

        let metadata = prefs.get_model_cache_metadata(ProviderId::OpenAI).unwrap();
        assert_eq!(
            metadata.ttl_seconds,
            dynamic_model_cache_ttl(ProviderId::OpenAI)
        );
        assert_eq!(metadata.model_count, 1);
        assert_eq!(metadata.fingerprint, model_catalog_fingerprint(&models));
    }

    #[test]
    fn invalid_cache_metadata_marks_model_cache_stale() {
        let (prefs, _temp) = create_preferences();
        let models = vec![ModelMetadata::new(
            "gpt-5-codex",
            "GPT-5 Codex",
            ProviderId::OpenAI,
        )];

        prefs.cache_models(ProviderId::OpenAI, &models).unwrap();

        let broken = DynamicModelCacheMetadata {
            fetched_at: unix_timestamp(),
            ttl_seconds: dynamic_model_cache_ttl(ProviderId::OpenAI),
            model_count: 99,
            fingerprint: 0,
        };
        prefs
            .set(
                "openai_models_cache_meta",
                &serde_json::to_string(&broken).unwrap(),
            )
            .unwrap();

        assert!(prefs.is_model_cache_stale(ProviderId::OpenAI));
    }
}
