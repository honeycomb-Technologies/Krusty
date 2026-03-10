//! Core AI Client
//!
//! The main AiClient struct that handles API communication with multiple providers.
//! Routes requests through appropriate format handlers based on API format.

use anyhow::Result;
use reqwest::Client;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tracing::{error, info};

use super::config::{AiClientConfig, CallOptions};
use crate::ai::model_profile::{build_system_prompt_sections, SystemPromptSections};
use crate::ai::providers::{AuthHeader, ProviderId};
use crate::ai::types::ModelMessage;
use crate::constants;

/// API version header for Anthropic
const API_VERSION: &str = "2023-06-01";

/// Krusty's core philosophy and behavioral guidance
pub const KRUSTY_SYSTEM_PROMPT: &str = r#"You are Krusty, an AI coding assistant focused on finishing real software tasks cleanly.

## Operating Contract

- Work until the task is actually complete or you are blocked by a real external constraint.
- Do not stop early because the work is long, repetitive, or inconvenient.
- Inspect the codebase before making changes. Use evidence from the repository, not guesses.
- Preserve the user's intent. Do not silently widen scope or substitute a different task.

## Execution Rules

- Read before editing. Prefer the smallest correct change.
- When a bug has a concrete root cause, fix the cause rather than masking the symptom.
- If a task spans multiple steps, continue through them instead of returning a premature summary.
- If something fails, explain the failure precisely and try the next reasonable recovery path.
- Do not claim success while known errors, broken builds, or unfinished edits remain.

## Context Discipline

- Respect project instructions and local conventions.
- Keep important constraints in view across long tool-use loops.
- Do not drop relevant context just because the conversation is long; summarize and continue when needed.

## Tool Discipline

- Use structured tools when available.
- Read files instead of guessing contents.
- Edit existing files instead of rewriting them wholesale unless replacement is clearly the safest path.
- Avoid unnecessary shell indirection when a dedicated tool exists.

## Output Style

- Be direct, professional, and concrete.
- Report tradeoffs and risks plainly.
- No flattery, no filler, no performative certainty.
"#;

/// AI API client supporting multiple providers
pub struct AiClient {
    http: Client,
    config: AiClientConfig,
    api_key: String,
}

impl AiClient {
    /// Create the HTTP client with configuration optimized for SSE streaming
    fn create_http_client() -> Client {
        Client::builder()
            .user_agent("Krusty/1.0")
            .connect_timeout(constants::http::CONNECT_TIMEOUT)
            // Long timeout for streaming - extended thinking + large tool outputs can take 5+ minutes
            .timeout(constants::http::STREAM_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                error!("Failed to build HTTP client: {}. Using default client.", e);
                Client::new()
            })
    }

    /// Create a new client with API key
    pub fn new(config: AiClientConfig, api_key: String) -> Self {
        Self {
            http: Self::create_http_client(),
            config,
            api_key,
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
    ) -> SystemPromptSections {
        build_system_prompt_sections(
            self.provider_id(),
            self.config().api_format,
            model,
            messages,
            custom_system_prompt,
        )
    }

    /// Build a request with proper authentication headers
    pub(crate) fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut request = self.http.post(url);

        // Add auth header based on provider config
        match self.config.auth_header {
            AuthHeader::Bearer => {
                request = request.header("authorization", format!("Bearer {}", self.api_key));
                info!(
                    "Using Bearer authentication for {}",
                    self.config.provider_id
                );
            }
            AuthHeader::XApiKey => {
                request = request.header("x-api-key", &self.api_key);
                info!("Using API key authentication");
            }
        }

        // Add Anthropic API headers if using Anthropic-compatible API
        if self.config.uses_anthropic_api() {
            request = request.header("anthropic-version", API_VERSION);
        }

        // Common headers
        request = request.header("content-type", "application/json");

        // Add custom headers
        for (key, value) in &self.config.custom_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        request
    }

    /// Build a WebSocket request with auth + configured headers.
    pub(crate) fn build_websocket_request(
        &self,
        url: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
        let mut request = url
            .into_client_request()
            .map_err(|e| anyhow::anyhow!("Invalid websocket request URL: {}", e))?;

        let headers = request.headers_mut();

        match self.config.auth_header {
            AuthHeader::Bearer => {
                headers.insert("authorization", format!("Bearer {}", self.api_key).parse()?);
            }
            AuthHeader::XApiKey => {
                headers.insert("x-api-key", self.api_key.parse()?);
            }
        }

        headers.insert("content-type", "application/json".parse()?);

        for (key, value) in &self.config.custom_headers {
            if let (Ok(name), Ok(val)) = (
                key.parse::<tokio_tungstenite::tungstenite::http::header::HeaderName>(),
                value.parse::<tokio_tungstenite::tungstenite::http::HeaderValue>(),
            ) {
                headers.insert(name, val);
            }
        }

        for (key, value) in extra_headers {
            if let (Ok(name), Ok(val)) = (
                (*key).parse::<tokio_tungstenite::tungstenite::http::header::HeaderName>(),
                (*value).parse::<tokio_tungstenite::tungstenite::http::HeaderValue>(),
            ) {
                headers.insert(name, val);
            }
        }

        Ok(request)
    }

    /// Build a request with beta headers for thinking/reasoning
    pub(crate) fn build_request_with_beta(
        &self,
        url: &str,
        beta_headers: &[&str],
    ) -> reqwest::RequestBuilder {
        let mut request = self.build_request(url);

        // Beta headers for native Anthropic API (direct provider)
        // Third-party Anthropic-compatible providers (Z.ai, MiniMax, etc.) don't need them
        if self.config.provider_id == ProviderId::Anthropic && !beta_headers.is_empty() {
            let combined = beta_headers.join(",");
            request = request.header("anthropic-beta", combined);
            info!("Added anthropic-beta headers for Anthropic provider");
        }

        request
    }

    /// Handle an error response and return a formatted error
    pub(crate) async fn handle_error_response(
        &self,
        response: reqwest::Response,
    ) -> Result<reqwest::Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let error_text = response.text().await.unwrap_or_default();
        error!("API error response: {} - {}", status, error_text);
        Err(anyhow::anyhow!("API error: {} - {}", status, error_text))
    }
}
