//! Provider model cache for ACP
//!
//! Caches provider configurations and model information to avoid repeated lookups.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::ai::providers::{get_provider, ProviderId};

/// Cached provider model information
#[derive(Debug, Clone)]
pub struct CachedProviderInfo {
    pub provider_id: ProviderId,
    pub base_url: String,
    pub default_model: String,
    pub models: Vec<String>,
}

/// Provider model cache
pub struct ModelManager {
    /// Cached provider information to avoid repeated get_provider() calls
    provider_cache: Arc<RwLock<HashMap<ProviderId, CachedProviderInfo>>>,
}

impl ModelManager {
    /// Create a new model manager
    pub fn new() -> Self {
        Self {
            provider_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get provider information, using cache if available
    pub async fn get_provider(&self, provider_id: ProviderId) -> Option<CachedProviderInfo> {
        // Check cache first
        {
            let cache_read = self.provider_cache.read().await;
            if let Some(cached) = cache_read.get(&provider_id) {
                debug!("Using cached provider info for {:?}", provider_id);
                return Some(cached.clone());
            }
        }

        // Cache miss - fetch and cache
        debug!("Fetching provider info for {:?} (cache miss)", provider_id);
        let provider_config = get_provider(provider_id)?;

        let info = CachedProviderInfo {
            provider_id,
            base_url: provider_config.base_url.clone(),
            default_model: provider_config.default_model().to_string(),
            models: provider_config
                .models
                .iter()
                .map(|m| m.id.clone())
                .collect(),
        };

        // Update cache
        {
            let mut cache_write = self.provider_cache.write().await;
            cache_write.insert(provider_id, info.clone());
        }

        Some(info)
    }

    /// Get default model for a provider
    pub async fn get_default_model(&self, provider_id: ProviderId) -> Option<String> {
        self.get_provider(provider_id)
            .await
            .map(|info| info.default_model)
    }

    /// Get base URL for a provider
    pub async fn get_base_url(&self, provider_id: ProviderId) -> Option<String> {
        self.get_provider(provider_id)
            .await
            .map(|info| info.base_url)
    }

    /// Invalidate cache for a specific provider
    pub async fn invalidate_provider(&self, provider_id: ProviderId) {
        let mut cache_write = self.provider_cache.write().await;
        cache_write.remove(&provider_id);
        debug!("Invalidated cache for provider {:?}", provider_id);
    }

    /// Invalidate entire cache
    pub async fn invalidate_all(&self) {
        let mut cache_write = self.provider_cache.write().await;
        cache_write.clear();
        debug!("Invalidated entire model cache");
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_model_manager_caching() {
        let manager = ModelManager::new();

        // First call should cache
        let info1 = manager.get_provider(ProviderId::MiniMax).await;
        assert!(info1.is_some());

        // Second call should use cache
        let info2 = manager.get_provider(ProviderId::MiniMax).await;
        assert!(info2.is_some());

        // Both should return the same info
        assert_eq!(info1.unwrap().default_model, info2.unwrap().default_model);
    }

    #[tokio::test]
    async fn test_model_manager_invalidation() {
        let manager = ModelManager::new();

        // Cache a provider
        manager.get_provider(ProviderId::MiniMax).await;
        assert!(manager
            .provider_cache
            .read()
            .await
            .contains_key(&ProviderId::MiniMax));

        // Invalidate it
        manager.invalidate_provider(ProviderId::MiniMax).await;
        assert!(!manager
            .provider_cache
            .read()
            .await
            .contains_key(&ProviderId::MiniMax));
    }
}
