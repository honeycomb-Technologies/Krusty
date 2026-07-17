//! Exponential backoff with jitter
//!
//! Implements retry logic for transient API errors including rate limiting (429).

use std::fmt;
use std::future::Future;
use std::time::Duration;

use rand::Rng;
use tracing::warn;

/// Structured provider HTTP failure retained across the transport boundary.
///
/// Keeping the status and `Retry-After` value typed lets the shared streaming
/// caller retry transient setup failures without parsing user-facing strings.
#[derive(Debug)]
pub(crate) struct ProviderHttpError {
    label: String,
    status: u16,
    status_text: String,
    body: String,
    retry_after: Option<Duration>,
}

impl ProviderHttpError {
    pub(crate) fn new(
        label: impl Into<String>,
        status: u16,
        status_text: impl Into<String>,
        body: impl Into<String>,
        retry_after: Option<Duration>,
    ) -> Self {
        Self {
            label: label.into(),
            status,
            status_text: status_text.into(),
            body: body.into(),
            retry_after,
        }
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    pub(crate) fn retry_after_value(&self) -> Option<Duration> {
        self.retry_after
    }
}

impl fmt::Display for ProviderHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {} - {}",
            self.label, self.status_text, self.body
        )
    }
}

impl std::error::Error for ProviderHttpError {}

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
/// Krusty's transports and compatibility adapters.
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
        let capacity = ProviderHttpError::new(
            "API error",
            429,
            "429 Too Many Requests",
            "temporarily at capacity",
            Some(Duration::from_secs(3)),
        );
        assert!(capacity.is_retryable());
        assert_eq!(capacity.retry_after(), Some(Duration::from_secs(3)));
        assert_eq!(
            capacity.to_string(),
            "API error: 429 Too Many Requests - temporarily at capacity"
        );

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
