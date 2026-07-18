//! Rate limiting and retry logic
//!
//! Provides exponential backoff with jitter for handling API rate limits and transient errors.
//!
//! Used by primary streaming setup and subagent API calls to handle transient
//! rate limits and provider server errors before caller-visible work begins.

mod backoff;

pub use backoff::{
    extract_http_status, is_retryable_error_message, is_retryable_status, with_retry, IsRetryable,
    RetryConfig,
};
pub(crate) use backoff::{
    provider_http_error, safe_provider_code, safe_provider_event_error, ProviderHttpError,
};
