use reqwest::Client;

use crate::ai::client::config::{AiClientConfig, CallOptions};
use crate::ai::client::streaming::codex::session::CodexWsPool;
use crate::ai::model_profile::{build_system_prompt_sections, SystemPromptSections};
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, ModelMessage};

/// AI API client supporting multiple providers
pub struct AiClient {
    pub(super) http: Client,
    pub(super) config: AiClientConfig,
    pub(super) api_key: String,
    pub(crate) codex_ws_pool: CodexWsPool,
}

impl AiClient {
    /// Create a new client with API key
    pub fn new(config: AiClientConfig, api_key: String) -> Self {
        Self {
            http: Self::create_http_client(),
            config,
            api_key,
            codex_ws_pool: CodexWsPool::default(),
        }
    }

    /// Alias for new() - backwards compatible
    pub fn with_api_key(config: AiClientConfig, api_key: String) -> Self {
        Self::new(config, api_key)
    }

    /// Get the API key
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Get the provider ID for this client
    pub fn provider_id(&self) -> ProviderId {
        self.config.provider_id()
    }

    /// Get the current configuration
    pub fn config(&self) -> &AiClientConfig {
        &self.config
    }

    pub(crate) fn canonical_call_options(&self, model: &str, options: &CallOptions) -> CallOptions {
        options.canonicalized_for(self.provider_id(), model, self.config().api_format)
    }

    pub(crate) fn system_prompt_sections(
        &self,
        model: &str,
        messages: &[ModelMessage],
        custom_system_prompt: Option<&str>,
        _tools: Option<&[AiTool]>,
    ) -> SystemPromptSections {
        build_system_prompt_sections(
            self.provider_id(),
            self.config().api_format,
            model,
            messages,
            custom_system_prompt,
            &[],
        )
    }
}
