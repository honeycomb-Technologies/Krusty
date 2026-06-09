use crate::ai::providers::{get_provider, ProviderId};
use crate::auth::{resolve_anthropic_auth, resolve_grok_auth, resolve_openai_auth};
use crate::storage::credentials::CredentialStore;

use super::{
    persist_shared_current_model, AcpError, AvailableModelRecord, KrustyAgent, ModelConfig,
};

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

            let Some(catalog_credential) =
                crate::ai::catalog::credential_for_dynamic_models(provider, &store)
            else {
                tracing::debug!(
                    "Skipping dynamic {:?} model fetch: no catalog API key configured",
                    provider
                );
                push_static_provider_models(&mut models, &store, provider);
                continue;
            };

            match crate::ai::catalog::fetch_dynamic_models(provider, &catalog_credential).await {
                Ok(fetched) => {
                    let fetched_count = fetched.len();
                    for model in fetched {
                        let model_id = format!("{}:{}", provider.storage_key(), model.id);
                        models.push((
                            model_id,
                            provider,
                            model.id.clone(),
                            catalog_credential.clone(),
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
        let available = self.available_models.read().await;
        let model_config = available
            .iter()
            .find(|(id, _, _, _, _)| id == model_id)
            .ok_or_else(|| AcpError::ProtocolError(format!("Model not found: {}", model_id)))?;

        let provider = model_config.1;
        let actual_model_id = model_config.2.clone();
        let listed_credential = model_config.3.clone();
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

    /// Get the current model ID.
    pub async fn current_model_id(&self) -> Option<String> {
        self.current_model
            .read()
            .await
            .as_ref()
            .map(|m| format!("{}:{}", m.provider.storage_key(), m.model_id))
    }
}
