//! ACP prompt processor.
//!
//! Keeps ACP-to-AI wiring separate from the streaming loop and ACP content
//! adaptation helpers so the main processor type stays focused.

mod content;
mod loop_impl;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use crate::agent::AgentConfig;
use crate::ai::client::{AiClient, AiClientConfig};
use crate::ai::format_detection::detect_api_format;
use crate::ai::models::{
    resolve_model_metadata, ModelAuthScope, ModelMetadata, ResolvedModelRuntime,
};
use crate::ai::providers::{get_provider, AuthHeader, ProviderConfig, ProviderId};
use crate::auth::{
    resolve_anthropic_auth, resolve_openai_auth, AnthropicAuthType, OpenAIAuthResolution,
    OpenAIAuthType,
};
use crate::process::ProcessRegistry;
use crate::storage::CredentialStore;
use crate::tools::ToolRegistry;

/// ACP default token budget for direct prompt calls.
const ACP_DEFAULT_MAX_TOKENS: usize = 8192;

/// Prompt processor that connects ACP to Mitsuro's AI and tools.
pub struct PromptProcessor {
    /// AI client for making inference calls.
    ai_client: Option<Arc<AiClient>>,
    /// Tool registry for executing tools.
    tools: Arc<ToolRegistry>,
    /// Shared agent runtime config.
    agent_config: AgentConfig,
    /// Background process registry shared by ACP sessions in this connection.
    process_registry: Arc<ProcessRegistry>,
    /// Canonical database used by orchestrator persistence and compaction.
    db_path: PathBuf,
}

impl PromptProcessor {
    /// Create a new prompt processor.
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self::with_db_path(tools, crate::paths::config_dir().join("mitsuro.db"))
    }

    pub fn with_db_path(tools: Arc<ToolRegistry>, db_path: PathBuf) -> Self {
        Self {
            ai_client: None,
            tools,
            agent_config: AgentConfig::default(),
            process_registry: Arc::new(ProcessRegistry::new()),
            db_path,
        }
    }

    /// Initialize the AI client with an API key and explicit model selection.
    pub fn init_ai_client(
        &mut self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
    ) -> bool {
        self.init_ai_client_with_auth_scope(api_key, provider, model_override, None, None)
    }

    pub(super) fn init_ai_client_with_auth_scope(
        &mut self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
        auth_scope: Option<ModelAuthScope>,
        account_id: Option<String>,
    ) -> bool {
        let client = self.build_ai_client_with_auth_scope(
            api_key,
            provider,
            model_override,
            auth_scope,
            account_id,
        );
        self.ai_client = client;
        self.ai_client.is_some()
    }

    /// Initialize from the exact catalog row selected by ACP discovery.
    pub(super) fn init_ai_client_for_metadata(
        &mut self,
        api_key: String,
        metadata: &ModelMetadata,
        runtime: ResolvedModelRuntime,
        account_id: Option<String>,
    ) -> bool {
        self.ai_client = self.build_ai_client_for_metadata(api_key, metadata, runtime, account_id);
        self.ai_client.is_some()
    }

    /// Build an isolated client without mutating the processor's default model.
    pub fn build_ai_client(
        &self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
    ) -> Option<Arc<AiClient>> {
        self.build_ai_client_with_auth_scope(api_key, provider, model_override, None, None)
    }

    pub(super) fn build_ai_client_with_auth_scope(
        &self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
        auth_scope: Option<ModelAuthScope>,
        account_id: Option<String>,
    ) -> Option<Arc<AiClient>> {
        let Some(model) = model_override
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
        else {
            tracing::warn!(
                "ACP AI client not initialized for {:?}: no model selected",
                provider
            );
            return None;
        };

        let mut config =
            self.config_for_selected_credential(provider, &model, &api_key, auth_scope, account_id);
        config.max_tokens = ACP_DEFAULT_MAX_TOKENS;

        let mut metadata =
            resolve_model_metadata(config.provider_id, &config.model, config.api_format);
        metadata.auth_scope = auth_scope.or_else(|| {
            (config.provider_id == ProviderId::OpenAI).then_some(
                if config.uses_chatgpt_codex_format() {
                    ModelAuthScope::OAuth
                } else {
                    ModelAuthScope::ApiKey
                },
            )
        });
        let client =
            match AiClient::new_with_resolved_model(config, api_key, metadata.resolve_runtime()) {
                Ok(client) => Arc::new(client),
                Err(error) => {
                    tracing::error!(
                        provider = ?provider,
                        model = %model,
                        %error,
                        "Refusing ACP client whose transport disagrees with resolved model metadata"
                    );
                    return None;
                }
            };

        tracing::info!(
            "AI client initialized: provider={:?}, model={}",
            provider,
            model
        );
        Some(client)
    }

    /// Build an isolated client from one frozen catalog row. This path must be
    /// used by ACP model selection so dynamic capabilities, auth scope, and
    /// provider transport are not re-inferred from a wire slug.
    pub(super) fn build_ai_client_for_metadata(
        &self,
        api_key: String,
        metadata: &ModelMetadata,
        runtime: ResolvedModelRuntime,
        account_id: Option<String>,
    ) -> Option<Arc<AiClient>> {
        if runtime.key != metadata.key() || runtime.wire_model_id != metadata.id {
            tracing::error!(
                provider = ?metadata.provider,
                model = %metadata.id,
                "Refusing ACP client whose frozen runtime disagrees with its catalog row"
            );
            return None;
        }
        if metadata.provider == ProviderId::OpenAI
            && metadata.auth_scope == Some(ModelAuthScope::OAuth)
            && metadata.api_format != crate::ai::models::ApiFormat::OpenAIResponses
        {
            tracing::error!(
                model = %metadata.id,
                "Refusing an OpenAI OAuth ACP row that does not use the Responses transport"
            );
            return None;
        }

        let mut config = self.config_for_exact_metadata(metadata, &api_key, account_id);
        config.max_tokens = ACP_DEFAULT_MAX_TOKENS.min(runtime.capabilities.max_output);
        let client = match AiClient::new_with_resolved_model(config, api_key, runtime) {
            Ok(client) => Arc::new(client),
            Err(error) => {
                tracing::error!(
                    provider = ?metadata.provider,
                    model = %metadata.id,
                    %error,
                    "Refusing ACP client whose transport disagrees with exact catalog metadata"
                );
                return None;
            }
        };

        tracing::info!(
            "AI client initialized from exact ACP model row: provider={:?}, model={}",
            metadata.provider,
            metadata.id
        );
        Some(client)
    }

    pub fn default_ai_client(&self) -> Option<Arc<AiClient>> {
        self.ai_client.clone()
    }

    fn config_for_selected_credential(
        &self,
        provider: ProviderId,
        model: &str,
        credential: &str,
        auth_scope: Option<ModelAuthScope>,
        account_id: Option<String>,
    ) -> AiClientConfig {
        match provider {
            ProviderId::OpenAI => self
                .openai_config_for_selected_credential(model, credential, auth_scope, account_id),
            ProviderId::Anthropic => {
                self.anthropic_config_for_selected_credential(model, credential)
            }
            ProviderId::Grok => AiClientConfig::for_grok(model),
            _ => self.generic_provider_config(provider, model),
        }
    }

    fn config_for_exact_metadata(
        &self,
        metadata: &ModelMetadata,
        credential: &str,
        account_id: Option<String>,
    ) -> AiClientConfig {
        if metadata.provider == ProviderId::Grok {
            return AiClientConfig::for_grok_with_metadata(metadata);
        }

        let mut config = self.config_for_selected_credential(
            metadata.provider,
            &metadata.id,
            credential,
            metadata.auth_scope,
            account_id,
        );

        // API-key OpenAI supports both public endpoints. Select the endpoint
        // advertised by the row instead of repeating model-name detection.
        if metadata.provider == ProviderId::OpenAI && !config.uses_chatgpt_codex_format() {
            use crate::ai::models::ApiFormat;
            use crate::ai::providers::{OPENAI_CHAT_API, OPENAI_RESPONSES_API};
            config.base_url = Some(
                match metadata.api_format {
                    ApiFormat::OpenAI => OPENAI_CHAT_API,
                    ApiFormat::OpenAIResponses => OPENAI_RESPONSES_API,
                    _ => config.base_url.as_deref().unwrap_or_default(),
                }
                .to_string(),
            );
        }
        config.api_format = metadata.api_format;
        config
    }

    fn openai_config_for_selected_credential(
        &self,
        model: &str,
        credential: &str,
        auth_scope: Option<ModelAuthScope>,
        account_id: Option<String>,
    ) -> AiClientConfig {
        let resolved = match auth_scope {
            Some(ModelAuthScope::ApiKey) => OpenAIAuthResolution {
                auth_type: OpenAIAuthType::ApiKey,
                credential: Some(credential.to_string()),
                account_id: None,
            },
            Some(ModelAuthScope::OAuth) => OpenAIAuthResolution {
                auth_type: OpenAIAuthType::ChatGptOAuth,
                credential: Some(credential.to_string()),
                account_id,
            },
            None => {
                let credentials = CredentialStore::load().unwrap_or_default();
                let resolved = resolve_openai_auth(&credentials, model);
                if resolved.credential.as_deref() == Some(credential) {
                    resolved
                } else {
                    OpenAIAuthResolution {
                        auth_type: OpenAIAuthType::ApiKey,
                        credential: Some(credential.to_string()),
                        account_id: None,
                    }
                }
            }
        };
        AiClientConfig::for_openai_with_auth_resolution(model, resolved)
    }

    fn anthropic_config_for_selected_credential(
        &self,
        model: &str,
        credential: &str,
    ) -> AiClientConfig {
        let credentials = CredentialStore::load().unwrap_or_default();
        let resolved = resolve_anthropic_auth(&credentials);
        let auth_type = if resolved.credential.as_deref() == Some(credential) {
            resolved.auth_type
        } else {
            AnthropicAuthType::ApiKey
        };

        let mut config = AiClientConfig::for_anthropic_with_auth_detection(model, &credentials);
        config.auth_header = ProviderConfig::anthropic_auth_header_for_auth(auth_type);
        if !matches!(auth_type, AnthropicAuthType::OAuth) {
            config.custom_headers.clear();
        }
        config
    }

    fn generic_provider_config(&self, provider: ProviderId, model: &str) -> AiClientConfig {
        let provider_config = get_provider(provider);
        let (base_url, auth_header, custom_headers) = if let Some(pc) = provider_config {
            (
                Some(pc.base_url.clone()),
                pc.auth_header,
                pc.custom_headers.clone(),
            )
        } else {
            (None, AuthHeader::XApiKey, std::collections::HashMap::new())
        };

        AiClientConfig {
            model: model.to_string(),
            max_tokens: ACP_DEFAULT_MAX_TOKENS,
            base_url,
            auth_header,
            provider_id: provider,
            api_format: detect_api_format(provider, model),
            custom_headers,
        }
    }
}
