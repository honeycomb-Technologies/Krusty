//! Exponential backoff with jitter
//!
//! Implements retry logic for transient API errors including rate limiting (429).

use std::future::Future;
use std::time::Duration;

use rand::Rng;
use tracing::warn;

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
}

/// Trait for errors that may be retryable
pub trait IsRetryable {
    /// Check if this error is retryable
    fn is_retryable(&self) -> bool;

    /// Get the retry-after duration if specified by the server
    fn retry_after(&self) -> Option<Duration>;
}

/// HTTP status codes that should trigger retry
pub const RETRYABLE_STATUS_CODES: &[u16] = &[
    429, // Too Many Requests
    500, // Internal Server Error
    502, // Bad Gateway
    503, // Service Unavailable
    504, // Gateway Timeout
];

/// Check if an HTTP status code is retryable
pub fn is_retryable_status(status: u16) -> bool {
    RETRYABLE_STATUS_CODES.contains(&status)
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
                let wait = e.retry_after().unwrap_or(delay);

                // Add jitter to prevent thundering herd
                let jittered = if config.jitter {
                    let jitter_ms = rand::thread_rng().gen_range(0..1000);
                    wait + Duration::from_millis(jitter_ms)
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

/// Parse Retry-After header value
///
/// The header can be either:
/// - A number of seconds (e.g., "120")
/// - An HTTP date (e.g., "Wed, 21 Oct 2015 07:28:00 GMT")
pub fn parse_retry_after(header_value: &str) -> Option<Duration> {
    // Try parsing as seconds first
    if let Ok(seconds) = header_value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    // Try parsing as HTTP date
    if let Ok(date) = httpdate::parse_http_date(header_value) {
        let now = std::time::SystemTime::now();
        if let Ok(duration) = date.duration_since(now) {
            return Some(duration);
        }
    }

    None
}

/// A simple retryable error wrapper for HTTP errors
#[derive(Debug)]
pub struct HttpError {
    pub status: u16,
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)
    }
}

impl std::error::Error for HttpError {}

impl IsRetryable for HttpError {
    fn is_retryable(&self) -> bool {
        is_retryable_status(self.status)
    }

    fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retryable_status_codes() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(404));
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
}
