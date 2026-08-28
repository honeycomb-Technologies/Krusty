//! Operational streaming policy resolved from the wire transport.
//!
//! Prompt style and transport behavior are separate concerns. A model-family
//! prompt overlay must not silently alter queue limits or network timeouts.

use std::time::Duration;

use reqwest::{redirect::Policy, Client, ClientBuilder};

use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;

/// Start every provider HTTP client from fail-closed replay policies.
///
/// Application-level retry policy remains owned by the caller. Redirects are
/// transport-level replays, and reqwest otherwise retries protocol NACKs by
/// default; neither may create an uncounted provider request.
pub(crate) fn provider_http_client_builder() -> ClientBuilder {
    Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
}

/// Build the minimal provider HTTP client used by catalog fetchers and as the
/// fallback when the fully configured streaming client cannot be constructed.
pub(crate) fn build_provider_http_client() -> reqwest::Result<Client> {
    provider_http_client_builder().build()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDrainPolicy {
    pub smooth_batch_limit: usize,
    pub moderate_batch_limit: usize,
    pub catch_up_batch_limit: usize,
    pub moderate_backlog_threshold: usize,
    pub catch_up_backlog_threshold: usize,
    pub moderate_backlog_age: Duration,
    pub catch_up_backlog_age: Duration,
    pub hard_queue_limit: usize,
}

impl Default for StreamDrainPolicy {
    fn default() -> Self {
        Self {
            smooth_batch_limit: 12,
            moderate_batch_limit: 32,
            catch_up_batch_limit: 96,
            moderate_backlog_threshold: 24,
            catch_up_backlog_threshold: 80,
            moderate_backlog_age: Duration::from_millis(40),
            catch_up_backlog_age: Duration::from_millis(120),
            hard_queue_limit: 384,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamTransportPolicy {
    pub idle_timeout: Duration,
    pub drain: StreamDrainPolicy,
}

impl StreamTransportPolicy {
    pub fn resolve(provider: ProviderId, api_format: ApiFormat) -> Self {
        if matches!(api_format, ApiFormat::OpenAIResponses) {
            return Self {
                idle_timeout: Duration::from_secs(1_200),
                drain: StreamDrainPolicy {
                    smooth_batch_limit: 18,
                    moderate_batch_limit: 48,
                    catch_up_batch_limit: 128,
                    moderate_backlog_threshold: 28,
                    catch_up_backlog_threshold: 96,
                    moderate_backlog_age: Duration::from_millis(35),
                    catch_up_backlog_age: Duration::from_millis(100),
                    hard_queue_limit: 512,
                },
            };
        }

        if matches!(api_format, ApiFormat::Anthropic | ApiFormat::Google) {
            return Self {
                idle_timeout: Duration::from_secs(900),
                drain: StreamDrainPolicy {
                    smooth_batch_limit: 12,
                    moderate_batch_limit: 28,
                    catch_up_batch_limit: 80,
                    moderate_backlog_threshold: 20,
                    catch_up_backlog_threshold: 72,
                    moderate_backlog_age: Duration::from_millis(40),
                    catch_up_backlog_age: Duration::from_millis(120),
                    hard_queue_limit: 384,
                },
            };
        }

        Self {
            idle_timeout: Duration::from_secs(if provider == ProviderId::Grok {
                1_200
            } else {
                900
            }),
            drain: StreamDrainPolicy::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_transport_not_model_name_controls_stream_policy() {
        let first = StreamTransportPolicy::resolve(ProviderId::OpenAI, ApiFormat::OpenAIResponses);
        let second = StreamTransportPolicy::resolve(ProviderId::Grok, ApiFormat::OpenAIResponses);
        let anthropic = StreamTransportPolicy::resolve(ProviderId::Anthropic, ApiFormat::Anthropic);

        assert_eq!(first, second);
        assert!(first.drain.catch_up_batch_limit > anthropic.drain.catch_up_batch_limit);
        assert!(first.idle_timeout > anthropic.idle_timeout);
    }
}
