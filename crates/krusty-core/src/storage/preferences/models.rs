use anyhow::Result;

use crate::ai::models::{
    dynamic_model_cache_ttl, model_catalog_fingerprint, DynamicModelCacheMetadata, ModelMetadata,
};
use crate::ai::providers::ProviderId;
use crate::storage::unix_timestamp;

use super::core::Preferences;

impl Preferences {
    pub fn get_recent_models(&self) -> Vec<String> {
        self.get("recent_models")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn set_recent_models(&self, models: &[String]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set("recent_models", &json)
    }

    pub fn add_recent_model(&self, model_id: &str) -> Result<()> {
        let mut recent = self.get_recent_models();
        recent.retain(|id| id != model_id);
        recent.insert(0, model_id.to_string());
        recent.truncate(10);
        self.set_recent_models(&recent)
    }

    pub fn get_current_model(&self) -> Option<String> {
        self.get("current_model")
    }

    pub fn set_current_model(&self, model_id: &str) -> Result<()> {
        self.set("current_model", model_id)
    }

    pub fn get_cached_openrouter_models(&self) -> Option<Vec<ModelMetadata>> {
        self.get_cached_models(ProviderId::OpenRouter)
    }

    pub fn cache_openrouter_models(&self, models: &[ModelMetadata]) -> Result<()> {
        self.cache_models(ProviderId::OpenRouter, models)
    }

    pub fn is_openrouter_cache_stale(&self) -> bool {
        self.is_model_cache_stale(ProviderId::OpenRouter)
    }

    pub fn get_cached_models(&self, provider: ProviderId) -> Option<Vec<ModelMetadata>> {
        self.get(&format!("{}_models_cache", provider.storage_key()))
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn get_model_cache_metadata(
        &self,
        provider: ProviderId,
    ) -> Option<DynamicModelCacheMetadata> {
        self.get(&format!("{}_models_cache_meta", provider.storage_key()))
            .and_then(|s| serde_json::from_str(&s).ok())
    }

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

    /// Remove every persisted snapshot marker for a provider.
    ///
    /// Catalogs are entitlement-scoped. Credential rotation or revocation must
    /// not allow a prior account's model list to be restored on the next start.
    pub fn clear_model_cache(&self, provider: ProviderId) -> Result<()> {
        let prefix = provider.storage_key();
        self.delete(&format!("{prefix}_models_cache"))?;
        self.delete(&format!("{prefix}_models_cache_meta"))?;
        self.delete(&format!("{prefix}_models_cached_at"))
    }

    pub fn get_custom_models(&self, provider: ProviderId) -> Vec<ModelMetadata> {
        self.get(&format!("{}_custom_models", provider.storage_key()))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn set_custom_models(&self, provider: ProviderId, models: &[ModelMetadata]) -> Result<()> {
        let json = serde_json::to_string(models)?;
        self.set(&format!("{}_custom_models", provider.storage_key()), &json)
    }

    pub fn save_custom_model(&self, model: &ModelMetadata) -> Result<()> {
        let mut models = self.get_custom_models(model.provider);
        models.retain(|existing| existing.id != model.id);
        models.insert(0, model.clone());
        models.truncate(32);
        self.set_custom_models(model.provider, &models)
    }

    pub fn is_model_cache_stale(&self, provider: ProviderId) -> bool {
        if let Some(metadata) = self.get_model_cache_metadata(provider) {
            if metadata.fetched_at == 0
                || metadata.ttl_seconds == 0
                || unix_timestamp().saturating_sub(metadata.fetched_at) > metadata.ttl_seconds
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
        cached_at == 0 || unix_timestamp().saturating_sub(cached_at) > 86400
    }
}
