use crate::ai::catalog::{CatalogAuthKind, CatalogCredential};
use crate::ai::format_detection::detect_api_format;
use crate::ai::models::{resolve_model_metadata, ModelAuthScope, ModelCatalogSource, ModelKey};
use crate::ai::providers::{get_provider, ProviderId};
use crate::auth::{resolve_anthropic_auth, resolve_grok_auth, resolve_openai_auth};
use crate::storage::credentials::CredentialStore;

use super::{
    acp_model_id_for_key, decode_acp_model_id, persist_shared_current_model, AcpError,
    AvailableModelRecord, MitsuroAgent, ModelConfig,
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
        let api_format = model_info
            .api_format
            .unwrap_or_else(|| detect_api_format(provider, &model_info.id));
        let mut metadata = resolve_model_metadata(provider, &model_info.id, api_format);
        metadata.catalog_source = ModelCatalogSource::Curated;

        // OpenAI API-key and OAuth rows can share the same wire slug while
        // requiring different endpoints and headers. Preserve both surfaces
        // even when live discovery falls back to the curated catalog.
        if provider == ProviderId::OpenAI {
            let identities = crate::ai::catalog::credentials_for_dynamic_models(provider, store);
            for auth_scope in [ModelAuthScope::ApiKey, ModelAuthScope::OAuth] {
                let Some(credential) = openai_credential_for_scope(&identities, auth_scope) else {
                    continue;
                };
                let mut scoped_metadata = metadata.clone();
                scoped_metadata.auth_scope = Some(auth_scope);
                if auth_scope == ModelAuthScope::OAuth {
                    scoped_metadata.api_format = crate::ai::models::ApiFormat::OpenAIResponses;
                }
                models.push(AvailableModelRecord::new(
                    scoped_metadata,
                    credential.value,
                    credential.account_id,
                ));
            }
        } else if let Some(credential) = credential_for_model(store, provider, &model_info.id, None)
        {
            models.push(AvailableModelRecord::new(
                metadata,
                credential.value,
                credential.account_id,
            ));
        }
        tracing::debug!(
            "Added model: {} from {:?}",
            model_info.display_name,
            provider
        );
    }
}

impl MitsuroAgent {
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
                        models.push(AvailableModelRecord::new(
                            model,
                            model_credential.value,
                            model_credential.account_id,
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
        let key = model_config.key().clone();
        let listed_credential = SelectedCredential {
            value: model_config.credential,
            account_id: model_config.account_id,
        };
        let selected_credential = CredentialStore::load()
            .ok()
            .and_then(|store| {
                credential_for_model(&store, key.provider, &key.model_id, key.auth_scope)
            })
            .unwrap_or(listed_credential);

        tracing::info!(
            "Switching to model: {} (provider: {:?})",
            key.model_id,
            key.provider
        );

        let initialized = self.processor.write().await.init_ai_client_for_metadata(
            selected_credential.value,
            &model_config.metadata,
            model_config.runtime,
            selected_credential.account_id,
        );
        if !initialized {
            return Err(AcpError::NotAuthenticated(format!(
                "Unable to initialize exact model {}",
                key.model_id
            )));
        }
        *self.current_model.write().await = Some(ModelConfig { key: key.clone() });
        persist_shared_current_model(&key);

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
        let key = model_config.key().clone();
        let listed_credential = SelectedCredential {
            value: model_config.credential,
            account_id: model_config.account_id,
        };
        let selected_credential = CredentialStore::load()
            .ok()
            .and_then(|store| {
                credential_for_model(&store, key.provider, &key.model_id, key.auth_scope)
            })
            .unwrap_or(listed_credential);

        let client = self
            .processor
            .read()
            .await
            .build_ai_client_for_metadata(
                selected_credential.value,
                &model_config.metadata,
                model_config.runtime.clone(),
                selected_credential.account_id,
            )
            .ok_or_else(|| {
                AcpError::NotAuthenticated(format!("Unable to initialize model {}", key.model_id))
            })?;
        session
            .set_model_client(
                SessionModelSelection {
                    key,
                    acp_model_id: model_config.acp_model_id.clone(),
                    catalog_revision: model_config.runtime.catalog_revision.clone(),
                },
                client,
            )
            .await;
        if persist {
            session.persist_model_selection().await;
        }
        Ok(())
    }

    pub(super) async fn resolve_persisted_model_id(&self, persisted: &str) -> Option<String> {
        let models = self.available_models.read().await;

        if let Some(record) = models
            .iter()
            .find(|record| record.acp_model_id == persisted)
        {
            return Some(record.acp_model_id.clone());
        }
        if let Some(key) = decode_acp_model_id(persisted) {
            return models
                .iter()
                .find(|record| record.key() == &key)
                .map(|record| record.acp_model_id.clone());
        }

        // Legacy ACP versions persisted either a bare slug or provider:slug.
        // Restore only when that lossy identifier maps to one exact key.
        let mut matches = models
            .iter()
            .filter(|record| {
                record.key().model_id == persisted
                    || format!(
                        "{}:{}",
                        record.key().provider.storage_key(),
                        record.key().model_id
                    ) == persisted
            })
            .map(|record| record.acp_model_id.clone())
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        (matches.len() == 1).then(|| matches.remove(0))
    }

    pub(super) async fn resolve_model_key(&self, key: &ModelKey) -> Option<String> {
        self.available_models
            .read()
            .await
            .iter()
            .find(|record| record.key() == key)
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
            .map(|model| acp_model_id_for_key(&model.key))
    }
}
