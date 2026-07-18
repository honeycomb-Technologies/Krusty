//! StreamableHttpClient implementation for reqwest 0.12
//!
//! The rmcp SDK defines a `StreamableHttpClient` trait for HTTP transports.
//! The built-in `transport-streamable-http-client-reqwest` feature requires
//! reqwest 0.13, but Krusty uses reqwest 0.12. We implement the trait
//! ourselves against 0.12 to avoid version conflicts.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use futures::stream::BoxStream;
use futures::StreamExt;
use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::{Error as SseError, Sse, SseStream};

const MAX_MCP_HTTP_JSON_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MCP_SSE_EVENT_BYTES: usize = 1024 * 1024;

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
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("static MCP HTTP client configuration must be valid"),
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

#[derive(Debug, thiserror::Error)]
enum BoundedSseBodyError {
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("MCP SSE event exceeds the {limit} byte decompressed limit")]
    EventTooLarge { limit: usize },
}

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
                let byte_stream = bounded_sse_byte_stream(response);
                let sse_stream = SseStream::from_byte_stream(byte_stream);
                let boxed: BoxStream<'static, Result<Sse, SseError>> = sse_stream.boxed();
                Ok(StreamableHttpPostResponse::Sse(boxed, session_id))
            }
            Some(ct) if ct.contains("application/json") => {
                let body = read_bounded_json_body(response).await?;
                let message: ServerJsonRpcMessage =
                    serde_json::from_slice(&body).map_err(StreamableHttpError::Deserialize)?;
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

        let byte_stream = bounded_sse_byte_stream(response);
        let sse_stream = SseStream::from_byte_stream(byte_stream);
        Ok(sse_stream.boxed())
    }
}

async fn read_bounded_json_body(
    response: reqwest::Response,
) -> Result<Bytes, StreamableHttpError<ReqwestError>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MCP_HTTP_JSON_RESPONSE_BYTES as u64)
    {
        return Err(response_limit_error(
            "JSON response",
            MAX_MCP_HTTP_JSON_RESPONSE_BYTES,
        ));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| StreamableHttpError::Client(ReqwestError(error)))?;
        if !extend_bounded(&mut bytes, &chunk, MAX_MCP_HTTP_JSON_RESPONSE_BYTES) {
            return Err(response_limit_error(
                "JSON response",
                MAX_MCP_HTTP_JSON_RESPONSE_BYTES,
            ));
        }
    }
    Ok(Bytes::from(bytes))
}

fn extend_bounded(buffer: &mut Vec<u8>, chunk: &[u8], limit: usize) -> bool {
    if chunk.len() > limit.saturating_sub(buffer.len()) {
        return false;
    }
    buffer.extend_from_slice(chunk);
    true
}

fn response_limit_error(
    response_kind: &'static str,
    limit: usize,
) -> StreamableHttpError<ReqwestError> {
    StreamableHttpError::UnexpectedServerResponse(
        format!("MCP {response_kind} exceeds the {limit} byte decompressed limit").into(),
    )
}

fn bounded_sse_byte_stream(
    response: reqwest::Response,
) -> BoxStream<'static, Result<Bytes, BoundedSseBodyError>> {
    response
        .bytes_stream()
        .scan(SseEventLimit::default(), |state, chunk| {
            let next = if state.failed {
                None
            } else {
                Some(match chunk {
                    Ok(bytes) => match state.observe(&bytes) {
                        Ok(()) => Ok(bytes),
                        Err(error) => {
                            state.failed = true;
                            Err(error)
                        }
                    },
                    Err(error) => {
                        state.failed = true;
                        Err(BoundedSseBodyError::Reqwest(error))
                    }
                })
            };
            futures::future::ready(next)
        })
        .boxed()
}

#[derive(Debug, Default)]
struct SseEventLimit {
    event_bytes: usize,
    line_has_content: bool,
    previous_was_cr: bool,
    failed: bool,
}

impl SseEventLimit {
    fn observe(&mut self, bytes: &[u8]) -> Result<(), BoundedSseBodyError> {
        self.observe_with_limit(bytes, MAX_MCP_SSE_EVENT_BYTES)
    }

    fn observe_with_limit(
        &mut self,
        bytes: &[u8],
        limit: usize,
    ) -> Result<(), BoundedSseBodyError> {
        for byte in bytes {
            // CRLF is one line ending. The CR was already accounted for and
            // finalized the line, so the following LF is not a blank line.
            if self.previous_was_cr && *byte == b'\n' {
                self.previous_was_cr = false;
                continue;
            }
            self.previous_was_cr = false;
            self.event_bytes = self
                .event_bytes
                .checked_add(1)
                .ok_or(BoundedSseBodyError::EventTooLarge { limit })?;

            match byte {
                b'\r' => {
                    self.finish_line();
                    self.previous_was_cr = true;
                }
                b'\n' => self.finish_line(),
                _ => self.line_has_content = true,
            }
            if self.event_bytes > limit {
                return Err(BoundedSseBodyError::EventTooLarge { limit });
            }
        }
        Ok(())
    }

    fn finish_line(&mut self) {
        if !self.line_has_content {
            self.event_bytes = 0;
        }
        self.line_has_content = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_limit_is_per_event_and_handles_split_crlf() {
        let mut limit = SseEventLimit::default();
        limit.observe_with_limit(b"data: 1\r", 16).unwrap();
        limit.observe_with_limit(b"\n\r", 16).unwrap();
        limit.observe_with_limit(b"\ndata: 2\n\n", 16).unwrap();
        assert!(limit
            .observe_with_limit(b"data: this event is too large", 8)
            .is_err());
    }

    #[test]
    fn json_body_limit_is_aggregate_and_does_not_append_overflow_chunk() {
        let mut body = Vec::new();
        assert!(extend_bounded(&mut body, b"1234", 6));
        assert!(!extend_bounded(&mut body, b"789", 6));
        assert_eq!(body, b"1234");
        assert!(extend_bounded(&mut body, b"56", 6));
        assert_eq!(body, b"123456");
    }

    #[tokio::test]
    async fn mcp_http_client_does_not_follow_redirects() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let server_thread = std::thread::spawn(move || {
            let request = server
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .expect("redirect test request");
            let location = tiny_http::Header::from_bytes("Location", "/redirected").unwrap();
            request
                .respond(tiny_http::Response::empty(302).with_header(location))
                .unwrap();
        });

        let client = ReqwestStreamableHttpClient::new();
        let response = client
            .build_request(
                reqwest::Method::GET,
                &format!("http://{address}/origin"),
                None,
                None,
                &HashMap::new(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        server_thread.join().unwrap();
    }
}
