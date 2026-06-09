//! Model fetching handlers
//!
//! Async model fetching from dynamic providers such as OpenRouter and OpenAI.

use crate::ai::client::CallOptions;
use crate::ai::models::ModelMetadata;
use crate::ai::providers::ProviderId;
use crate::tui::app::App;
use crate::tui::utils::DynamicModelUpdate;

impl App {
    pub fn apply_model_selection(&mut self, metadata: ModelMetadata, is_custom: bool) {
        let provider_id = metadata.provider;
        let model_id = metadata.id.clone();

        if provider_id != self.runtime.active_provider {
            self.switch_provider(provider_id);
            if !self.is_authenticated() {
                let _ = futures::executor::block_on(self.try_load_auth());
            }
        }

        self.runtime.current_model = model_id.clone();

        let auth = self.resolve_auth_for_active_provider();
        self.runtime.api_key = auth.clone();
        self.runtime.ai_client = auth.map(|key| {
            let config = self.create_client_config();
            crate::ai::client::AiClient::with_api_key(config, key)
        });

        let registry = self.services.model_registry.clone();
        futures::executor::block_on(async {
            registry.upsert_model(metadata.clone()).await;
            registry.mark_recent(&model_id).await;
        });

        if let Some(ref prefs) = self.services.preferences {
            if let Err(e) = prefs.set_current_model(&model_id) {
                tracing::warn!("Failed to save current model: {}", e);
            }
            if let Err(e) = prefs.add_recent_model(&model_id) {
                tracing::warn!("Failed to save recent model: {}", e);
            }
            if is_custom {
                if let Err(e) = prefs.save_custom_model(&metadata) {
                    tracing::warn!("Failed to persist custom model metadata: {}", e);
                }
            }
        }
    }

    pub fn toggle_fast_mode(&mut self) -> Option<bool> {
        let supports_fast_mode = CallOptions {
            fast_mode: true,
            ..Default::default()
        }
        .service_tier_for_provider(self.runtime.active_provider)
        .is_some();
        if !supports_fast_mode {
            return None;
        }

        self.runtime.fast_mode = !self.runtime.fast_mode;
        Some(self.runtime.fast_mode)
    }

    /// Start async fetch of models for a dynamic provider.
    pub fn start_dynamic_model_fetch(&mut self, provider: ProviderId) {
        if !crate::ai::catalog::supports_dynamic_models(provider) {
            return;
        }

        if self.runtime.channels.dynamic_models.is_some() {
            return;
        }

        if !self.runtime.dynamic_model_fetches.insert(provider) {
            return;
        }

        let credential = crate::ai::catalog::credential_for_dynamic_models(
            provider,
            &self.services.credential_store,
        );

        let Some(credential) = credential else {
            tracing::warn!(
                "Cannot fetch {:?} models: no credential configured",
                provider
            );
            self.runtime.dynamic_model_fetches.remove(&provider);
            return;
        };

        self.ui.popups.model.set_loading(true);

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.dynamic_models = Some(rx);

        let registry = self.services.model_registry.clone();

        tokio::spawn(async move {
            let result = crate::ai::catalog::fetch_dynamic_models(provider, &credential).await;

            if let Ok(models) = &result {
                registry.set_models(provider, models.clone()).await;
                tracing::info!("Fetched {} {:?} models", models.len(), provider);
            } else if let Err(error) = &result {
                tracing::error!("Failed to fetch {:?} models: {}", provider, error);
            }

            let _ = tx.send(DynamicModelUpdate {
                provider,
                result: result.map_err(|e| e.to_string()),
            });
        });
    }

    /// Poll for dynamic model fetch completion.
    pub fn poll_dynamic_model_fetch(&mut self) {
        let Some(rx) = &mut self.runtime.channels.dynamic_models else {
            return;
        };

        match rx.try_recv() {
            Ok(update) => {
                self.runtime.dynamic_model_fetches.remove(&update.provider);
                match update.result {
                    Ok(models) => {
                        self.cache_dynamic_models(update.provider, &models);
                        self.reapply_custom_models(update.provider);
                        self.refresh_model_popup();
                    }
                    Err(error) => {
                        self.ui.popups.model.set_error(error);
                    }
                }
                self.runtime.channels.dynamic_models = None;
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.runtime.dynamic_model_fetches.clear();
                self.runtime.channels.dynamic_models = None;
            }
        }
    }

    fn cache_dynamic_models(&self, provider: ProviderId, models: &[ModelMetadata]) {
        let Some(ref prefs) = self.services.preferences else {
            return;
        };

        if let Err(error) = prefs.cache_models(provider, models) {
            tracing::warn!("Failed to cache {:?} models: {}", provider, error);
        }
    }

    fn reapply_custom_models(&self, provider: ProviderId) {
        let Some(ref prefs) = self.services.preferences else {
            return;
        };

        let custom_models = prefs.get_custom_models(provider);
        if custom_models.is_empty() {
            return;
        }

        let registry = self.services.model_registry.clone();
        futures::executor::block_on(async move {
            for metadata in custom_models {
                registry.upsert_model(metadata).await;
            }
        });
    }

    pub fn should_refresh_dynamic_models(&self, provider: ProviderId) -> bool {
        if !crate::ai::catalog::supports_dynamic_models(provider) {
            return false;
        }

        let has_models = self
            .services
            .model_registry
            .try_has_models(provider)
            .unwrap_or(false);

        if !has_models {
            return true;
        }

        self.services
            .preferences
            .as_ref()
            .map(|prefs| prefs.is_model_cache_stale(provider))
            .unwrap_or(true)
    }

    /// Refresh model popup with current registry data.
    pub fn refresh_model_popup(&mut self) {
        let configured = self.configured_providers();

        if let Some((recent_models, models_by_provider)) = self
            .services
            .model_registry
            .try_get_organized_models(&configured)
        {
            let models_vec: Vec<_> = ProviderId::all()
                .iter()
                .filter_map(|id| {
                    models_by_provider
                        .get(id)
                        .map(|models| (*id, models.clone()))
                })
                .collect();

            self.ui.popups.model.set_models(recent_models, models_vec);
        }
    }
}
