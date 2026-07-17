use anyhow::Result;
use reqwest::header::RETRY_AFTER;
use reqwest::Client;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tracing::{error, info};

use crate::ai::providers::{AuthHeader, ProviderId};
use crate::constants;

use super::client::AiClient;

/// API version header for Anthropic
const API_VERSION: &str = "2023-06-01";

/// OAuth beta header required for Bearer token auth on Anthropic
const OAUTH_BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20";

impl AiClient {
    /// Create the HTTP client with configuration optimized for SSE streaming
    pub(super) fn create_http_client() -> Client {
        Client::builder()
            .user_agent("Krusty/1.0")
            .connect_timeout(constants::http::CONNECT_TIMEOUT)
            .timeout(constants::http::STREAM_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                error!("Failed to build HTTP client: {}. Using default client.", e);
                Client::new()
            })
    }

    /// Build a request with proper authentication headers
    pub(crate) fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut request = self.http.post(url);

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

        if self.config.uses_anthropic_api() {
            request = request.header("anthropic-version", API_VERSION);

            if self.config.auth_header == AuthHeader::Bearer
                && self.config.provider_id() == ProviderId::Anthropic
            {
                request = request.header("anthropic-beta", OAUTH_BETA_HEADER);
            }
        }

        request = request.header("content-type", "application/json");

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

        if self.config.provider_id == ProviderId::Anthropic {
            let mut all_betas: Vec<&str> = Vec::new();

            if self.config.auth_header == AuthHeader::Bearer {
                all_betas.push(OAUTH_BETA_HEADER);
            }

            all_betas.extend_from_slice(beta_headers);

            if !all_betas.is_empty() {
                let combined = all_betas.join(",");
                request = request.header("anthropic-beta", combined);
            }
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

        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(crate::ai::retry::parse_retry_after);
        let error_text = response.text().await.unwrap_or_default();
        error!("API error response: {} - {}", status, error_text);
        Err(crate::ai::retry::ProviderHttpError::new(
            "API error",
            status.as_u16(),
            status.to_string(),
            error_text,
            retry_after,
        )
        .into())
    }
}
