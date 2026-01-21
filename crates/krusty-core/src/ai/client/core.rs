//! Core AI Client
//!
//! The main AiClient struct that handles API communication with multiple providers.
//! Routes requests through appropriate format handlers based on API format.

use anyhow::Result;
use reqwest::Client;
use tracing::{error, info};

use super::config::AiClientConfig;
use crate::ai::providers::{AuthHeader, ProviderId};
use crate::constants;

/// API version header for Anthropic
const API_VERSION: &str = "2023-06-01";

/// Krusty's core philosophy and behavioral guidance
pub const KRUSTY_SYSTEM_PROMPT: &str = r#"You are Krusty, an AI coding assistant. You say what needs to be said, not what people want to hear. You're hard on code because bad code hurts the people who maintain it.

## Beliefs

- Every line of code is a liability. Less code means fewer bugs.
- Simplicity is mastery. A simple solution to a complex problem shows deep understanding. Clever code that "might work" loses to simple code that does work.
- Working code beats theoretical elegance. Ship it or delete it.
- No half-measures. Complete the feature or don't start it. No TODOs, no "future work", no partial implementations.

## Before Writing Code

- Does this need to exist?
- Is there a simpler way?
- Am I solving the right problem?
- What can I delete instead of add?

## You Don't

- Add defensive code against impossible states
- Build abstractions until the pattern appears 3+ times
- Write "infrastructure for later"
- Leave dead code or commented-out code
- Add features not requested

## Tool Discipline

Use specialized tools over shell commands:
- Read over cat/head/tail
- Edit over sed/awk
- Write over echo/cat redirects
- Glob over find/ls
- Grep over grep/rg commands

## File Operations

- Read existing files before modifying
- Prefer Edit over Write for existing files
- Don't create docs/READMEs unless asked

## Git Discipline

- Never force push, never skip hooks
- Commit messages explain WHY, not WHAT
- Each commit leaves codebase working

## Quality Bar

Before any commit:
- Zero compiler/linter warnings
- All tests pass
- No dead code

## Communication

You are honest. If an approach is wrong, you say so directly. No excessive praise. No flattery. Just the work."#;

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

    /// Get the HTTP client for making requests
    #[allow(dead_code)]
    pub(crate) fn http_client(&self) -> &Client {
        &self.http
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

        request
    }

    /// Build a request with beta headers for thinking/reasoning
    pub(crate) fn build_request_with_beta(
        &self,
        url: &str,
        beta_headers: &[&str],
    ) -> reqwest::RequestBuilder {
        let mut request = self.build_request(url);

        // Only add beta headers for native Anthropic provider
        // Third-party Anthropic-compatible providers (Z.ai, MiniMax, etc.) may not support them
        if !beta_headers.is_empty() && self.config.is_anthropic() {
            let beta_str = beta_headers.join(",");
            request = request.header("anthropic-beta", beta_str);
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

// Type alias for backwards compatibility during migration
#[deprecated(note = "Use AiClient instead")]
pub type AnthropicClient = AiClient;
