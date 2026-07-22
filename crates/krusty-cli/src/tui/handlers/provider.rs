//! Provider and authentication handlers
//!
//! Provider switching, API key management, and client creation.

use std::sync::Arc;

use anyhow::Result;

use crate::ai::client::AiClient;
use crate::ai::models::{ModelKey, ModelMetadata, ProjectModelRef};
use crate::ai::providers::ProviderId;
use crate::storage::ProjectSettings;
use crate::tools::register_agent_tool;
use crate::tui::app::App;
use crate::tui::auth::{resolve_openai_auth_for_metadata, translate_model_for_provider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimaryRunModelSource {
    Explicit,
    Session,
    Project,
    Preference,
}

fn choose_primary_run_model(
    explicit: Option<ModelKey>,
    session: Option<ProjectModelRef>,
    project: Option<ProjectModelRef>,
    preference: Option<ProjectModelRef>,
) -> Option<(PrimaryRunModelSource, ProjectModelRef)> {
    explicit
        .map(ProjectModelRef::Exact)
        .map(|model_ref| (PrimaryRunModelSource::Explicit, model_ref))
        .or_else(|| session.map(|model_ref| (PrimaryRunModelSource::Session, model_ref)))
        .or_else(|| project.map(|model_ref| (PrimaryRunModelSource::Project, model_ref)))
        .or_else(|| preference.map(|model_ref| (PrimaryRunModelSource::Preference, model_ref)))
}

impl App {
    /// Resolve and activate the exact model for the next primary TUI run.
    ///
    /// This is intentionally called before client construction. Invalid
    /// explicit/session/project identities fail closed rather than falling
    /// through to a lower-precedence preference.
    pub(crate) fn prepare_primary_run_model(
        &mut self,
        project_settings: &ProjectSettings,
    ) -> Result<ModelMetadata> {
        let explicit = self
            .runtime
            .model_selection_explicit
            .then(|| self.runtime.current_model_key.clone())
            .flatten();
        if self.runtime.model_selection_explicit && explicit.is_none() {
            anyhow::bail!("The explicit model selection has no exact model key");
        }

        let session = if let Some(session_id) = self.runtime.current_session_id.as_deref() {
            let session_manager = self
                .services
                .session_manager
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No session manager is available"))?;
            let session = session_manager
                .get_session(session_id)?
                .ok_or_else(|| anyhow::anyhow!("Session '{session_id}' no longer exists"))?;
            session.model_key.map(ProjectModelRef::Exact).or_else(|| {
                session
                    .model
                    .map(|model| model.trim().to_string())
                    .filter(|model| !model.is_empty())
                    .map(ProjectModelRef::Legacy)
            })
        } else {
            None
        };
        let session_was_empty = session.is_none();
        let preference = self
            .runtime
            .current_model_key
            .clone()
            .map(ProjectModelRef::Exact)
            .or_else(|| {
                let model = self.runtime.current_model.trim();
                (!model.is_empty()).then(|| ProjectModelRef::Legacy(model.to_string()))
            });
        let (source, model_ref) = choose_primary_run_model(
            explicit,
            session,
            project_settings.model.clone(),
            preference,
        )
        .ok_or_else(|| anyhow::anyhow!("No model selected. Use /model to choose one."))?;

        let metadata = futures::executor::block_on(
            self.services
                .model_registry
                .resolve_project_model_ref(&model_ref),
        )
        .map_err(|error| anyhow::anyhow!("Cannot resolve model for this run: {error}"))?;

        self.runtime.current_model = metadata.id.clone();
        self.runtime.current_model_key = Some(metadata.key());
        self.runtime.active_provider = metadata.provider;
        self.reconcile_model_controls(&metadata);
        self.runtime.api_key = self.resolve_auth_for_active_provider();
        self.runtime.ai_client = self.create_ai_client();

        if source == PrimaryRunModelSource::Project && session_was_empty {
            if let (Some(session_manager), Some(session_id)) = (
                self.services.session_manager.as_ref(),
                self.runtime.current_session_id.as_deref(),
            ) {
                session_manager.update_session_model_selection(
                    session_id,
                    Some(&metadata.key()),
                    metadata.catalog_revision.as_deref(),
                )?;
            }
        }

        Ok(metadata)
    }

    pub(crate) fn has_selected_model(&self) -> bool {
        self.runtime.current_model_key.as_ref().is_some_and(|key| {
            !self.runtime.current_model.trim().is_empty()
                && key.model_id == self.runtime.current_model
                && key.provider == self.runtime.active_provider
        })
    }

    /// Resolve the exact selected catalog row without falling back to a bare
    /// model slug. All runtime capability and transport decisions use this.
    pub(crate) fn selected_model_metadata(&self) -> Option<ModelMetadata> {
        let key = self.runtime.current_model_key.as_ref()?;
        if key.model_id != self.runtime.current_model
            || key.provider != self.runtime.active_provider
        {
            return None;
        }
        self.services.model_registry.try_get_model_by_key(key)
    }

    /// Find a unique row for a legacy/provider translation. Multiple auth or
    /// transport rows with the same slug are intentionally not guessed.
    fn unique_model_key_for_provider(&self, provider: ProviderId, model: &str) -> Option<ModelKey> {
        let (_, models_by_provider) = futures::executor::block_on(
            self.services
                .model_registry
                .get_organized_models(ProviderId::all()),
        );
        let mut matches = models_by_provider
            .get(&provider)?
            .iter()
            .filter(|metadata| metadata.id == model)
            .map(ModelMetadata::key);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    pub(crate) fn persist_current_model_selection(&self) {
        let Some(ref prefs) = self.services.preferences else {
            return;
        };

        let result = if let Some(key) = self.runtime.current_model_key.as_ref().filter(|key| {
            key.model_id == self.runtime.current_model
                && key.provider == self.runtime.active_provider
        }) {
            prefs.set_current_model_key(key)
        } else if !self.runtime.current_model.trim().is_empty() {
            // Legacy/bare assignments must invalidate any stale exact key.
            prefs.set_current_model(self.runtime.current_model.trim())
        } else {
            prefs.clear_current_model()
        };

        if let Err(error) = result {
            tracing::warn!("Failed to persist current model selection: {}", error);
        }
    }

    pub(crate) fn persist_current_session_model_selection(&self) {
        let (Some(session_manager), Some(session_id)) = (
            self.services.session_manager.as_ref(),
            self.runtime.current_session_id.as_deref(),
        ) else {
            return;
        };

        let result = if let Some(key) = self.runtime.current_model_key.as_ref().filter(|key| {
            key.model_id == self.runtime.current_model
                && key.provider == self.runtime.active_provider
        }) {
            let catalog_revision = self
                .selected_model_metadata()
                .and_then(|metadata| metadata.catalog_revision);
            session_manager.update_session_model_selection(
                session_id,
                Some(key),
                catalog_revision.as_deref(),
            )
        } else {
            let model = (!self.runtime.current_model.trim().is_empty())
                .then_some(self.runtime.current_model.trim());
            session_manager.update_session_model(session_id, model)
        };

        if let Err(error) = result {
            tracing::warn!(%error, %session_id, "Failed to persist session model selection");
        }
    }

    pub(crate) fn resolve_auth_for_active_provider(&self) -> Option<String> {
        if self.runtime.active_provider == ProviderId::Anthropic {
            let resolved =
                krusty_core::auth::resolve_anthropic_auth(&self.services.credential_store);
            return resolved.credential;
        }

        if self.runtime.active_provider == ProviderId::OpenAI {
            let metadata = self.selected_model_metadata()?;
            let resolved =
                resolve_openai_auth_for_metadata(&metadata, &self.services.credential_store);
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

        if let Some(client) = self.create_ai_client() {
            self.runtime.ai_client = Some(client);
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
    pub fn create_client_config(&self) -> Option<crate::ai::client::AiClientConfig> {
        let metadata = self.selected_model_metadata()?;
        Some(crate::tui::auth::create_client_config(
            &metadata,
            &self.services.credential_store,
        ))
    }

    /// Create an AI client with the current provider configuration
    pub fn create_ai_client(&self) -> Option<AiClient> {
        if !self.has_selected_model() {
            return None;
        }
        let metadata = self.selected_model_metadata()?;

        if self.runtime.active_provider == ProviderId::OpenAI {
            // Resolve provenance once and use the same result for both the
            // credential and endpoint. A scoped catalog row must never fall
            // back to the other OpenAI transport.
            let resolution =
                resolve_openai_auth_for_metadata(&metadata, &self.services.credential_store);
            let credential = resolution.credential.clone()?;
            let mut config = crate::ai::client::AiClientConfig::for_openai_with_auth_resolution(
                &metadata.id,
                resolution,
            );
            config.api_format = metadata.api_format;
            config.max_tokens = metadata.max_output;
            return match AiClient::new_with_resolved_model(
                config,
                credential,
                metadata.resolve_runtime(),
            ) {
                Ok(client) => Some(client),
                Err(error) => {
                    tracing::warn!(%error, "Refused inconsistent OpenAI model runtime");
                    None
                }
            };
        }

        let config = self.create_client_config()?;
        let credential = self.resolve_auth_for_active_provider()?;
        match AiClient::new_with_resolved_model(config, credential, metadata.resolve_runtime()) {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::warn!(%error, "Refused inconsistent model runtime");
                None
            }
        }
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

        // Catalogs are account-scoped. Discard the previous account snapshot
        // and force a new fetch before resolving transport for this key. The
        // refresh path keeps the client disabled until curated/live metadata
        // for the new credential generation is installed.
        if crate::ai::catalog::supports_dynamic_models(self.runtime.active_provider) {
            self.refresh_dynamic_models_after_credential_change(self.runtime.active_provider);
            return;
        }

        let auth = self.resolve_auth_for_active_provider();
        self.runtime.api_key = auth;
        self.runtime.ai_client = if self.has_selected_model() {
            self.create_ai_client()
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
        let had_selected_model = self.has_selected_model();
        self.runtime.active_provider = provider_id;

        if let Err(e) = crate::storage::credentials::ActiveProviderStore::save(provider_id) {
            tracing::warn!("Failed to save active provider: {}", e);
        }

        let next_key = if previous_provider == provider_id {
            self.runtime.current_model_key.clone().filter(|key| {
                key.provider == provider_id && key.model_id == self.runtime.current_model
            })
        } else {
            translate_model_for_provider(
                &self.runtime.current_model,
                previous_provider,
                provider_id,
            )
            .and_then(|model| self.unique_model_key_for_provider(provider_id, &model))
        };
        let next_model = next_key
            .as_ref()
            .map(|key| key.model_id.clone())
            .unwrap_or_default();

        let cleared_model = had_selected_model && next_model.is_empty();
        self.runtime.current_model = next_model;
        self.runtime.current_model_key = next_key;
        if let Some(metadata) = self.selected_model_metadata() {
            self.reconcile_model_controls(&metadata);
        } else {
            self.runtime.fast_mode = false;
            self.runtime.thinking_level = crate::tui::app::ThinkingLevel::Off;
        }
        self.persist_current_model_selection();
        self.persist_current_session_model_selection();

        let auth = self.resolve_auth_for_active_provider();
        self.runtime.api_key = auth.clone();
        self.runtime.ai_client = if self.has_selected_model() {
            self.create_ai_client()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::ApiFormat;

    fn key(id: &str) -> ModelKey {
        ModelKey::new(ProviderId::OpenRouter, id, ApiFormat::OpenAI)
    }

    #[test]
    fn primary_run_model_precedence_is_explicit_session_project_preference() {
        let explicit = key("explicit");
        let session = ProjectModelRef::Exact(key("session"));
        let project = ProjectModelRef::Exact(key("project"));
        let preference = ProjectModelRef::Exact(key("preference"));

        let selected = choose_primary_run_model(
            Some(explicit.clone()),
            Some(session.clone()),
            Some(project.clone()),
            Some(preference.clone()),
        );
        assert_eq!(
            selected,
            Some((
                PrimaryRunModelSource::Explicit,
                ProjectModelRef::Exact(explicit)
            ))
        );
        assert_eq!(
            choose_primary_run_model(
                None,
                Some(session.clone()),
                Some(project.clone()),
                Some(preference.clone())
            ),
            Some((PrimaryRunModelSource::Session, session))
        );
        assert_eq!(
            choose_primary_run_model(None, None, Some(project.clone()), Some(preference.clone())),
            Some((PrimaryRunModelSource::Project, project))
        );
        assert_eq!(
            choose_primary_run_model(None, None, None, Some(preference.clone())),
            Some((PrimaryRunModelSource::Preference, preference))
        );
    }
}
