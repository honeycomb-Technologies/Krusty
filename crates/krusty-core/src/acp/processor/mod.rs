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
use crate::ai::providers::{get_provider, AuthHeader, ProviderId};
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
        use std::collections::HashMap;

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

        let provider_config = get_provider(provider);

        let (base_url, auth_header, custom_headers) = if let Some(pc) = provider_config {
            (
                Some(pc.base_url.clone()),
                pc.auth_header,
                pc.custom_headers.clone(),
            )
        } else {
            (None, AuthHeader::XApiKey, HashMap::new())
        };

        let api_format = detect_api_format(provider, &model);

        let config = AiClientConfig {
            model: model.clone(),
            max_tokens: ACP_DEFAULT_MAX_TOKENS,
            base_url,
            auth_header,
            provider_id: provider,
            api_format,
            custom_headers,
        };

        let client = Arc::new(AiClient::new(config, api_key));
        self.ai_client = Some(client);

        tracing::info!(
            "AI client initialized: provider={:?}, model={}",
            provider,
            model
        );
        true
    }
}
