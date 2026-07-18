use crate::ai::catalog::{CatalogAuthKind, CatalogCredential};
use crate::ai::models::ModelAuthScope;
use crate::ai::providers::{get_provider, ProviderId};
use crate::auth::{resolve_anthropic_auth, resolve_grok_auth, resolve_openai_auth};
use crate::storage::credentials::CredentialStore;

use super::{
    persist_shared_current_model, AcpError, AvailableModelRecord, KrustyAgent, ModelConfig,
};
use crate::acp::session::{SessionModelSelection, SessionState};

#[derive(Clone)]
struct SelectedCredential {
    value: String,
    account_id: Option<String>,
}

fn openai_credential_for_scope(
    identities: &[CatalogCredential],
    auth_scope: ModelAuthScope,
) -> Option<SelectedCredential> {
    let desired_kind = match auth_scope {
        ModelAuthScope::ApiKey => CatalogAuthKind::ApiKey,
        ModelAuthScope::OAuth => CatalogAuthKind::OAuth,
    };
    identities
        .iter()
        .find(|identity| identity.kind == desired_kind)
        .map(|identity| SelectedCredential {
            value: identity.credential().to_string(),
            account_id: identity.account_id.clone(),
        })
}

fn credential_for_model(
    store: &CredentialStore,
    provider: ProviderId,
    model_id: &str,
    auth_scope: Option<ModelAuthScope>,
) -> Option<SelectedCredential> {
    match provider {
        ProviderId::OpenAI => {
            if let Some(auth_scope) = auth_scope {
                let identities =
                    crate::ai::catalog::credentials_for_dynamic_models(provider, store);
                return openai_credential_for_scope(&identities, auth_scope);
            }
            let resolved = resolve_openai_auth(store, model_id);
            resolved.credential.map(|value| SelectedCredential {
                value,
                account_id: resolved.account_id,
            })
        }
        ProviderId::Anthropic => {
            resolve_anthropic_auth(store)
                .credential
                .map(|value| SelectedCredential {
                    value,
                    account_id: None,
                })
        }
        ProviderId::Grok => resolve_grok_auth(store)
            .credential
            .map(|value| SelectedCredential {
                value,
                account_id: None,
            }),
        _ => store.get_auth(&provider).map(|value| SelectedCredential {
            value,
            account_id: None,
        }),
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
        let Some(credential) = credential_for_model(store, provider, &model_info.id, None) else {
            continue;
        };
        let model_id = format!("{}:{}", provider.storage_key(), model_info.id);
        models.push(AvailableModelRecord {
            acp_model_id: model_id,
            provider,
            model_id: model_info.id.clone(),
            credential: credential.value,
            display_name: model_info.display_name.clone(),
            auth_scope: None,
            account_id: credential.account_id,
        });
        tracing::debug!(
            "Added model: {} from {:?}",
            model_info.display_name,
            provider
        );
    }
}

impl KrustyAgent {
    /// Detect all available models from configured providers.
    pub(super) async fn detect_available_models(&self) -> Vec<AvailableModelRecord> {
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

            let catalog_identities =
                crate::ai::catalog::credentials_for_dynamic_models(provider, &store);
            if catalog_identities.is_empty() {
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
                        let model_credential = if provider == ProviderId::OpenAI {
                            match model.auth_scope {
                                Some(scope) => {
                                    openai_credential_for_scope(&catalog_identities, scope)
                                }
                                None => credential_for_model(&store, provider, &model.id, None),
                            }
                        } else {
                            credential_for_model(&store, provider, &model.id, model.auth_scope)
                        };
                        let Some(model_credential) = model_credential else {
                            continue;
                        };
                        let model_id = format!("{}:{}", provider.storage_key(), model.id);
                        models.push(AvailableModelRecord {
                            acp_model_id: model_id,
                            provider,
                            model_id: model.id,
                            credential: model_credential.value,
                            display_name: model.display_name,
                            auth_scope: model.auth_scope,
                            account_id: model_credential.account_id,
                        });
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
        let provider = model_config.provider;
        let actual_model_id = model_config.model_id.clone();
        let listed_credential = SelectedCredential {
            value: model_config.credential,
            account_id: model_config.account_id,
        };
        let selected_credential = CredentialStore::load()
            .ok()
            .and_then(|store| {
                credential_for_model(&store, provider, &actual_model_id, model_config.auth_scope)
            })
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

        self.processor.write().await.init_ai_client_with_auth_scope(
            selected_credential.value,
            provider,
            Some(actual_model_id),
            model_config.auth_scope,
            selected_credential.account_id,
        );

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
        let provider = model_config.provider;
        let actual_model_id = model_config.model_id.clone();
        let listed_credential = SelectedCredential {
            value: model_config.credential,
            account_id: model_config.account_id,
        };
        let selected_credential = CredentialStore::load()
            .ok()
            .and_then(|store| {
                credential_for_model(&store, provider, &actual_model_id, model_config.auth_scope)
            })
            .unwrap_or(listed_credential);

        let client = self
            .processor
            .read()
            .await
            .build_ai_client_with_auth_scope(
                selected_credential.value,
                provider,
                Some(actual_model_id.clone()),
                model_config.auth_scope,
                selected_credential.account_id,
            )
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
                    acp_model_id: model_config.acp_model_id.clone(),
                },
                client,
            )
            .await;
        if persist {
            session.persist_model(&model_config.acp_model_id).await;
        }
        Ok(())
    }

    pub(super) async fn resolve_persisted_model_id(&self, persisted: &str) -> Option<String> {
        self.available_models
            .read()
            .await
            .iter()
            .find(|record| record.acp_model_id == persisted || record.model_id == persisted)
            .map(|record| record.acp_model_id.clone())
    }

    async fn resolve_model_record(&self, model_id: &str) -> Result<AvailableModelRecord, AcpError> {
        self.available_models
            .read()
            .await
            .iter()
            .find(|record| record.acp_model_id == model_id)
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
