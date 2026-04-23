use std::time::Duration;

use crate::ai::retry::is_retryable_status;
use crate::ai::retry::IsRetryable;

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
        let message = err.to_string();
        let status = extract_status_from_error(&message);
        Self {
            message,
            status,
            retry_after: None,
        }
    }
}

/// Try to extract an HTTP status code from a provider error message.
pub fn extract_status_from_error(message: &str) -> Option<u16> {
    for pattern in &["HTTP ", "status: ", "status code: "] {
        if let Some(pos) = message.find(pattern) {
            let start = pos + pattern.len();
            let code_str: String = message[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(code) = code_str.parse() {
                return Some(code);
            }
        }
    }
    None
}
