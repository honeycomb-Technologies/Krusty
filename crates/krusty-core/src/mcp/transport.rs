//! StreamableHttpClient implementation for reqwest 0.12
//!
//! The rmcp SDK defines a `StreamableHttpClient` trait for HTTP transports.
//! The built-in `transport-streamable-http-client-reqwest` feature requires
//! reqwest 0.13, but Krusty uses reqwest 0.12. We implement the trait
//! ourselves against 0.12 to avoid version conflicts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use futures::StreamExt;
use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sha2::{Digest, Sha256};
use sse_stream::{Error as SseError, Sse, SseStream};

const MAX_MCP_JSON_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MCP_JSON_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

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
                let body = read_bounded_mcp_json_response(response).await?;
                let message: ServerJsonRpcMessage =
                    serde_json::from_slice(&body).map_err(|error| {
                        StreamableHttpError::UnexpectedServerResponse(
                            safe_mcp_json_decode_error(&error, &body).into(),
                        )
                    })?;
                Ok(StreamableHttpPostResponse::Json(message, session_id))
            }
            other => Err(StreamableHttpError::UnexpectedContentType(
                other.map(|_| "unsupported".to_string()),
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

async fn read_bounded_mcp_json_response(
    response: reqwest::Response,
) -> Result<Vec<u8>, StreamableHttpError<ReqwestError>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MCP_JSON_RESPONSE_BYTES as u64)
    {
        return Err(StreamableHttpError::UnexpectedServerResponse(
            format!(
                "MCP JSON response exceeded byte limit (limit_bytes={MAX_MCP_JSON_RESPONSE_BYTES})"
            )
            .into(),
        ));
    }

    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(8 * 1024);
    let deadline = tokio::time::Instant::now() + MCP_JSON_RESPONSE_TIMEOUT;
    loop {
        let next = tokio::time::timeout_at(deadline, stream.next())
            .await
            .map_err(|_| {
                StreamableHttpError::UnexpectedServerResponse(
                    format!(
                        "MCP JSON response body timed out (limit_bytes={}, captured_bytes={}, response_fingerprint={})",
                        MAX_MCP_JSON_RESPONSE_BYTES,
                        body.len(),
                        mcp_response_fingerprint(&body)
                    )
                    .into(),
                )
            })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|_| {
            StreamableHttpError::UnexpectedServerResponse(
                format!(
                    "MCP JSON response body read failed (captured_bytes={}, response_fingerprint={})",
                    body.len(),
                    mcp_response_fingerprint(&body)
                )
                .into(),
            )
        })?;
        let remaining = MAX_MCP_JSON_RESPONSE_BYTES.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            return Err(StreamableHttpError::UnexpectedServerResponse(
                format!(
                    "MCP JSON response exceeded byte limit (limit_bytes={}, response_fingerprint={})",
                    MAX_MCP_JSON_RESPONSE_BYTES,
                    mcp_response_fingerprint(&body)
                )
                .into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn safe_mcp_json_decode_error(error: &serde_json::Error, body: &[u8]) -> String {
    let category = match error.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    };
    format!(
        "MCP JSON response was invalid (category={}, line={}, column={}, response_fingerprint={})",
        category,
        error.line(),
        error.column(),
        mcp_response_fingerprint(body)
    )
}

fn mcp_response_fingerprint(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"krusty-mcp-response-v1\0");
    hasher.update(body.len().to_le_bytes());
    hasher.update(body);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_json_decode_error_does_not_reflect_response_content() {
        const SENTINEL: &str = "MCP_RESPONSE_SENTINEL_86ac";
        let body = format!(r#"{{"jsonrpc":"2.0","result":"{SENTINEL}""#);
        let parse_error = serde_json::from_slice::<ServerJsonRpcMessage>(body.as_bytes())
            .expect_err("invalid JSON should fail");
        let safe_error = safe_mcp_json_decode_error(&parse_error, body.as_bytes());
        assert!(!safe_error.contains(SENTINEL));
        assert!(safe_error.contains("response_fingerprint=sha256:"));
    }

    #[test]
    fn mcp_response_limit_is_finite() {
        assert!(MAX_MCP_JSON_RESPONSE_BYTES <= 4 * 1024 * 1024);
        assert!(MCP_JSON_RESPONSE_TIMEOUT <= Duration::from_secs(10));
    }
}
