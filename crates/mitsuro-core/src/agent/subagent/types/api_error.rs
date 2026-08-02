use std::time::Duration;

use crate::ai::retry::{extract_http_status, is_retryable_status, IsRetryable, ProviderHttpError};

/// Error type for subagent API calls that supports retry logic.
#[derive(Debug)]
pub struct SubAgentApiError {
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for SubAgentApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self.status {
            write!(f, "HTTP {}: {}", status, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for SubAgentApiError {}

impl IsRetryable for SubAgentApiError {
    fn is_retryable(&self) -> bool {
        match self.status {
            Some(status) => is_retryable_status(status),
            None => {
                self.message.contains("timeout")
                    || self.message.contains("connection")
                    || self.message.contains("network")
            }
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

impl From<anyhow::Error> for SubAgentApiError {
    fn from(err: anyhow::Error) -> Self {
        let typed_http = err.downcast_ref::<ProviderHttpError>();
        let message = err.to_string();
        let status = typed_http
            .map(ProviderHttpError::status)
            .or_else(|| extract_status_from_error(&message));
        Self {
            message,
            status,
            retry_after: typed_http.and_then(ProviderHttpError::retry_after_value),
        }
    }
}

/// Try to extract an HTTP status code from a provider error message.
pub fn extract_status_from_error(message: &str) -> Option<u16> {
    extract_http_status(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_standard_provider_error_status() {
        assert_eq!(
            extract_status_from_error("API error: 429 Too Many Requests - capacity"),
            Some(429)
        );
        assert_eq!(
            extract_status_from_error("API error: 402 Payment Required - limit"),
            Some(402)
        );
    }

    #[test]
    fn typed_provider_error_preserves_retry_after() {
        let error = anyhow::Error::new(ProviderHttpError::new(
            "API error",
            429,
            "429 Too Many Requests",
            "capacity",
            Some(Duration::from_secs(2)),
        ));

        let error = SubAgentApiError::from(error);
        assert_eq!(error.status, Some(429));
        assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
        assert!(error.is_retryable());
    }
}
