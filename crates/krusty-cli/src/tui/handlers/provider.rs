//! Provider and authentication handlers
//!
//! Provider switching, API key management, and client creation.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::agent::dual_mind::{DualMind, DualMindConfig};
use crate::ai::client::AiClient;
use crate::ai::providers::ProviderId;
use crate::tools::{register_build_tool, register_explore_tool};
use crate::tui::app::App;

impl App {
    /// Try to load existing authentication for the active provider
    pub async fn try_load_auth(&mut self) -> Result<()> {
        // Try credential store for all providers (API keys and OAuth tokens)
        if let Some(key) = self
            .services
            .credential_store
            .get_auth(&self.active_provider)
        {
            let config = self.create_client_config();
            self.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.api_key = Some(key);
            self.init_dual_mind();
            self.register_explore_tool_if_client().await;
            return Ok(());
        }

        Ok(())
    }

    /// Register explore and build tools if client is available
    pub(crate) async fn register_explore_tool_if_client(&mut self) {
        let client = self.create_ai_client();

        if let Some(client) = client {
            let client = Arc::new(client);

            // Register explore tool
            register_explore_tool(
                &self.services.tool_registry,
                client.clone(),
                self.cancellation.clone(),
            )
            .await;

            // Register build tool (The Kraken)
            register_build_tool(
                &self.services.tool_registry,
                client,
                self.cancellation.clone(),
            )
            .await;

            // Update cached tools so API knows about explore and build
            self.services.cached_ai_tools = self.services.tool_registry.get_ai_tools().await;
            tracing::info!(
                "Registered explore and build tools, total tools: {}",
                self.services.cached_ai_tools.len()
            );
        }
    }

    /// Create AiClientConfig for the current active provider
    pub fn create_client_config(&self) -> crate::ai::client::AiClientConfig {
        crate::tui::auth::create_client_config(
            self.active_provider,
            &self.current_model,
            &self.services.credential_store,
            &self.services.model_registry,
        )
    }

    /// Create an AI client with the current provider configuration
    pub fn create_ai_client(&self) -> Option<AiClient> {
        let config = self.create_client_config();
        self.api_key
            .as_ref()
            .map(|key| AiClient::with_api_key(config, key.clone()))
    }

    /// Set API key for current provider and create client
    pub fn set_api_key(&mut self, key: String) {
        // Create client with provider config
        let config = self.create_client_config();
        self.ai_client = Some(AiClient::with_api_key(config, key.clone()));
        self.api_key = Some(key.clone());
        self.init_dual_mind();

        // Save to credential store (unified storage for all providers)
        self.services
            .credential_store
            .set(self.active_provider, key);
        if let Err(e) = self.services.credential_store.save() {
            tracing::warn!("Failed to save credential store: {}", e);
        }
    }

    /// Switch to a different provider
    /// Automatically translates the current model to the equivalent in the new provider
    pub fn switch_provider(&mut self, provider_id: ProviderId) {
        use crate::tui::auth::{translate_model_for_provider, validate_model_for_provider};

        let previous_provider = self.active_provider;
        self.active_provider = provider_id;

        // Save active provider selection
        if let Err(e) = crate::storage::credentials::ActiveProviderStore::save(provider_id) {
            tracing::warn!("Failed to save active provider: {}", e);
        }

        // Translate model ID to the new provider's format
        let (translated, changed) =
            translate_model_for_provider(&self.current_model, previous_provider, provider_id);
        if changed {
            self.current_model = translated.clone();
            if let Some(ref prefs) = self.services.preferences {
                if let Err(e) = prefs.set_current_model(&translated) {
                    tracing::warn!("Failed to save current model: {}", e);
                }
            }
        }

        // Validate the model exists for this provider (fallback to default if not)
        let (validated, was_fallback) =
            validate_model_for_provider(&self.current_model, provider_id);
        if was_fallback {
            self.current_model = validated.clone();
            if let Some(ref prefs) = self.services.preferences {
                if let Err(e) = prefs.set_current_model(&validated) {
                    tracing::warn!("Failed to save current model: {}", e);
                }
            }
        }

        // Try to load credentials for the new provider (API key or OAuth token)
        if let Some(key) = self.services.credential_store.get_auth(&provider_id) {
            let config = self.create_client_config();
            self.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.api_key = Some(key);
            self.init_dual_mind();
            tracing::info!(
                "Switched to provider {} (loaded existing auth)",
                provider_id
            );
        } else {
            // No stored credentials - user will need to authenticate
            self.ai_client = None;
            self.api_key = None;
            self.dual_mind = None;
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
        self.ai_client.is_some()
    }

    /// Initialize dual-mind system when AI client is available
    fn init_dual_mind(&mut self) {
        let Some(client) = self.create_ai_client() else {
            self.dual_mind = None;
            return;
        };

        let client = Arc::new(client);
        let config = DualMindConfig {
            enabled: true,
            review_all: false, // Only review significant actions
            max_discussion_depth: 3,
        };

        let dual_mind = DualMind::with_tools(
            client,
            self.cancellation.clone(),
            config,
            self.services.tool_registry.clone(),
            self.working_dir.clone(),
        );

        self.dual_mind = Some(Arc::new(RwLock::new(dual_mind)));
        tracing::info!("Dual-mind system initialized (Big Claw / Little Claw)");
    }
}
