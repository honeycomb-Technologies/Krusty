//! Provider and authentication handlers
//!
//! Provider switching, API key management, and client creation.

use std::sync::Arc;

use anyhow::Result;

use crate::ai::client::AiClient;
use crate::ai::providers::ProviderId;
use crate::tools::register_agent_tool;
use crate::tui::app::App;

impl App {
    fn resolve_auth_for_active_provider(&self) -> Option<String> {
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

        // Try credential store for all providers (API keys and OAuth tokens)
        if let Some(key) = self.resolve_auth_for_active_provider() {
            let config = self.create_client_config();
            self.runtime.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.runtime.api_key = Some(key);
            self.register_agent_tool_if_client().await;
            return Ok(());
        }

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
            self.services.cached_ai_tools = self.services.tool_registry.get_ai_tools().await;
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
        let config = self.create_client_config();
        self.resolve_auth_for_active_provider()
            .map(|key| AiClient::with_api_key(config, key))
    }

    /// Set API key for current provider and create client
    pub fn set_api_key(&mut self, key: String) {
        // Create client with provider config
        let config = self.create_client_config();
        self.runtime.ai_client = Some(AiClient::with_api_key(config, key.clone()));
        self.runtime.api_key = Some(key.clone());

        // Save to credential store (unified storage for all providers)
        self.services
            .credential_store
            .set(self.runtime.active_provider, key);
        if let Err(e) = self.services.credential_store.save() {
            tracing::warn!("Failed to save credential store: {}", e);
        }
    }

    /// Switch to a different provider
    /// Automatically translates the current model to the equivalent in the new provider
    pub fn switch_provider(&mut self, provider_id: ProviderId) {
        use crate::tui::auth::{translate_model_for_provider, validate_model_for_provider};

        let previous_provider = self.runtime.active_provider;
        self.runtime.active_provider = provider_id;

        // Save active provider selection
        if let Err(e) = crate::storage::credentials::ActiveProviderStore::save(provider_id) {
            tracing::warn!("Failed to save active provider: {}", e);
        }

        // Translate model ID to the new provider's format
        let (translated, changed) = translate_model_for_provider(
            &self.runtime.current_model,
            previous_provider,
            provider_id,
        );
        if changed {
            self.runtime.current_model = translated.clone();
            if let Some(ref prefs) = self.services.preferences {
                if let Err(e) = prefs.set_current_model(&translated) {
                    tracing::warn!("Failed to save current model: {}", e);
                }
            }
        }

        // Validate the model exists for this provider (fallback to default if not)
        let (validated, was_fallback) =
            validate_model_for_provider(&self.runtime.current_model, provider_id);
        if was_fallback {
            self.runtime.current_model = validated.clone();
            if let Some(ref prefs) = self.services.preferences {
                if let Err(e) = prefs.set_current_model(&validated) {
                    tracing::warn!("Failed to save current model: {}", e);
                }
            }
        }

        // Try to load credentials for the new provider (API key or OAuth token)
        let auth = if provider_id == ProviderId::Anthropic {
            krusty_core::auth::resolve_anthropic_auth(&self.services.credential_store).credential
        } else if provider_id == ProviderId::OpenAI {
            krusty_core::auth::resolve_openai_auth(
                &self.services.credential_store,
                &self.runtime.current_model,
            )
            .credential
        } else {
            self.services.credential_store.get_auth(&provider_id)
        };

        if let Some(key) = auth {
            let config = self.create_client_config();
            self.runtime.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.runtime.api_key = Some(key);
            tracing::info!(
                "Switched to provider {} (loaded existing auth)",
                provider_id
            );
        } else {
            // No stored credentials - user will need to authenticate
            self.runtime.ai_client = None;
            self.runtime.api_key = None;
            tracing::info!(
                "Switched to provider {} (requires authentication)",
                provider_id
            );
        }
    }

    /// Get list of configured provider IDs (ones with API keys)
    pub fn configured_providers(&self) -> Vec<ProviderId> {
        // Use providers_with_auth to include both API key and OAuth-authenticated providers
        self.services.credential_store.providers_with_auth()
    }

    /// Check if authenticated
    pub fn is_authenticated(&self) -> bool {
        self.runtime.ai_client.is_some()
    }
}
