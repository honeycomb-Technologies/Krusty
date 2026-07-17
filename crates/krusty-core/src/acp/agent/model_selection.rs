use crate::ai::providers::{get_provider, ProviderId};
use crate::auth::{resolve_anthropic_auth, resolve_grok_auth, resolve_openai_auth};
use crate::storage::credentials::CredentialStore;

use super::{
    persist_shared_current_model, AcpError, AvailableModelRecord, KrustyAgent, ModelConfig,
};
use crate::acp::session::{SessionModelSelection, SessionState};

fn credential_for_model(
    store: &CredentialStore,
    provider: ProviderId,
    model_id: &str,
) -> Option<String> {
    match provider {
        ProviderId::OpenAI => resolve_openai_auth(store, model_id).credential,
        ProviderId::Anthropic => resolve_anthropic_auth(store).credential,
        ProviderId::Grok => resolve_grok_auth(store).credential,
        _ => store.get_auth(&provider),
    }
}

fn push_static_provider_models(
    models: &mut Vec<AvailableModelRecord>,
    store: &CredentialStore,
    provider: ProviderId,
) {
    let Some(provider_config) = get_provider(provider) else {
        return;
    };

    for model_info in &provider_config.models {
        let Some(credential) = credential_for_model(store, provider, &model_info.id) else {
            continue;
        };
        let model_id = format!("{}:{}", provider.storage_key(), model_info.id);
        models.push((
            model_id,
            provider,
            model_info.id.clone(),
            credential,
            model_info.display_name.clone(),
        ));
        tracing::debug!(
            "Added model: {} from {:?}",
            model_info.display_name,
            provider
        );
    }
}

impl KrustyAgent {
    /// Detect all available models from configured providers.
    pub async fn detect_available_models(&self) -> Vec<AvailableModelRecord> {
        let mut models = Vec::new();

        let store = match CredentialStore::load() {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!("Failed to load credential store: {}", e);
                return models;
            }
        };

        let configured: std::collections::HashSet<_> =
            store.providers_with_auth().into_iter().collect();
        tracing::info!("Found {} configured providers", configured.len());

        for &provider in ProviderId::all() {
            if !configured.contains(&provider) {
                continue;
            }

            if !crate::ai::catalog::supports_dynamic_models(provider) {
                push_static_provider_models(&mut models, &store, provider);
                continue;
            }

            if crate::ai::catalog::credentials_for_dynamic_models(provider, &store).is_empty() {
                tracing::debug!(
                    "Skipping dynamic {:?} model fetch: no catalog credential configured",
                    provider
                );
                push_static_provider_models(&mut models, &store, provider);
                continue;
            }

            match crate::ai::catalog::fetch_dynamic_models_for_store(provider, &store).await {
                Ok(fetched) => {
                    let fetched_count = fetched.len();
                    for model in fetched {
                        let Some(model_credential) =
                            credential_for_model(&store, provider, &model.id)
                        else {
                            continue;
                        };
                        let model_id = format!("{}:{}", provider.storage_key(), model.id);
                        models.push((
                            model_id,
                            provider,
                            model.id.clone(),
                            model_credential,
                            model.display_name.clone(),
                        ));
                    }
                    tracing::info!("Added {} models from {:?}", fetched_count, provider);
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch {:?} models: {}", provider, e);
                    push_static_provider_models(&mut models, &store, provider);
                }
            }
        }

        tracing::info!("Total models available: {}", models.len());
        models
    }

    /// Set the current model and reinitialize the processor.
    pub async fn set_model(&self, model_id: &str) -> Result<(), AcpError> {
        let model_config = self.resolve_model_record(model_id).await?;
        let provider = model_config.1;
        let actual_model_id = model_config.2.clone();
        let listed_credential = model_config.3;
        let api_key = CredentialStore::load()
            .ok()
            .and_then(|store| credential_for_model(&store, provider, &actual_model_id))
            .unwrap_or(listed_credential);

        tracing::info!(
            "Switching to model: {} (provider: {:?})",
            actual_model_id,
            provider
        );

        *self.current_model.write().await = Some(ModelConfig {
            provider,
            model_id: actual_model_id.clone(),
        });
        persist_shared_current_model(provider, &actual_model_id);

        self.processor
            .write()
            .await
            .init_ai_client(api_key, provider, Some(actual_model_id));

        Ok(())
    }

    /// Select a model for one session without mutating any other ACP session or
    /// the connection-wide default model.
    pub(super) async fn set_model_for_session(
        &self,
        session: &SessionState,
        model_id: &str,
        persist: bool,
    ) -> Result<(), AcpError> {
        let model_config = self.resolve_model_record(model_id).await?;
        let provider = model_config.1;
        let actual_model_id = model_config.2.clone();
        let listed_credential = model_config.3;
        let api_key = CredentialStore::load()
            .ok()
            .and_then(|store| credential_for_model(&store, provider, &actual_model_id))
            .unwrap_or(listed_credential);

        let client = self
            .processor
            .read()
            .await
            .build_ai_client(api_key, provider, Some(actual_model_id.clone()))
            .ok_or_else(|| {
                AcpError::NotAuthenticated(format!(
                    "Unable to initialize model {}",
                    actual_model_id
                ))
            })?;
        session
            .set_model_client(
                SessionModelSelection {
                    provider,
                    model_id: actual_model_id,
                    acp_model_id: model_config.0.clone(),
                },
                client,
            )
            .await;
        if persist {
            session.persist_model(&model_config.0).await;
        }
        Ok(())
    }

    pub(super) async fn resolve_persisted_model_id(&self, persisted: &str) -> Option<String> {
        self.available_models
            .read()
            .await
            .iter()
            .find(|(id, _, actual, _, _)| id == persisted || actual == persisted)
            .map(|(id, _, _, _, _)| id.clone())
    }

    async fn resolve_model_record(&self, model_id: &str) -> Result<AvailableModelRecord, AcpError> {
        self.available_models
            .read()
            .await
            .iter()
            .find(|(id, _, _, _, _)| id == model_id)
            .cloned()
            .ok_or_else(|| AcpError::ProtocolError(format!("Model not found: {}", model_id)))
    }

    /// Get the current model ID.
    pub async fn current_model_id(&self) -> Option<String> {
        self.current_model
            .read()
            .await
            .as_ref()
            .map(|m| format!("{}:{}", m.provider.storage_key(), m.model_id))
    }
}
