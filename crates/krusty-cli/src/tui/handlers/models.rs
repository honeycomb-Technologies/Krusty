//! Model fetching handlers
//!
//! Async model fetching from dynamic providers (OpenRouter).

use crate::ai::providers::ProviderId;
use crate::tui::app::App;

impl App {
    /// Start async fetch of OpenRouter models
    pub fn start_openrouter_fetch(&mut self) {
        // Don't start if already fetching
        if self.runtime.channels.openrouter_models.is_some() {
            return;
        }

        // Get OpenRouter API key
        let api_key = match self.services.credential_store.get(&ProviderId::OpenRouter) {
            Some(key) => key.clone(),
            None => {
                tracing::warn!("Cannot fetch OpenRouter models: no API key configured");
                return;
            }
        };

        // Mark popup as loading (only if popup is open)
        self.ui.popups.model.set_loading(true);

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.openrouter_models = Some(rx);

        // Clone registry for async task
        let registry = self.services.model_registry.clone();

        tokio::spawn(async move {
            let result = crate::ai::openrouter::fetch_models(&api_key).await;

            match &result {
                Ok(models) => {
                    // Store in registry
                    registry
                        .set_models(ProviderId::OpenRouter, models.clone())
                        .await;
                    tracing::info!("Fetched {} OpenRouter models", models.len());
                }
                Err(e) => {
                    tracing::error!("Failed to fetch OpenRouter models: {}", e);
                }
            }

            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }

    /// Poll for OpenRouter model fetch completion
    pub fn poll_openrouter_fetch(&mut self) {
        if let Some(rx) = &mut self.runtime.channels.openrouter_models {
            match rx.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(models) => {
                            // Cache models to preferences
                            if let Some(ref prefs) = self.services.preferences {
                                if let Err(e) = prefs.cache_openrouter_models(&models) {
                                    tracing::warn!("Failed to cache OpenRouter models: {}", e);
                                }
                            }
                            // Refresh the popup with new models
                            self.refresh_model_popup();
                            tracing::info!(
                                "OpenRouter models loaded and cached: {} models",
                                models.len()
                            );
                        }
                        Err(e) => {
                            self.ui.popups.model.set_error(e);
                        }
                    }
                    self.runtime.channels.openrouter_models = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.ui
                        .popups
                        .model
                        .set_error("Fetch task closed unexpectedly".to_string());
                    self.runtime.channels.openrouter_models = None;
                }
            }
        }
    }

    /// Refresh model popup with current registry data
    pub fn refresh_model_popup(&mut self) {
        let configured = self.configured_providers();

        // Get organized models from registry (non-blocking)
        if let Some((recent_models, models_by_provider)) = self
            .services
            .model_registry
            .try_get_organized_models(&configured)
        {
            // Convert HashMap to Vec sorted by provider display order
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
