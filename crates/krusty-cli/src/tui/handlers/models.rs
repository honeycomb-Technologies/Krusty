//! Model fetching handlers
//!
//! Async model fetching from dynamic providers (OpenRouter, OpenAI, Grok, …).
//! Catalog refresh is aligned with the server: any provider with live models
//! and valid catalog credentials can be refreshed, including concurrently.
//!
//! Registry mutations happen on the async fetch tasks — never `block_on` on the
//! TUI poll path — so model refresh cannot hitch the terminal event loop.

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
        let metadata_for_registry = metadata.clone();
        // User-initiated selection can afford a short sync apply; still avoid
        // holding locks across the whole UI frame by spawning when possible.
        futures::executor::block_on(async {
            registry.upsert_model(metadata_for_registry).await;
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

    /// Refresh every dynamic provider that has catalog credentials and a stale cache.
    ///
    /// Mirrors server `initialize_models` so CLI and server model pickers stay aligned.
    pub fn refresh_stale_dynamic_model_catalogs(&mut self) {
        for provider in crate::ai::catalog::dynamic_model_providers() {
            if !self.should_refresh_dynamic_models(provider) {
                continue;
            }
            if crate::ai::catalog::credential_for_dynamic_models(
                provider,
                &self.services.credential_store,
            )
            .is_none()
            {
                continue;
            }
            self.start_dynamic_model_fetch(provider);
        }
    }

    /// Start async fetch of models for a dynamic provider.
    pub fn start_dynamic_model_fetch(&mut self, provider: ProviderId) {
        if !crate::ai::catalog::supports_dynamic_models(provider) {
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

        let custom_models = self
            .services
            .preferences
            .as_ref()
            .map(|prefs| prefs.get_custom_models(provider))
            .unwrap_or_default();

        let tx = self.ensure_dynamic_model_tx();
        self.ui.popups.model.set_loading(true);

        let registry = self.services.model_registry.clone();
        tokio::spawn(async move {
            let result = crate::ai::catalog::fetch_dynamic_models(provider, &credential).await;

            match &result {
                Ok(models) => {
                    registry.set_models(provider, models.clone()).await;
                    for metadata in custom_models {
                        registry.upsert_model(metadata).await;
                    }
                    tracing::info!("Fetched {} {:?} models", models.len(), provider);
                }
                Err(error) => {
                    tracing::error!("Failed to fetch {:?} models: {}", provider, error);
                }
            }

            let _ = tx.send(DynamicModelUpdate {
                provider,
                result: result.map_err(|e| e.to_string()),
            });
        });
    }

    fn ensure_dynamic_model_tx(
        &mut self,
    ) -> tokio::sync::mpsc::UnboundedSender<DynamicModelUpdate> {
        if let Some(tx) = &self.runtime.channels.dynamic_models_tx {
            return tx.clone();
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.runtime.channels.dynamic_models = Some(rx);
        self.runtime.channels.dynamic_models_tx = Some(tx.clone());
        tx
    }

    /// Poll for dynamic model fetch completion.
    pub fn poll_dynamic_model_fetch(&mut self) {
        let Some(rx) = &mut self.runtime.channels.dynamic_models else {
            return;
        };

        let mut received = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(update) => received.push(update),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.runtime.dynamic_model_fetches.clear();
                    self.runtime.channels.dynamic_models = None;
                    self.runtime.channels.dynamic_models_tx = None;
                    self.ui.popups.model.set_loading(false);
                    return;
                }
            }
        }

        if received.is_empty() {
            return;
        }

        let mut any_success = false;
        let mut last_error: Option<String> = None;

        for update in received {
            self.runtime.dynamic_model_fetches.remove(&update.provider);
            match update.result {
                Ok(models) => {
                    // Registry already updated on the async task — only cache + UI here.
                    self.cache_dynamic_models(update.provider, &models);
                    any_success = true;
                }
                Err(error) => {
                    last_error = Some(format!("{:?}: {error}", update.provider));
                }
            }
        }

        if any_success {
            self.refresh_model_popup();
        }

        if self.runtime.dynamic_model_fetches.is_empty() {
            self.ui.popups.model.set_loading(false);
            if let Some(error) = last_error {
                if !any_success {
                    self.ui.popups.model.set_error(error);
                } else {
                    tracing::warn!("Partial dynamic model refresh failure: {error}");
                }
            }
            // Drop channel halves when idle so the next refresh recreates cleanly.
            self.runtime.channels.dynamic_models = None;
            self.runtime.channels.dynamic_models_tx = None;
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
