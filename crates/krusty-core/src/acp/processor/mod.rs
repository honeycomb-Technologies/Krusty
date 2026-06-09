//! ACP prompt processor.
//!
//! Keeps ACP-to-AI wiring separate from the streaming loop and ACP content
//! adaptation helpers so the main processor type stays focused.

mod content;
mod loop_impl;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::agent::AgentConfig;
use crate::ai::client::{AiClient, AiClientConfig};
use crate::ai::format_detection::detect_api_format;
use crate::ai::providers::{get_provider, AuthHeader, ProviderConfig, ProviderId};
use crate::auth::{resolve_anthropic_auth, resolve_openai_auth, AnthropicAuthType, OpenAIAuthType};
use crate::storage::CredentialStore;
use crate::tools::git_identity::GitIdentity;
use crate::tools::ToolRegistry;

/// ACP default token budget for direct prompt calls.
const ACP_DEFAULT_MAX_TOKENS: usize = 8192;

/// Prompt processor that connects ACP to Krusty's AI and tools.
pub struct PromptProcessor {
    /// AI client for making inference calls.
    ai_client: Option<Arc<AiClient>>,
    /// Tool registry for executing tools.
    tools: Arc<ToolRegistry>,
    /// Git identity for commit attribution.
    git_identity: Option<GitIdentity>,
    /// Shared agent runtime config.
    agent_config: AgentConfig,
}

impl PromptProcessor {
    /// Create a new prompt processor.
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self {
            ai_client: None,
            tools,
            git_identity: Some(GitIdentity::default()),
            agent_config: AgentConfig::default(),
        }
    }

    /// Set git identity for commit attribution.
    pub fn set_git_identity(&mut self, identity: GitIdentity) {
        self.git_identity = Some(identity);
    }

    /// Initialize the AI client with an API key and explicit model selection.
    pub fn init_ai_client(
        &mut self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
    ) -> bool {
        let Some(model) = model_override
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
        else {
            self.ai_client = None;
            tracing::warn!(
                "ACP AI client not initialized for {:?}: no model selected",
                provider
            );
            return false;
        };

        let mut config = self.config_for_selected_credential(provider, &model, &api_key);
        config.max_tokens = ACP_DEFAULT_MAX_TOKENS;

        let client = Arc::new(AiClient::new(config, api_key));
        self.ai_client = Some(client);

        tracing::info!(
            "AI client initialized: provider={:?}, model={}",
            provider,
            model
        );
        true
    }

    fn config_for_selected_credential(
        &self,
        provider: ProviderId,
        model: &str,
        credential: &str,
    ) -> AiClientConfig {
        match provider {
            ProviderId::OpenAI => self.openai_config_for_selected_credential(model, credential),
            ProviderId::Anthropic => {
                self.anthropic_config_for_selected_credential(model, credential)
            }
            ProviderId::Grok => AiClientConfig::for_grok(model),
            _ => self.generic_provider_config(provider, model),
        }
    }

    fn openai_config_for_selected_credential(
        &self,
        model: &str,
        credential: &str,
    ) -> AiClientConfig {
        let credentials = CredentialStore::load().unwrap_or_default();
        let resolved = resolve_openai_auth(&credentials, model);
        let auth_type = if resolved.credential.as_deref() == Some(credential) {
            resolved.auth_type
        } else {
            OpenAIAuthType::ApiKey
        };
        let mut custom_headers = std::collections::HashMap::new();
        if matches!(auth_type, OpenAIAuthType::ChatGptOAuth) {
            if let Some(account_id) = resolved.account_id {
                custom_headers.insert("ChatGPT-Account-Id".to_string(), account_id);
            }
        }

        AiClientConfig {
            model: model.to_string(),
            max_tokens: ACP_DEFAULT_MAX_TOKENS,
            base_url: Some(ProviderConfig::openai_url_for_auth(model, auth_type).to_string()),
            auth_header: AuthHeader::Bearer,
            provider_id: ProviderId::OpenAI,
            api_format: ProviderConfig::openai_format_for_auth(model, auth_type),
            custom_headers,
        }
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
