use anyhow::Result;
use reqwest::Client;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tracing::{error, info};

use crate::ai::providers::{AuthHeader, ProviderId};
use crate::ai::transport_policy::{build_provider_http_client, provider_http_client_builder};
use crate::constants;

use super::client::AiClient;

/// API version header for Anthropic
const API_VERSION: &str = "2023-06-01";

/// OAuth beta header required for Bearer token auth on Anthropic
const OAUTH_BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20";

impl AiClient {
    /// Create the HTTP client with configuration optimized for SSE streaming
    pub(super) fn create_http_client() -> Client {
        provider_http_client_builder()
            .user_agent("Mitsuro/1.0")
            .connect_timeout(constants::http::CONNECT_TIMEOUT)
            .timeout(constants::http::STREAM_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                error!(
                    "Failed to build configured HTTP client: {}. Using fail-closed fallback client.",
                    e
                );
                build_provider_http_client().unwrap_or_else(|fallback_error| {
                    panic!(
                        "failed to build provider HTTP client without redirects: {fallback_error}"
                    )
                })
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

        let error = crate::ai::retry::provider_http_error(response, "API error").await;
        error.log();
        Err(error.into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use reqwest::StatusCode;
    use tiny_http::{Header, Response, Server};

    use super::*;

    #[tokio::test]
    async fn shared_provider_http_client_returns_redirect_without_replaying_request() {
        let server = Server::http("127.0.0.1:0").expect("test server should bind");
        let base_url = format!("http://{}", server.server_addr());
        let redirect_url = format!("{base_url}/followed");
        let (request_tx, request_rx) = mpsc::channel();
        let server_thread = thread::spawn(move || {
            let request = server.recv().expect("initial request should arrive");
            let initial_path = request.url().to_string();
            request
                .respond(
                    Response::from_string("redirect")
                        .with_status_code(307)
                        .with_header(
                            Header::from_bytes("Location", redirect_url)
                                .expect("redirect header should be valid"),
                        ),
                )
                .expect("redirect should be sent");

            let followed_path = server
                .recv_timeout(Duration::from_millis(300))
                .expect("redirect probe should succeed")
                .map(|request| {
                    let path = request.url().to_string();
                    request
                        .respond(Response::from_string("followed"))
                        .expect("followed response should be sent");
                    path
                });
            request_tx
                .send((initial_path, followed_path))
                .expect("request evidence should be recorded");
        });

        let response = AiClient::create_http_client()
            .post(format!("{base_url}/initial"))
            .body("one governed request")
            .send()
            .await
            .expect("redirect response should be returned");

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        server_thread.join().expect("server thread should finish");
        let (initial_path, followed_path) = request_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("request evidence should arrive");
        assert_eq!(initial_path, "/initial");
        assert_eq!(followed_path, None);
    }
}
