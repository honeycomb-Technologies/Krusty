//! StreamableHttpClient implementation for reqwest 0.12
//!
//! The rmcp SDK defines a `StreamableHttpClient` trait for HTTP transports.
//! The built-in `transport-streamable-http-client-reqwest` feature requires
//! reqwest 0.13, but Krusty uses reqwest 0.12. We implement the trait
//! ourselves against 0.12 to avoid version conflicts.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::BoxStream;
use futures::StreamExt;
use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::{Error as SseError, Sse, SseStream};

/// A reqwest 0.12 based implementation of rmcp's `StreamableHttpClient` trait.
#[derive(Debug, Clone)]
pub struct ReqwestStreamableHttpClient {
    client: reqwest::Client,
}

impl Default for ReqwestStreamableHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestStreamableHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Build a request builder with common headers
    fn build_request(
        &self,
        method: reqwest::Method,
        uri: &str,
        session_id: Option<&str>,
        auth_header: Option<&str>,
        custom_headers: &HashMap<HeaderName, HeaderValue>,
    ) -> reqwest::RequestBuilder {
        let mut builder = self.client.request(method, uri);

        if let Some(sid) = session_id {
            builder = builder.header("Mcp-Session-Id", sid);
        }
        if let Some(auth) = auth_header {
            builder = builder.header("Authorization", auth);
        }
        for (name, value) in custom_headers {
            builder = builder.header(name.as_str(), value.as_bytes());
        }

        builder
    }
}

#[derive(Debug, thiserror::Error)]
#[error("reqwest error: {0}")]
pub struct ReqwestError(#[from] reqwest::Error);

impl StreamableHttpClient for ReqwestStreamableHttpClient {
    type Error = ReqwestError;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let body = serde_json::to_string(&message).map_err(StreamableHttpError::Deserialize)?;

        let response = self
            .build_request(
                reqwest::Method::POST,
                &uri,
                session_id.as_deref(),
                auth_header.as_deref(),
                &custom_headers,
            )
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(body)
            .send()
            .await
            .map_err(|e| StreamableHttpError::Client(ReqwestError(e)))?;

        let status = response.status();

        if status == reqwest::StatusCode::ACCEPTED {
            return Ok(StreamableHttpPostResponse::Accepted);
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(StreamableHttpError::SessionExpired);
        }

        if !status.is_success() {
            return Err(StreamableHttpError::UnexpectedServerResponse(
                format!("HTTP {}", status).into(),
            ));
        }

        let session_id = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        match content_type.as_deref() {
            Some(ct) if ct.contains("text/event-stream") => {
                let byte_stream = response.bytes_stream();
                let sse_stream = SseStream::from_byte_stream(byte_stream);
                let boxed: BoxStream<'static, Result<Sse, SseError>> = sse_stream.boxed();
                Ok(StreamableHttpPostResponse::Sse(boxed, session_id))
            }
            Some(ct) if ct.contains("application/json") => {
                let text = response
                    .text()
                    .await
                    .map_err(|e| StreamableHttpError::Client(ReqwestError(e)))?;
                let message: ServerJsonRpcMessage =
                    serde_json::from_str(&text).map_err(StreamableHttpError::Deserialize)?;
                Ok(StreamableHttpPostResponse::Json(message, session_id))
            }
            other => Err(StreamableHttpError::UnexpectedContentType(
                other.map(|s| s.to_string()),
            )),
        }
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let response = self
            .build_request(
                reqwest::Method::DELETE,
                &uri,
                Some(&session_id),
                auth_header.as_deref(),
                &custom_headers,
            )
            .send()
            .await
            .map_err(|e| StreamableHttpError::Client(ReqwestError(e)))?;

        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportDeleteSession);
        }

        if !response.status().is_success() {
            return Err(StreamableHttpError::UnexpectedServerResponse(
                format!("DELETE returned HTTP {}", response.status()).into(),
            ));
        }

        Ok(())
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut builder = self.build_request(
            reqwest::Method::GET,
            &uri,
            Some(&session_id),
            auth_header.as_deref(),
            &custom_headers,
        );
        builder = builder.header("Accept", "text/event-stream");

        if let Some(last_id) = &last_event_id {
            builder = builder.header("Last-Event-ID", last_id.as_str());
        }

        let response = builder
            .send()
            .await
            .map_err(|e| StreamableHttpError::Client(ReqwestError(e)))?;

        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }

        if !response.status().is_success() {
            return Err(StreamableHttpError::UnexpectedServerResponse(
                format!("GET stream returned HTTP {}", response.status()).into(),
            ));
        }

        let byte_stream = response.bytes_stream();
        let sse_stream = SseStream::from_byte_stream(byte_stream);
        Ok(sse_stream.boxed())
    }
}
