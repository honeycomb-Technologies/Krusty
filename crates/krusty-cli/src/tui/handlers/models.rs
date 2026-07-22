//! Model fetching handlers
//!
//! Async model fetching from dynamic providers (OpenRouter, OpenAI, Grok, …).
//! Catalog refresh is aligned with the server: any provider with live models
//! and valid catalog credentials can be refreshed, including concurrently.
//!
//! Registry mutations happen on the async fetch tasks — never `block_on` on the
//! TUI poll path — so model refresh cannot hitch the terminal event loop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

use crate::ai::format_detection::detect_api_format;
use crate::ai::models::{resolve_model_metadata, ModelMetadata};
use crate::ai::providers::{get_provider, ProviderId, ReasoningControl};
use crate::tui::app::App;
use crate::tui::app::ThinkingLevel;
use crate::tui::utils::DynamicModelUpdate;
use tokio::sync::Mutex;

struct CatalogRefreshSlot {
    fetch_lock: Mutex<()>,
    commit_lock: Mutex<()>,
    auth_generation: AtomicU64,
}

impl CatalogRefreshSlot {
    fn new() -> Self {
        Self {
            fetch_lock: Mutex::new(()),
            commit_lock: Mutex::new(()),
            auth_generation: AtomicU64::new(0),
        }
    }
}

static CATALOG_REFRESH_SLOTS: LazyLock<HashMap<ProviderId, CatalogRefreshSlot>> =
    LazyLock::new(|| {
        ProviderId::all()
            .iter()
            .copied()
            .map(|provider| (provider, CatalogRefreshSlot::new()))
            .collect()
    });

fn catalog_refresh_slot(provider: ProviderId) -> &'static CatalogRefreshSlot {
    CATALOG_REFRESH_SLOTS
        .get(&provider)
        .expect("every provider has a catalog refresh slot")
}

impl App {
    pub(crate) fn reconcile_model_controls(&mut self, metadata: &ModelMetadata) {
        if metadata.fast_mode.is_none() {
            self.runtime.fast_mode = false;
        }

        let reasoning_is_controllable = metadata.supports_thinking
            && metadata.reasoning_control != Some(ReasoningControl::OutputOnly);
        let mut thinking_levels = metadata
            .supported_reasoning_levels
            .iter()
            .copied()
            .map(ThinkingLevel::from_reasoning_effort)
            .filter(|level| *level != ThinkingLevel::Ultra)
            .collect::<Vec<_>>();
        thinking_levels.dedup();
        let fallback = metadata
            .default_reasoning_level
            .map(ThinkingLevel::from_reasoning_effort)
            .filter(|level| !matches!(level, ThinkingLevel::Off | ThinkingLevel::Ultra))
            .unwrap_or(ThinkingLevel::Medium);
        if reasoning_is_controllable && thinking_levels.is_empty() {
            thinking_levels = if metadata.reasoning_is_mandatory {
                vec![fallback]
            } else {
                vec![ThinkingLevel::Off, fallback]
            };
        } else if metadata.reasoning_is_mandatory {
            thinking_levels.retain(|level| *level != ThinkingLevel::Off);
            if reasoning_is_controllable && thinking_levels.is_empty() {
                thinking_levels.push(fallback);
            }
        } else if reasoning_is_controllable && !thinking_levels.contains(&ThinkingLevel::Off) {
            thinking_levels.insert(0, ThinkingLevel::Off);
        }
        if !reasoning_is_controllable {
            self.runtime.thinking_level = ThinkingLevel::Off;
        } else if !thinking_levels.contains(&self.runtime.thinking_level) {
            self.runtime.thinking_level = metadata
                .default_reasoning_level
                .map(ThinkingLevel::from_reasoning_effort)
                .filter(|level| thinking_levels.contains(level))
                .unwrap_or(thinking_levels[0]);
        }
    }

    pub fn apply_model_selection(&mut self, metadata: ModelMetadata, is_custom: bool) {
        let provider_id = metadata.provider;
        let model_id = metadata.id.clone();
        let model_key = metadata.key();

        if provider_id != self.runtime.active_provider {
            self.switch_provider(provider_id);
            if !self.is_authenticated() {
                let _ = futures::executor::block_on(self.try_load_auth());
            }
        }

        self.runtime.current_model = model_id;
        self.runtime.current_model_key = Some(model_key.clone());
        self.runtime.model_selection_explicit = true;
        self.reconcile_model_controls(&metadata);

        let registry = self.services.model_registry.clone();
        let metadata_for_registry = metadata.clone();
        // User-initiated selection can afford a short sync apply; still avoid
        // holding locks across the whole UI frame by spawning when possible.
        futures::executor::block_on(async {
            registry.upsert_model(metadata_for_registry).await;
            registry.mark_recent_key(&model_key).await;
        });

        // Construct transport only after custom/live metadata is registered so
        // model availability and OpenAI auth provenance can fail closed.
        self.runtime.api_key = self.resolve_auth_for_active_provider();
        self.runtime.ai_client = self.create_ai_client();

        if let Some(ref prefs) = self.services.preferences {
            if let Err(e) = prefs.set_current_model_key(&model_key) {
                tracing::warn!("Failed to save current model: {}", e);
            }
            if let Err(e) = prefs.add_recent_model_key(&model_key) {
                tracing::warn!("Failed to save recent model: {}", e);
            }
            if is_custom {
                if let Err(e) = prefs.save_custom_model(&metadata) {
                    tracing::warn!("Failed to persist custom model metadata: {}", e);
                }
            }
        }
        self.persist_current_session_model_selection();
    }

    pub fn toggle_fast_mode(&mut self) -> Option<bool> {
        let supports_fast_mode = self
            .selected_model_metadata()
            .and_then(|model| model.fast_mode)
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
        self.spawn_dynamic_model_fetch(provider, false);
    }

    fn spawn_dynamic_model_fetch(&mut self, provider: ProviderId, reset_catalog: bool) {
        if !crate::ai::catalog::supports_dynamic_models(provider) {
            return;
        }

        if !self.runtime.dynamic_model_fetches.insert(provider) {
            return;
        }

        let credentials = self.services.credential_store.clone();
        let has_credentials =
            !crate::ai::catalog::credentials_for_dynamic_models(provider, &credentials).is_empty();
        if !has_credentials && !reset_catalog {
            tracing::warn!(
                "Cannot fetch {:?} models: no credential configured",
                provider
            );
            self.runtime.dynamic_model_fetches.remove(&provider);
            return;
        }

        let custom_models = self
            .services
            .preferences
            .as_ref()
            .map(|prefs| prefs.get_custom_models(provider))
            .unwrap_or_default();
        let curated_models = reset_catalog.then(|| curated_provider_models(provider));
        let registry = self.services.model_registry.clone();
        let generation = catalog_refresh_slot(provider)
            .auth_generation
            .load(Ordering::Acquire);
        let tx = self.ensure_dynamic_model_tx();
        self.ui.popups.model.set_loading(true);

        tokio::spawn(async move {
            let slot = catalog_refresh_slot(provider);

            if let Some(curated_models) = curated_models {
                let commit_guard = slot.commit_lock.lock().await;
                if slot.auth_generation.load(Ordering::Acquire) != generation {
                    return;
                }
                registry.set_models(provider, curated_models).await;
                for metadata in custom_models.iter().cloned() {
                    registry.upsert_model(metadata).await;
                }
                drop(commit_guard);
                let _ = tx.send(DynamicModelUpdate::CatalogReset {
                    provider,
                    generation,
                });
            }

            if !has_credentials {
                let _ = tx.send(DynamicModelUpdate::RefreshFinished {
                    provider,
                    generation,
                    result: Err(format!("No catalog credential configured for {provider}")),
                });
                return;
            }

            // Serialize provider fetches, but capture/check credential generation
            // independently so rotation can supersede a slow request immediately.
            let _fetch_guard = slot.fetch_lock.lock().await;
            if slot.auth_generation.load(Ordering::Acquire) != generation {
                return;
            }
            let result =
                crate::ai::catalog::fetch_dynamic_models_for_store(provider, &credentials).await;

            match &result {
                Ok(models) => {
                    let _commit_guard = slot.commit_lock.lock().await;
                    if slot.auth_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
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

            let _ = tx.send(DynamicModelUpdate::RefreshFinished {
                provider,
                generation,
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

        let mut catalog_changed = false;
        let mut any_success = false;
        let mut last_error: Option<String> = None;

        for update in received {
            match update {
                DynamicModelUpdate::CatalogReset {
                    provider,
                    generation,
                } => {
                    if catalog_refresh_slot(provider)
                        .auth_generation
                        .load(Ordering::Acquire)
                        != generation
                    {
                        continue;
                    }
                    self.rebind_active_client_after_catalog_change(provider);
                    catalog_changed = true;
                }
                DynamicModelUpdate::RefreshFinished {
                    provider,
                    generation,
                    result,
                } => {
                    if catalog_refresh_slot(provider)
                        .auth_generation
                        .load(Ordering::Acquire)
                        != generation
                    {
                        continue;
                    }
                    self.runtime.dynamic_model_fetches.remove(&provider);
                    match result {
                        Ok(models) => {
                            // The background task installed the registry snapshot
                            // under the same generation/commit guard.
                            self.cache_dynamic_models(provider, &models);
                            self.rebind_active_client_after_catalog_change(provider);
                            catalog_changed = true;
                            any_success = true;
                        }
                        Err(error) => {
                            last_error = Some(format!("{provider:?}: {error}"));
                        }
                    }
                }
            }
        }

        if catalog_changed {
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

    /// Invalidate entitlement-scoped catalog state and fetch with the new credential.
    ///
    /// A provider generation and commit lock prevent an older in-flight account
    /// fetch from being applied after credential rotation. Other providers keep
    /// refreshing independently.
    pub fn refresh_dynamic_models_after_credential_change(&mut self, provider: ProviderId) {
        if !crate::ai::catalog::supports_dynamic_models(provider) {
            return;
        }

        // Supersede first so an old fetch cannot commit while local account
        // state is being reset.
        catalog_refresh_slot(provider)
            .auth_generation
            .fetch_add(1, Ordering::AcqRel);

        if self.runtime.active_provider == provider {
            // Do not allow the old account's selected row to execute during
            // the asynchronous reset/refetch window.
            self.runtime.api_key = None;
            self.runtime.ai_client = None;
        }

        if let Some(ref prefs) = self.services.preferences {
            if let Err(error) = prefs.clear_model_cache(provider) {
                tracing::warn!("Failed to invalidate {:?} model cache: {}", provider, error);
            }
        }

        // Other providers remain active while this provider is reset/refetched.
        self.runtime.dynamic_model_fetches.remove(&provider);
        self.spawn_dynamic_model_fetch(provider, true);
    }

    fn rebind_active_client_after_catalog_change(&mut self, provider: ProviderId) {
        if self.runtime.active_provider != provider || !self.has_selected_model() {
            return;
        }

        // Account-only models disappear during reset. Do not keep a client
        // alive for stale catalog state while the replacement fetch is pending.
        let Some(metadata) = self.selected_model_metadata() else {
            self.runtime.api_key = None;
            self.runtime.ai_client = None;
            return;
        };
        self.reconcile_model_controls(&metadata);

        self.runtime.api_key = self.resolve_auth_for_active_provider();
        self.runtime.ai_client = self.create_ai_client();
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

fn curated_provider_models(provider: ProviderId) -> Vec<ModelMetadata> {
    let Some(config) = get_provider(provider) else {
        return Vec::new();
    };

    config
        .models
        .iter()
        .map(|model_info| {
            let api_format = detect_api_format(provider, &model_info.id);
            let inferred = resolve_model_metadata(provider, &model_info.id, api_format);
            let mut model = ModelMetadata::new(&model_info.id, &model_info.display_name, provider)
                .with_context(model_info.context_window, model_info.max_output);
            if let Some(reasoning) = model_info.reasoning {
                model = model.with_thinking(reasoning);
            }
            model.supported_reasoning_levels = model_info.supported_reasoning_levels.clone();
            model.default_reasoning_level = model_info.default_reasoning_level;
            model.reasoning_is_mandatory = model_info.reasoning_is_mandatory;
            model.reasoning_control = model_info.reasoning_control;
            model.fast_mode = model_info.fast_mode;
            model.supports_tools = config.supports_tools;
            model.supports_vision = inferred.supports_vision;
            model.api_format = inferred.api_format;
            model
        })
        .collect()
}
