//! Exponential backoff with jitter
//!
//! Implements retry logic for transient API errors including rate limiting (429).

use std::fmt;
use std::future::Future;
use std::time::Duration;

use futures::StreamExt;
use rand::Rng;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{error, warn};

/// Maximum provider error payload retained long enough to derive safe metadata.
///
/// Provider failures are untrusted network input and frequently contain prompts,
/// credentials, or user data. The raw body is never retained on the error or
/// written to logs; this prefix is used only for a fingerprint and bounded JSON
/// metadata extraction.
const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_CORRELATION_METADATA_BYTES: usize = 512;
const PROVIDER_ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);

const REQUEST_ID_HEADERS: &[&str] = &[
    "x-request-id",
    "request-id",
    "openai-request-id",
    "x-correlation-id",
    "cf-ray",
    "x-amzn-requestid",
    "x-amz-request-id",
    "x-goog-request-id",
];

/// Codes in this list are fixed protocol vocabulary rather than reflected
/// provider content. Any unknown value is represented only by a fingerprint.
const KNOWN_PROVIDER_CODES: &[&str] = &[
    "api_error",
    "authentication_error",
    "bad_request",
    "billing_hard_limit_reached",
    "capacity_exceeded",
    "conflict",
    "content_policy_violation",
    "context_length_exceeded",
    "forbidden",
    "gateway_timeout",
    "insufficient_quota",
    "internal_server_error",
    "invalid_request_error",
    "model_not_found",
    "not_found",
    "not_found_error",
    "overloaded_error",
    "permission_error",
    "quota_exceeded",
    "rate_limit_error",
    "rate_limit_exceeded",
    "request_too_large",
    "resource_exhausted",
    "server_error",
    "service_unavailable",
    "too_many_requests",
    "unauthorized",
    "unprocessable_entity",
    "usage_limit_reached",
];

/// Structured provider HTTP failure retained across the transport boundary.
///
/// Keeping the status and `Retry-After` value typed lets the shared streaming
/// caller retry transient setup failures without parsing user-facing strings.
pub(crate) struct ProviderHttpError {
    label: String,
    status: u16,
    status_text: String,
    provider_code: Option<String>,
    provider_code_fingerprint: Option<String>,
    request_id_fingerprint: Option<String>,
    response_fingerprint: String,
    captured_body_bytes: usize,
    body_truncated: bool,
    body_read_failed: bool,
    body_read_timed_out: bool,
    retry_after: Option<Duration>,
}

impl ProviderHttpError {
    /// Compatibility constructor for typed errors synthesized by internal
    /// adapters and tests. The supplied body is inspected only through the
    /// same safe, bounded metadata path used for HTTP responses.
    #[cfg(test)]
    pub(crate) fn new(
        label: impl Into<String>,
        status: u16,
        _status_text: impl Into<String>,
        body: impl Into<String>,
        retry_after: Option<Duration>,
    ) -> Self {
        let body = body.into();
        let body_bytes = body.as_bytes();
        let captured_len = body_bytes.len().min(MAX_PROVIDER_ERROR_BODY_BYTES);
        Self::from_observation(
            label,
            status,
            &body_bytes[..captured_len],
            retry_after,
            None,
            body_bytes.len() > MAX_PROVIDER_ERROR_BODY_BYTES,
            false,
            false,
        )
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    pub(crate) fn retry_after_value(&self) -> Option<Duration> {
        self.retry_after
    }

    fn from_observation(
        label: impl Into<String>,
        status: u16,
        captured_body: &[u8],
        retry_after: Option<Duration>,
        header_request_id: Option<String>,
        body_truncated: bool,
        body_read_failed: bool,
        body_read_timed_out: bool,
    ) -> Self {
        let parsed_body = (!body_truncated && !body_read_failed && !body_read_timed_out)
            .then(|| serde_json::from_slice::<Value>(captured_body).ok())
            .flatten();
        let raw_provider_code = parsed_body.as_ref().and_then(extract_provider_code);
        let provider_code = raw_provider_code.and_then(known_provider_code);
        let provider_code_fingerprint = raw_provider_code
            .filter(|_| provider_code.is_none())
            .map(|value| metadata_fingerprint(b"provider-code", value.as_bytes()));
        let request_id_fingerprint =
            header_request_id.or_else(|| parsed_body.as_ref().and_then(extract_body_request_id));

        Self {
            label: sanitize_label(&label.into()),
            status,
            status_text: canonical_status_text(status),
            provider_code,
            provider_code_fingerprint,
            request_id_fingerprint,
            response_fingerprint: fingerprint(captured_body),
            captured_body_bytes: captured_body.len(),
            body_truncated,
            body_read_failed,
            body_read_timed_out,
            retry_after,
        }
    }

    /// Emit only the typed, bounded metadata carried by this error. Keeping
    /// this logging contract beside `Display`/`Debug` prevents transports from
    /// accidentally reintroducing raw response-body logging.
    pub(crate) fn log(&self) {
        error!(
            provider = %self.label,
            status = self.status,
            status_text = %self.status_text,
            provider_code = self.provider_code.as_deref().unwrap_or("unknown"),
            provider_code_fingerprint = self
                .provider_code_fingerprint
                .as_deref()
                .unwrap_or("unknown"),
            request_id_fingerprint = self
                .request_id_fingerprint
                .as_deref()
                .unwrap_or("unknown"),
            response_fingerprint = %self.response_fingerprint,
            captured_body_bytes = self.captured_body_bytes,
            body_truncated = self.body_truncated,
            body_read_failed = self.body_read_failed,
            body_read_timed_out = self.body_read_timed_out,
            retry_after_ms = self
                .retry_after
                .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64),
            "Provider API request failed"
        );
    }
}

impl fmt::Debug for ProviderHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHttpError")
            .field("label", &self.label)
            .field("status", &self.status)
            .field("status_text", &self.status_text)
            .field("provider_code", &self.provider_code)
            .field("provider_code_fingerprint", &self.provider_code_fingerprint)
            .field("request_id_fingerprint", &self.request_id_fingerprint)
            .field("response_fingerprint", &self.response_fingerprint)
            .field("captured_body_bytes", &self.captured_body_bytes)
            .field("body_truncated", &self.body_truncated)
            .field("body_read_failed", &self.body_read_failed)
            .field("body_read_timed_out", &self.body_read_timed_out)
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

impl fmt::Display for ProviderHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {} [", self.label, self.status_text)?;
        if let Some(provider_code) = &self.provider_code {
            write!(formatter, "code={provider_code}, ")?;
        }
        if let Some(provider_code_fingerprint) = &self.provider_code_fingerprint {
            write!(formatter, "code_fingerprint={provider_code_fingerprint}, ")?;
        }
        if let Some(request_id_fingerprint) = &self.request_id_fingerprint {
            write!(
                formatter,
                "request_id_fingerprint={request_id_fingerprint}, "
            )?;
        }
        write!(
            formatter,
            "response_fingerprint={}, captured_body_bytes={}",
            self.response_fingerprint, self.captured_body_bytes
        )?;
        if self.body_truncated {
            formatter.write_str(", body_truncated=true")?;
        }
        if self.body_read_failed {
            formatter.write_str(", body_read_failed=true")?;
        }
        if self.body_read_timed_out {
            formatter.write_str(", body_read_timed_out=true")?;
        }
        formatter.write_str("]")
    }
}

impl std::error::Error for ProviderHttpError {}

/// Convert a non-success provider response into a safe typed error.
///
/// Only a bounded prefix is consumed. The returned error contains a hash and
/// tightly validated correlation metadata, never the raw response body.
pub(crate) async fn provider_http_error(
    response: reqwest::Response,
    label: impl Into<String>,
) -> ProviderHttpError {
    provider_http_error_with_timeout(response, label, PROVIDER_ERROR_BODY_TIMEOUT).await
}

async fn provider_http_error_with_timeout(
    response: reqwest::Response,
    label: impl Into<String>,
    body_timeout: Duration,
) -> ProviderHttpError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    let request_id = extract_header_request_id(response.headers());
    let content_length = response.content_length();
    let mut stream = response.bytes_stream();
    let mut captured_body = Vec::with_capacity(
        content_length
            .unwrap_or_default()
            .min(MAX_PROVIDER_ERROR_BODY_BYTES as u64) as usize,
    );
    let mut body_truncated = false;
    let mut body_read_failed = false;
    let mut body_read_timed_out = false;
    let deadline = tokio::time::Instant::now() + body_timeout;

    loop {
        let next = match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(next) => next,
            Err(_) => {
                body_read_timed_out = true;
                break;
            }
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                body_read_failed = true;
                break;
            }
        };
        let remaining = MAX_PROVIDER_ERROR_BODY_BYTES.saturating_sub(captured_body.len());
        if chunk.len() > remaining {
            captured_body.extend_from_slice(&chunk[..remaining]);
            body_truncated = true;
            break;
        }

        captured_body.extend_from_slice(&chunk);
        if captured_body.len() == MAX_PROVIDER_ERROR_BODY_BYTES
            && content_length.is_none_or(|length| length > MAX_PROVIDER_ERROR_BODY_BYTES as u64)
        {
            // With no trustworthy length, reaching the cap is conservatively
            // treated as truncation rather than reading another network chunk.
            body_truncated = true;
            break;
        }
    }

    ProviderHttpError::from_observation(
        label,
        status.as_u16(),
        &captured_body,
        retry_after,
        request_id,
        body_truncated,
        body_read_failed,
        body_read_timed_out,
    )
}

fn canonical_status_text(status: u16) -> String {
    reqwest::StatusCode::from_u16(status)
        .map(|status| status.to_string())
        .unwrap_or_else(|_| status.to_string())
}

fn sanitize_label(label: &str) -> String {
    let mut sanitized = String::with_capacity(label.len().min(80));
    for character in label.trim().chars().take(80) {
        if character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_' | '/' | '.') {
            sanitized.push(character);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.trim().is_empty() {
        "Provider API error".to_string()
    } else {
        sanitized
    }
}

fn fingerprint(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mitsuro-provider-response-v1\0");
    hasher.update(body.len().to_le_bytes());
    hasher.update(body);
    format!("sha256:{:x}", hasher.finalize())
}

fn extract_header_request_id(headers: &HeaderMap) -> Option<String> {
    REQUEST_ID_HEADERS.iter().find_map(|header| {
        headers
            .get(*header)
            .map(reqwest::header::HeaderValue::as_bytes)
            .filter(|value| !value.is_empty())
            .map(|value| metadata_fingerprint(b"request-id", value))
    })
}

fn extract_provider_code(body: &Value) -> Option<&str> {
    [
        body.pointer("/error/code"),
        body.pointer("/error/type"),
        body.pointer("/code"),
        body.pointer("/type"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
}

fn extract_body_request_id(body: &Value) -> Option<String> {
    [
        body.pointer("/request_id"),
        body.pointer("/requestId"),
        body.pointer("/error/request_id"),
        body.pointer("/error/requestId"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .filter(|value| !value.is_empty())
    .map(|value| metadata_fingerprint(b"request-id", value.as_bytes()))
}

fn known_provider_code(value: &str) -> Option<String> {
    let value = value.trim();
    KNOWN_PROVIDER_CODES
        .iter()
        .find(|known| value.eq_ignore_ascii_case(known))
        .map(|known| (*known).to_string())
}

/// Preserve fixed protocol vocabulary while preventing provider-controlled
/// codes and status strings from crossing into client-visible events verbatim.
pub(crate) fn safe_provider_code(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "unknown".to_string();
    }

    known_provider_code(value).unwrap_or_else(|| {
        format!(
            "unknown:{}",
            metadata_fingerprint(b"provider-code", value.as_bytes())
        )
    })
}

fn metadata_fingerprint(domain: &[u8], value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mitsuro-provider-metadata-v1\0");
    hasher.update(domain.len().to_le_bytes());
    hasher.update(domain);
    hasher.update(value.len().to_le_bytes());
    hasher.update(&value[..value.len().min(MAX_CORRELATION_METADATA_BYTES)]);
    format!("sha256:{:x}", hasher.finalize())
}

/// Build a provider-event failure string without reflecting any untrusted
/// message, type, or code. Fixed protocol vocabulary may be shown verbatim;
/// every other provider-controlled value is represented only by a stable hash.
pub(crate) fn safe_provider_event_error(
    label: &str,
    code: Option<&str>,
    category: Option<&str>,
    message: Option<&str>,
) -> String {
    let mut fields = Vec::with_capacity(3);

    if let Some(code) = code.filter(|value| !value.is_empty()) {
        if let Some(code) = known_provider_code(code) {
            fields.push(format!("code={code}"));
        } else {
            fields.push(format!(
                "code_fingerprint={}",
                metadata_fingerprint(b"provider-code", code.as_bytes())
            ));
        }
    }

    if let Some(category) = category.filter(|value| !value.is_empty()) {
        if let Some(category) = known_provider_code(category) {
            fields.push(format!("category={category}"));
        } else {
            fields.push(format!(
                "category_fingerprint={}",
                metadata_fingerprint(b"provider-category", category.as_bytes())
            ));
        }
    }

    if let Some(message) = message.filter(|value| !value.is_empty()) {
        fields.push(format!(
            "message_fingerprint={}",
            metadata_fingerprint(b"provider-message", message.as_bytes())
        ));
    }

    if fields.is_empty() {
        fields.push("metadata=unavailable".to_string());
    }
    format!("{} [{}]", sanitize_label(label), fields.join(", "))
}

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Whether to add random jitter to delays
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(32),
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// Create a configuration optimized for aggressive rate limit handling
    pub fn aggressive() -> Self {
        Self {
            max_retries: 8,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(60),
            jitter: true,
        }
    }

    /// Create a configuration for gentle retries (fewer attempts, shorter waits)
    pub fn gentle() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
            jitter: true,
        }
    }

    /// Bounded retries for an interactive streaming request before any output
    /// has been exposed to the caller.
    pub fn interactive_stream() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(8),
            jitter: true,
        }
    }
}

/// Trait for errors that may be retryable
pub trait IsRetryable {
    /// Check if this error is retryable
    fn is_retryable(&self) -> bool;

    /// Get the retry-after duration if specified by the server
    fn retry_after(&self) -> Option<Duration>;
}

impl IsRetryable for ProviderHttpError {
    fn is_retryable(&self) -> bool {
        is_retryable_status(self.status)
    }

    fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

impl IsRetryable for anyhow::Error {
    fn is_retryable(&self) -> bool {
        if let Some(error) = self.downcast_ref::<ProviderHttpError>() {
            return error.is_retryable();
        }

        // A generic response timeout may occur after the provider accepted and
        // billed a request. Retry only definite connection-establishment
        // failures unless the provider supplied a typed retryable status.
        self.downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_connect)
    }

    fn retry_after(&self) -> Option<Duration> {
        self.downcast_ref::<ProviderHttpError>()
            .and_then(IsRetryable::retry_after)
    }
}

/// Classify failures that occur while opening an interactive provider stream.
///
/// This boundary is narrower than the general [`IsRetryable`] contract: the
/// caller has not received a response or exposed provider output yet. Reqwest
/// can report an HTTP/2 reset or a connection closed during request dispatch as
/// a request error rather than a connect error. Those failures are safe to
/// retry here with the same bounded budget used for transient HTTP statuses.
pub(crate) fn is_retryable_interactive_stream_error(error: &anyhow::Error) -> bool {
    if error.is_retryable() {
        return true;
    }

    error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
        error.status().is_none()
            && !error.is_builder()
            && (error.is_request() || error.is_timeout())
    })
}

/// HTTP status codes that should trigger retry
pub const RETRYABLE_STATUS_CODES: &[u16] = &[
    429, // Too Many Requests
    500, // Internal Server Error
    502, // Bad Gateway
    503, // Service Unavailable
    504, // Gateway Timeout
    529, // Provider overloaded (Anthropic-compatible APIs)
];

/// Check if an HTTP status code is retryable
pub fn is_retryable_status(status: u16) -> bool {
    RETRYABLE_STATUS_CODES.contains(&status)
}

/// Extract a provider HTTP status from the common error formats retained by
/// Mitsuro's transports and compatibility adapters.
pub fn extract_http_status(message: &str) -> Option<u16> {
    for pattern in &["HTTP ", "status: ", "status code: ", "API error: "] {
        let Some(position) = message.find(pattern) else {
            continue;
        };
        let start = position + pattern.len();
        let code: String = message[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(status @ 100..=599) = code.parse() {
            return Some(status);
        }
    }
    None
}

/// Conservative classification for errors delivered inside an already-open
/// provider stream, where only strings remain available.
pub fn is_retryable_error_message(message: &str) -> bool {
    if let Some(status) = extract_http_status(message) {
        return is_retryable_status(status);
    }

    let message = message.to_ascii_lowercase();
    [
        "stream ended without a finish signal",
        "timed out",
        "timeout",
        "connection reset",
        "connection closed",
        "websocket closed before completion",
        "websocket ended before response completion",
        "network error",
        "temporarily at capacity",
        "temporarily unavailable",
        "resource has been exhausted",
        "resource exhausted",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

/// Parse either form allowed by the HTTP `Retry-After` header.
pub(crate) fn parse_retry_after(header_value: &str) -> Option<Duration> {
    if let Ok(seconds) = header_value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let date = httpdate::parse_http_date(header_value).ok()?;
    date.duration_since(std::time::SystemTime::now()).ok()
}

/// Execute an async operation with retry logic
///
/// Uses exponential backoff with optional jitter. Respects Retry-After headers
/// when provided by the server.
pub async fn with_retry<F, Fut, T, E>(config: &RetryConfig, operation: F) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: IsRetryable + std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if e.is_retryable() && attempt < config.max_retries => {
                // Check for Retry-After header
                // A provider-controlled Retry-After value must not suspend an
                // interactive turn beyond the caller's explicit retry budget.
                let wait = e.retry_after().unwrap_or(delay).min(config.max_delay);

                // Add jitter to prevent thundering herd
                let jittered = if config.jitter {
                    let jitter_ms = rand::thread_rng().gen_range(0..1000);
                    (wait + Duration::from_millis(jitter_ms)).min(config.max_delay)
                } else {
                    wait
                };

                warn!(
                    attempt = attempt + 1,
                    max_retries = config.max_retries,
                    delay_ms = jittered.as_millis() as u64,
                    "Retrying after error: {}",
                    e
                );

                tokio::time::sleep(jittered).await;
                attempt += 1;
                delay = (delay * 2).min(config.max_delay);
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    async fn serve_error_response(
        body: Vec<u8>,
        extra_headers: &'static str,
    ) -> (reqwest::Response, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have an address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("test request should connect");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            while request.len() < 8 * 1024 {
                let count = socket
                    .read(&mut chunk)
                    .await
                    .expect("test request should be readable");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            let response_headers = format!(
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nContent-Type: application/json\r\nRetry-After: 7\r\n{}Connection: close\r\n\r\n",
                body.len(),
                extra_headers
            );
            if socket.write_all(response_headers.as_bytes()).await.is_ok() {
                let _ = socket.write_all(&body).await;
            }
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/error"))
            .send()
            .await
            .expect("test response should be received");
        (response, server)
    }

    async fn serve_slow_error_response() -> (reqwest::Response, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have an address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("test request should connect");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            while request.len() < 8 * 1024 {
                let count = socket
                    .read(&mut chunk)
                    .await
                    .expect("test request should be readable");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = socket
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
                )
                .await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = socket.write_all(b"x").await;
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/slow-error"))
            .send()
            .await
            .expect("test response should be received");
        (response, server)
    }

    #[derive(Debug)]
    struct TestError {
        retryable: bool,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test error")
        }
    }

    impl IsRetryable for TestError {
        fn is_retryable(&self) -> bool {
            self.retryable
        }

        fn retry_after(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn test_retryable_status_codes() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(is_retryable_status(529));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(404));
    }

    #[test]
    fn retryable_error_messages_do_not_classify_payment_or_auth_failures() {
        assert!(is_retryable_error_message(
            "API error: 429 Too Many Requests - capacity"
        ));
        assert!(is_retryable_error_message(
            "AI stream ended without a finish signal"
        ));
        assert!(is_retryable_error_message(
            "Codex websocket closed before completion"
        ));
        assert!(is_retryable_error_message(
            "Sub-agent websocket ended before response completion"
        ));
        assert!(!is_retryable_error_message(
            "API error: 402 Payment Required - limit reached"
        ));
        assert!(!is_retryable_error_message(
            "API error: 403 Forbidden - invalid credentials"
        ));
    }

    #[test]
    fn test_parse_retry_after_seconds() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
    }

    #[test]
    fn test_default_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert!(config.jitter);
    }

    #[test]
    fn provider_http_error_preserves_retry_contract() {
        const SENTINEL: &str = "temporarily at capacity";
        let capacity = ProviderHttpError::new(
            "API error",
            429,
            "429 Too Many Requests",
            SENTINEL,
            Some(Duration::from_secs(3)),
        );
        assert!(capacity.is_retryable());
        assert_eq!(capacity.retry_after(), Some(Duration::from_secs(3)));
        let rendered = capacity.to_string();
        assert!(rendered.starts_with("API error: 429 Too Many Requests ["));
        assert!(rendered.contains("response_fingerprint=sha256:"));
        assert!(!rendered.contains(SENTINEL));
        assert!(!format!("{capacity:?}").contains(SENTINEL));

        let payment = ProviderHttpError::new(
            "API error",
            402,
            "402 Payment Required",
            "limit reached",
            None,
        );
        assert!(!payment.is_retryable());
    }

    #[tokio::test]
    async fn provider_error_body_read_is_bounded_and_secret_safe() {
        const BODY_SENTINEL: &str = "BODY_SENTINEL_91af3_DO_NOT_LOG";
        const HEADER_REQUEST_ID_SENTINEL: &str = "HEADER_SENTINEL_5bc29_DO_NOT_LOG";
        let mut body = BODY_SENTINEL.as_bytes().to_vec();
        body.resize(MAX_PROVIDER_ERROR_BODY_BYTES * 2, b'x');
        let (response, server) =
            serve_error_response(body, "X-Request-ID: HEADER_SENTINEL_5bc29_DO_NOT_LOG\r\n").await;

        let error = provider_http_error(response, "API error").await;
        server.await.expect("test server should finish");

        assert_eq!(error.status(), 503);
        assert_eq!(error.retry_after_value(), Some(Duration::from_secs(7)));
        assert!(error.request_id_fingerprint.is_some());
        assert_eq!(error.captured_body_bytes, MAX_PROVIDER_ERROR_BODY_BYTES);
        assert!(error.body_truncated);
        assert!(!error.body_read_failed);
        assert!(error.response_fingerprint.starts_with("sha256:"));
        assert_eq!(error.response_fingerprint.len(), 71);

        let display = error.to_string();
        let debug = format!("{error:?}");
        for sentinel in [BODY_SENTINEL, HEADER_REQUEST_ID_SENTINEL] {
            assert!(!display.contains(sentinel));
            assert!(!debug.contains(sentinel));
        }
        assert!(display.len() < 320);
        assert!(debug.len() < 640);
    }

    #[tokio::test]
    async fn provider_error_never_reflects_unknown_code_or_body_request_id() {
        const MESSAGE_SENTINEL: &str = "MESSAGE_SENTINEL_78e12_DO_NOT_LOG";
        const CODE_SENTINEL: &str = "CODE_SENTINEL_a31d9_DO_NOT_LOG";
        const BODY_REQUEST_ID_SENTINEL: &str = "BODY_REQUEST_SENTINEL_02f6c_DO_NOT_LOG";
        let body = format!(
            r#"{{"error":{{"code":"{CODE_SENTINEL}","message":"{MESSAGE_SENTINEL}"}},"request_id":"{BODY_REQUEST_ID_SENTINEL}"}}"#
        )
        .into_bytes();
        let (response, server) = serve_error_response(body, "").await;

        let error = provider_http_error(response, "API error").await;
        server.await.expect("test server should finish");

        assert!(error.provider_code.is_none());
        assert!(error.provider_code_fingerprint.is_some());
        assert!(error.request_id_fingerprint.is_some());
        assert!(!error.body_truncated);
        assert!(!error.body_read_failed);
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(display.contains("code_fingerprint=sha256:"));
        assert!(display.contains("request_id_fingerprint=sha256:"));
        for sentinel in [MESSAGE_SENTINEL, CODE_SENTINEL, BODY_REQUEST_ID_SENTINEL] {
            assert!(!display.contains(sentinel));
            assert!(!debug.contains(sentinel));
        }
    }

    #[tokio::test]
    async fn provider_error_body_read_has_a_short_total_deadline() {
        let (response, server) = serve_slow_error_response().await;
        let error =
            provider_http_error_with_timeout(response, "API error", Duration::from_millis(10))
                .await;
        server.await.expect("test server should finish");

        assert!(error.body_read_timed_out);
        assert!(!error.body_read_failed);
        assert!(error.to_string().contains("body_read_timed_out=true"));
    }

    #[test]
    fn provider_event_error_never_reflects_unknown_metadata() {
        const CODE_SENTINEL: &str = "EVENT_CODE_SENTINEL_32d1";
        const CATEGORY_SENTINEL: &str = "EVENT_CATEGORY_SENTINEL_c110";
        const MESSAGE_SENTINEL: &str = "EVENT_MESSAGE_SENTINEL_91ab";
        let error = safe_provider_event_error(
            "Provider stream failed",
            Some(CODE_SENTINEL),
            Some(CATEGORY_SENTINEL),
            Some(MESSAGE_SENTINEL),
        );
        for sentinel in [CODE_SENTINEL, CATEGORY_SENTINEL, MESSAGE_SENTINEL] {
            assert!(!error.contains(sentinel));
        }
        assert!(error.contains("code_fingerprint=sha256:"));
        assert!(error.contains("category_fingerprint=sha256:"));
        assert!(error.contains("message_fingerprint=sha256:"));
    }

    #[test]
    fn provider_error_codes_are_allowlisted_and_other_metadata_is_hashed() {
        assert_eq!(
            known_provider_code("RATE_LIMIT_EXCEEDED").as_deref(),
            Some("rate_limit_exceeded")
        );
        assert!(known_provider_code("CODE_SENTINEL_a31d9_DO_NOT_LOG").is_none());
        assert_eq!(
            safe_provider_code("RATE_LIMIT_EXCEEDED"),
            "rate_limit_exceeded"
        );
        let safe_unknown = safe_provider_code("CODE_SENTINEL_a31d9_DO_NOT_LOG");
        assert!(safe_unknown.starts_with("unknown:sha256:"));
        assert!(!safe_unknown.contains("CODE_SENTINEL_a31d9_DO_NOT_LOG"));
        let first = metadata_fingerprint(b"request-id", b"arbitrary-value");
        let repeated = metadata_fingerprint(b"request-id", b"arbitrary-value");
        let other_domain = metadata_fingerprint(b"provider-code", b"arbitrary-value");
        assert_eq!(first, repeated);
        assert_ne!(first, other_domain);
        assert_eq!(first.len(), 71);
        assert!(!first.contains("arbitrary-value"));
    }

    #[tokio::test]
    async fn retryable_failures_are_retried_until_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = RetryConfig {
            max_retries: 3,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            jitter: false,
        };

        let result = with_retry(&config, || {
            let attempts = Arc::clone(&attempts);
            async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(TestError { retryable: true })
                } else {
                    Ok("ok")
                }
            }
        })
        .await;

        assert_eq!(result.expect("third attempt should succeed"), "ok");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn terminal_failures_are_not_retried() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = RetryConfig {
            max_retries: 3,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            jitter: false,
        };

        let result: Result<(), TestError> = with_retry(&config, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(TestError { retryable: false }) }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_exhaustion_has_an_exact_attempt_bound() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = RetryConfig {
            max_retries: 2,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            jitter: false,
        };

        let result: Result<(), TestError> = with_retry(&config, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(TestError { retryable: true }) }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
