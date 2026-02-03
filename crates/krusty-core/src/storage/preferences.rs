//! User preferences storage

use anyhow::Result;
use rusqlite::params;

use crate::ai::models::ModelMetadata;

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

    /// Build WHERE clause and params for user filtering
    fn user_filter(&self) -> (&'static str, Vec<String>) {
        if let Some(ref uid) = self.user_id {
            ("WHERE user_id = ?1", vec![uid.clone()])
        } else {
            ("WHERE user_id IS NULL", vec![])
        }
    }

    /// Get a preference value
    pub fn get(&self, key: &str) -> Option<String> {
        let (where_clause, filter_params) = self.user_filter();

        let sql = format!(
            "SELECT value FROM user_preferences {} AND key = ?2",
            where_clause
        );

        let mut params: Vec<&dyn rusqlite::ToSql> = filter_params
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        params.push(&key);

        self.db
            .conn()
            .query_row(&sql, params.as_slice(), |row| row.get(0))
            .ok()
    }

    /// Set a preference value
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.db.conn().execute(
            "INSERT INTO user_preferences (key, value, updated_at, user_id)
             VALUES (?1, ?2, strftime('%s', 'now'), ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = strftime('%s', 'now'), user_id = ?3",
            params![key, value, self.user_id],
        )?;
        Ok(())
    }

    /// Delete a preference
    pub fn delete(&self, key: &str) -> Result<()> {
        let (where_clause, filter_params) = self.user_filter();
        let sql = format!("DELETE FROM user_preferences {} AND key = ?2", where_clause);

        let mut params: Vec<&dyn rusqlite::ToSql> = filter_params
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        params.push(&key);

        self.db.conn().execute(&sql, params.as_slice())?;
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

    /// Get current model ID (defaults to Claude Opus 4.5)
    pub fn get_current_model(&self) -> String {
        self.get("current_model")
            .unwrap_or_else(|| "claude-opus-4-5-20251101".to_string())
    }

    /// Save current model ID
    pub fn set_current_model(&self, model_id: &str) -> Result<()> {
        self.set("current_model", model_id)
    }

    /// Get cached OpenRouter models
    pub fn get_cached_openrouter_models(&self) -> Option<Vec<ModelMetadata>> {
        self.get("openrouter_models_cache")
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Cache OpenRouter models
    pub fn cache_openrouter_models(&self, models: &[ModelMetadata]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set("openrouter_models_cache", &json)?;
        self.set("openrouter_models_cached_at", &unix_timestamp().to_string())
    }

    /// Check if OpenRouter cache is stale (>24 hours old)
    pub fn is_openrouter_cache_stale(&self) -> bool {
        let cached_at: u64 = self
            .get("openrouter_models_cached_at")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        cached_at == 0 || (unix_timestamp() - cached_at) > 86400
    }

    /// Get cached OpenCode Zen models
    pub fn get_cached_opencodezen_models(&self) -> Option<Vec<ModelMetadata>> {
        self.get("opencodezen_models_cache")
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Cache OpenCode Zen models
    pub fn cache_opencodezen_models(&self, models: &[ModelMetadata]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set("opencodezen_models_cache", &json)?;
        self.set(
            "opencodezen_models_cached_at",
            &unix_timestamp().to_string(),
        )
    }

    /// Check if OpenCode Zen cache is stale (>24 hours old)
    pub fn is_opencodezen_cache_stale(&self) -> bool {
        let cached_at: u64 = self
            .get("opencodezen_models_cached_at")
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
}
