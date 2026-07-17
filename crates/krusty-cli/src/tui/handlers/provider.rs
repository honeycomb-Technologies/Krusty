//! Provider and authentication handlers
//!
//! Provider switching, API key management, and client creation.

use std::sync::Arc;

use anyhow::Result;

use crate::ai::client::AiClient;
use crate::ai::providers::{get_provider, ProviderId};
use crate::tools::register_agent_tool;
use crate::tui::app::App;
use crate::tui::auth::{infer_provider_for_model, translate_model_for_provider};

impl App {
    pub(crate) fn has_selected_model(&self) -> bool {
        !self.runtime.current_model.trim().is_empty()
    }

    pub(crate) fn persist_current_model_selection(&self) {
        let Some(ref prefs) = self.services.preferences else {
            return;
        };

        let result = if let Some(model) = (!self.runtime.current_model.trim().is_empty())
            .then_some(self.runtime.current_model.trim())
        {
            prefs.set_current_model(model)
        } else {
            prefs.delete("current_model")
        };

        if let Err(error) = result {
            tracing::warn!("Failed to persist current model selection: {}", error);
        }
    }

    pub(crate) fn sync_active_provider_to_current_model(&mut self) {
        let Some(provider) =
            infer_provider_for_model(&self.services.model_registry, &self.runtime.current_model)
        else {
            return;
        };

        self.runtime.active_provider = provider;
        if let Err(error) = crate::storage::credentials::ActiveProviderStore::save(provider) {
            tracing::warn!("Failed to save active provider: {}", error);
        }
    }

    fn model_available_for_provider(&self, model: &str, provider: ProviderId) -> bool {
        let model = model.trim();
        if model.is_empty() {
            return false;
        }

        if let Some(metadata) = self.services.model_registry.try_get_model(model) {
            return metadata.provider == provider;
        }

        get_provider(provider)
            .map(|config| config.has_model(model))
            .unwrap_or(false)
    }

    pub(crate) fn resolve_auth_for_active_provider(&self) -> Option<String> {
        if self.runtime.active_provider == ProviderId::Anthropic {
            let resolved =
                krusty_core::auth::resolve_anthropic_auth(&self.services.credential_store);
            return resolved.credential;
        }

        if self.runtime.active_provider == ProviderId::OpenAI {
            let resolved = krusty_core::auth::resolve_openai_auth(
                &self.services.credential_store,
                &self.runtime.current_model,
            );
            return resolved.credential;
        }

        if self.runtime.active_provider == ProviderId::Grok {
            let resolved = krusty_core::auth::resolve_grok_auth(&self.services.credential_store);
            return resolved.credential;
        }

        self.services
            .credential_store
            .get_auth(&self.runtime.active_provider)
    }

    /// Try to load existing authentication for the active provider
    pub async fn try_load_auth(&mut self) -> Result<()> {
        // Refresh expired OAuth tokens before checking auth
        if self.runtime.active_provider.supports_oauth() {
            if let Err(e) =
                krusty_core::auth::refresh_oauth_token(self.runtime.active_provider).await
            {
                tracing::debug!("OAuth token refresh skipped: {}", e);
            }
        }

        let auth = self.resolve_auth_for_active_provider();
        self.runtime.api_key = auth.clone();

        if !self.has_selected_model() {
            self.runtime.ai_client = None;
            return Ok(());
        }

        if let Some(key) = auth {
            let config = self.create_client_config();
            self.runtime.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.runtime.api_key = Some(key);
            self.register_agent_tool_if_client().await;
            return Ok(());
        }

        self.runtime.ai_client = None;
        Ok(())
    }

    /// Register the unified agent tool if client is available
    pub(crate) async fn register_agent_tool_if_client(&mut self) {
        let client = self.create_ai_client();

        if let Some(client) = client {
            let client = Arc::new(client);

            // Register unified agent tool (explore, plan, verify, build)
            register_agent_tool(
                &self.services.tool_registry,
                client,
                self.runtime.cancellation.clone(),
            )
            .await;

            // Update cached tools so API knows about the agent tool
            self.services.cached_ai_tools = self.services.tool_registry.get_ai_tools_all().await;
            tracing::info!(
                "Registered agent tool, total tools: {}",
                self.services.cached_ai_tools.len()
            );
        }
    }

    /// Create AiClientConfig for the current active provider
    pub fn create_client_config(&self) -> crate::ai::client::AiClientConfig {
        crate::tui::auth::create_client_config(
            self.runtime.active_provider,
            &self.runtime.current_model,
            &self.services.credential_store,
            &self.services.model_registry,
        )
    }

    /// Create an AI client with the current provider configuration
    pub fn create_ai_client(&self) -> Option<AiClient> {
        if !self.has_selected_model() {
            return None;
        }

        let config = self.create_client_config();
        self.resolve_auth_for_active_provider()
            .map(|key| AiClient::with_api_key(config, key))
    }

    /// Set API key for current provider and create client
    pub fn set_api_key(&mut self, key: String) {
        // Save to credential store first, then re-resolve auth for the selected
        // model so OpenAI endpoint/config detection and credential choice stay
        // in sync when both API-key and OAuth credentials exist.
        self.services
            .credential_store
            .set(self.runtime.active_provider, key);
        if let Err(e) = self.services.credential_store.save() {
            tracing::warn!("Failed to save credential store: {}", e);
        }

        let auth = self.resolve_auth_for_active_provider();
        self.runtime.api_key = auth.clone();
        self.runtime.ai_client = if self.has_selected_model() {
            auth.map(|key| {
                let config = self.create_client_config();
                AiClient::with_api_key(config, key)
            })
        } else {
            None
        };
    }

    /// Switch to a different provider.
    ///
    /// If the current model has a provider-specific equivalent, keep it.
    /// Otherwise clear the selected model and require explicit selection.
    pub fn switch_provider(&mut self, provider_id: ProviderId) {
        let previous_provider = self.runtime.active_provider;
        self.runtime.active_provider = provider_id;

        if let Err(e) = crate::storage::credentials::ActiveProviderStore::save(provider_id) {
            tracing::warn!("Failed to save active provider: {}", e);
        }

        let next_model = translate_model_for_provider(
            &self.runtime.current_model,
            previous_provider,
            provider_id,
        )
        .filter(|model| self.model_available_for_provider(model, provider_id))
        .unwrap_or_default();

        let cleared_model = self.has_selected_model() && next_model.is_empty();
        self.runtime.current_model = next_model;
        self.persist_current_model_selection();

        let auth = self.resolve_auth_for_active_provider();
        self.runtime.api_key = auth.clone();
        self.runtime.ai_client = if self.has_selected_model() {
            auth.clone().map(|key| {
                let config = self.create_client_config();
                AiClient::with_api_key(config, key)
            })
        } else {
            None
        };

        if self.runtime.ai_client.is_some() {
            tracing::info!(
                "Switched to provider {} (loaded existing auth)",
                provider_id
            );
        } else if auth.is_some() {
            tracing::info!("Switched to provider {} (no model selected)", provider_id);
        } else {
            tracing::info!(
                "Switched to provider {} (requires authentication)",
                provider_id
            );
        }

        if cleared_model {
            tracing::info!(
                "Cleared current model while switching to {} because no equivalent model is available",
                provider_id
            );
        }
    }

    /// Get list of configured provider IDs (ones with API keys)
    pub fn configured_providers(&self) -> Vec<ProviderId> {
        self.services.credential_store.providers_with_auth()
    }

    /// Check if authenticated
    pub fn is_authenticated(&self) -> bool {
        self.runtime.ai_client.is_some()
    }
}
