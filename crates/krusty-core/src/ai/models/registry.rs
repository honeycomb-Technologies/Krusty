use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::super::providers::ProviderId;
use super::{ModelMetadata, OrganizedModels};

/// Central model registry
///
/// Thread-safe store for all models from all providers.
/// Supports both static (built-in) and dynamic (fetched) models.
pub struct ModelRegistry {
    /// All models indexed by provider
    models: RwLock<HashMap<ProviderId, Vec<ModelMetadata>>>,

    /// Index for O(1) model lookup by ID -> (provider, index in provider's vec)
    model_index: RwLock<HashMap<String, (ProviderId, usize)>>,

    /// Recently used model IDs (most recent first)
    recent_ids: RwLock<Vec<String>>,

    /// Maximum recent models to track
    max_recent: usize,
}

impl ModelRegistry {
    /// Create new empty registry
    pub fn new() -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
            model_index: RwLock::new(HashMap::new()),
            recent_ids: RwLock::new(Vec::new()),
            max_recent: 10,
        }
    }

    /// Set models for a provider (replaces existing)
    pub async fn set_models(&self, provider: ProviderId, models: Vec<ModelMetadata>) {
        // A successful-but-empty discovery response must not erase the last
        // known-good or curated fallback catalog. Provider outages and schema
        // changes occasionally deserialize to an empty list.
        if models.is_empty() {
            tracing::warn!(?provider, "Ignoring empty model catalog refresh");
            return;
        }

        let mut seen = std::collections::HashSet::new();
        let models = models
            .into_iter()
            .filter(|model| seen.insert(model.id.clone()))
            .collect::<Vec<_>>();
        let mut all_models = self.models.write().await;
        let mut index = self.model_index.write().await;

        index.retain(|_, (p, _)| *p != provider);

        for (idx, model) in models.iter().enumerate() {
            index.insert(model.id.clone(), (provider, idx));
        }

        all_models.insert(provider, models);
    }

    /// Insert or replace a single model entry for its provider.
    ///
    /// This keeps custom/manual model IDs first-class in the registry so
    /// recents and context-window lookup behave consistently.
    pub async fn upsert_model(&self, metadata: ModelMetadata) {
        let provider = metadata.provider;
        let model_id = metadata.id.clone();

        let mut all_models = self.models.write().await;
        let mut index = self.model_index.write().await;
        let provider_models = all_models.entry(provider).or_default();

        if let Some((existing_provider, existing_idx)) = index.get(&model_id).copied() {
            if existing_provider == provider {
                if let Some(slot) = provider_models.get_mut(existing_idx) {
                    *slot = metadata;
                    return;
                }
            }
        }

        let insert_idx = provider_models.len();
        provider_models.push(metadata);
        index.insert(model_id, (provider, insert_idx));
    }

    /// Check if we have models for a provider
    pub async fn has_models(&self, provider: ProviderId) -> bool {
        let models = self.models.read().await;
        models
            .get(&provider)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    }

    /// Get a specific model by ID - O(1) lookup via index
    pub async fn get_model(&self, model_id: &str) -> Option<ModelMetadata> {
        let index = self.model_index.read().await;
        let (provider, idx) = index.get(model_id)?;

        let models = self.models.read().await;
        models.get(provider).and_then(|v| v.get(*idx)).cloned()
    }

    /// Get a specific model by ID (non-blocking, for use in sync contexts like rendering)
    /// Returns None if lock is contended or model not found - O(1) lookup via index
    pub fn try_get_model(&self, model_id: &str) -> Option<ModelMetadata> {
        let index = self.model_index.try_read().ok()?;
        let (provider, idx) = index.get(model_id)?;

        let models = self.models.try_read().ok()?;
        models.get(provider).and_then(|v| v.get(*idx)).cloned()
    }

    /// Record a model as recently used
    pub async fn mark_recent(&self, model_id: &str) {
        let mut recent = self.recent_ids.write().await;

        recent.retain(|id| id != model_id);
        recent.insert(0, model_id.to_string());
        recent.truncate(self.max_recent);
    }

    /// Set recent model IDs (for loading from preferences)
    pub async fn set_recent_ids(&self, ids: Vec<String>) {
        let mut recent = self.recent_ids.write().await;
        *recent = ids;
        recent.truncate(self.max_recent);
    }

    /// Get models organized for display - O(n) for recent models via index
    /// Returns: (recent_models, models_by_provider)
    pub async fn get_organized_models(
        &self,
        configured_providers: &[ProviderId],
    ) -> OrganizedModels {
        let models = self.models.read().await;
        let index = self.model_index.read().await;
        let recent_ids = self.recent_ids.read().await;

        let recent_models: Vec<ModelMetadata> = recent_ids
            .iter()
            .filter_map(|id| {
                let (provider, idx) = index.get(id)?;
                let model = models.get(provider)?.get(*idx)?;
                if configured_providers.contains(&model.provider) {
                    Some(model.clone())
                } else {
                    None
                }
            })
            .collect();

        let mut by_provider = HashMap::new();
        for provider in configured_providers {
            if let Some(provider_models) = models.get(provider) {
                if !provider_models.is_empty() {
                    by_provider.insert(*provider, provider_models.clone());
                }
            }
        }

        (recent_models, by_provider)
    }

    /// Get models organized for display (non-blocking) - O(n) for recent models via index
    /// Returns None if locks are contended
    pub fn try_get_organized_models(
        &self,
        configured_providers: &[ProviderId],
    ) -> Option<OrganizedModels> {
        let models = self.models.try_read().ok()?;
        let index = self.model_index.try_read().ok()?;
        let recent_ids = self.recent_ids.try_read().ok()?;

        let recent_models: Vec<ModelMetadata> = recent_ids
            .iter()
            .filter_map(|id| {
                let (provider, idx) = index.get(id)?;
                let model = models.get(provider)?.get(*idx)?;
                if configured_providers.contains(&model.provider) {
                    Some(model.clone())
                } else {
                    None
                }
            })
            .collect();

        let mut by_provider = HashMap::new();
        for provider in configured_providers {
            if let Some(provider_models) = models.get(provider) {
                if !provider_models.is_empty() {
                    by_provider.insert(*provider, provider_models.clone());
                }
            }
        }

        Some((recent_models, by_provider))
    }

    /// Check if provider has models (non-blocking)
    pub fn try_has_models(&self, provider: ProviderId) -> Option<bool> {
        let models = self.models.try_read().ok()?;
        Some(
            models
                .get(&provider)
                .map(|m| !m.is_empty())
                .unwrap_or(false),
        )
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared model registry type
pub type SharedModelRegistry = Arc<ModelRegistry>;

/// Create a new shared model registry
pub fn create_model_registry() -> SharedModelRegistry {
    Arc::new(ModelRegistry::new())
}
