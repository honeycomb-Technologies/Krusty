//! Builder for an authenticated reqwest client that speaks the way the
//! official Grok CLI does (headers, user-agent, etc.).

use crate::error::Result;
use crate::token::AuthToken;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use reqwest::Client;

#[derive(Debug, Clone)]
pub struct ClientBuilder {
    token: Option<AuthToken>,
    client_version: String,
    extra_headers: HeaderMap,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            token: None,
            client_version: crate::DEFAULT_CLIENT_VERSION.to_string(),
            extra_headers: HeaderMap::new(),
        }
    }
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_token(mut self, token: AuthToken) -> Self {
        self.token = Some(token);
        self
    }

    pub fn with_client_version(mut self, version: &str) -> Self {
        self.client_version = version.to_string();
        self
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        if let (Ok(n), Ok(v)) = (
            name.parse::<reqwest::header::HeaderName>(),
            value.parse::<reqwest::header::HeaderValue>(),
        ) {
            self.extra_headers.insert(n, v);
        }
        self
    }

    pub fn build(self) -> Result<AuthenticatedClient> {
        let mut headers = HeaderMap::new();

        if let Some(tok) = &self.token {
            let bearer = format!("Bearer {}", tok.access_token);
            headers.insert(AUTHORIZATION, HeaderValue::from_str(&bearer)?);
        }

        // Identification headers that the xAI backend uses for routing / quotas / debugging
        headers.insert(
            "x-grok-client-version",
            HeaderValue::from_str(&self.client_version)?,
        );

        // You can add x-grok-client-identifier, x-grok-*-id etc. from your harness here.
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Mozilla/5.0 (compatible; grok-agent/1.0; +https://x.ai)"),
        );

        for (k, v) in self.extra_headers {
            if let Some(k) = k {
                headers.insert(k, v);
            }
        }

        let client = Client::builder().default_headers(headers).build()?;

        Ok(AuthenticatedClient {
            inner: client,
            token: self.token,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedClient {
    inner: Client,
    token: Option<AuthToken>,
}

impl AuthenticatedClient {
    pub fn inner(&self) -> &Client {
        &self.inner
    }

    /// Current token (if any). Useful if you need to do raw WebSocket or other things.
    pub fn current_token(&self) -> Option<&AuthToken> {
        self.token.as_ref()
    }

    /// Convenience for JSON chat-style calls against the Grok backend.
    /// The exact path and body shape depend on whether you're hitting the public
    /// xAI API or the internal cli-chat-proxy used by the TUI.
    pub async fn post_json<T: serde::Serialize + ?Sized>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        Ok(self.inner.post(url).json(body).send().await?)
    }
}
