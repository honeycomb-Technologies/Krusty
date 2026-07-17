//! Streaming API calls
//!
//! Handles SSE streaming responses from different providers.

mod anthropic;
pub(crate) mod codex;
mod google;
mod openai;
mod request_options;
mod shared;

use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::info;

use super::config::CallOptions;
use super::core::AiClient;
use crate::ai::retry::{with_retry, RetryConfig};
use crate::ai::streaming::StreamPart;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Call the API with streaming response
    pub async fn call_streaming(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let canonical_options = self.canonical_call_options(&self.config().model, options);
        let retry_config = RetryConfig::interactive_stream();

        // A receiver is returned only after the provider accepts the request,
        // so typed transient HTTP failures and definite connect failures can be
        // retried here without duplicating visible deltas or local tool work.
        with_retry(&retry_config, || {
            self.call_streaming_once(messages.clone(), &canonical_options)
        })
        .await
    }

    async fn call_streaming_once(
        &self,
        messages: Vec<ModelMessage>,
        canonical_options: &CallOptions,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let call_start = Instant::now();
        info!("=== API CALL START ===");
        info!(
            "Model: {}, Messages: {}, Tools: {}, Thinking: {}, Format: {:?}",
            self.config().model,
            messages.len(),
            canonical_options
                .tools
                .as_ref()
                .map(|t| t.len())
                .unwrap_or(0),
            canonical_options.thinking.is_some(),
            self.config().api_format
        );

        if self.config().uses_openai_format() {
            return self
                .call_streaming_openai(messages, canonical_options, call_start)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_streaming_google(messages, canonical_options, call_start)
                .await;
        }

        self.call_streaming_anthropic(messages, canonical_options, call_start)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc as std_mpsc;
    use std::thread;
    use std::time::Duration;

    use tiny_http::{Header, Response, Server};

    use super::*;
    use crate::ai::client::AiClientConfig;
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::{AuthHeader, ProviderId};
    use crate::ai::types::{Content, Role};

    fn openai_test_client(url: String) -> AiClient {
        AiClient::new(
            AiClientConfig {
                model: "test-model".to_string(),
                max_tokens: 128,
                base_url: Some(url),
                auth_header: AuthHeader::Bearer,
                provider_id: ProviderId::OpenAI,
                api_format: ApiFormat::OpenAI,
                custom_headers: Default::default(),
            },
            "test-key".to_string(),
        )
    }

    fn user_message() -> ModelMessage {
        ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "reply with ok".to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn transient_http_failure_retries_before_exposing_stream() {
        let server = Server::http("127.0.0.1:0").expect("test server should bind");
        let url = format!("http://{}", server.server_addr());
        let (body_tx, body_rx) = std_mpsc::channel();
        let server_thread = thread::spawn(move || {
            for attempt in 0..2 {
                let mut request = server.recv().expect("request should arrive");
                let mut body = String::new();
                request
                    .as_reader()
                    .read_to_string(&mut body)
                    .expect("request body should be readable");
                body_tx.send(body).expect("body should be recorded");

                if attempt == 0 {
                    let response = Response::from_string("temporarily at capacity")
                        .with_status_code(429)
                        .with_header(
                            Header::from_bytes("Retry-After", "0")
                                .expect("retry header should be valid"),
                        );
                    request.respond(response).expect("429 should be sent");
                } else {
                    let response = Response::from_string(
                        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n\
                         data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                         data: [DONE]\n\n",
                    )
                    .with_header(
                        Header::from_bytes("Content-Type", "text/event-stream")
                            .expect("content type should be valid"),
                    );
                    request.respond(response).expect("SSE should be sent");
                }
            }
        });

        let client = openai_test_client(url);
        let mut stream = client
            .call_streaming(vec![user_message()], &CallOptions::default())
            .await
            .expect("second request should open a stream");

        let mut text = String::new();
        let mut finished = false;
        while let Some(part) = tokio::time::timeout(Duration::from_secs(5), stream.recv())
            .await
            .expect("stream should not stall")
        {
            match part {
                StreamPart::TextDelta { delta } => text.push_str(&delta),
                StreamPart::Finish { .. } => {
                    finished = true;
                    break;
                }
                StreamPart::Error { error } => panic!("unexpected stream error: {error}"),
                _ => {}
            }
        }

        server_thread.join().expect("server thread should finish");
        let first_body = body_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first body should be recorded");
        let second_body = body_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second body should be recorded");
        assert_eq!(first_body, second_body);
        assert_eq!(text, "ok");
        assert!(finished);
    }

    #[tokio::test]
    async fn payment_failure_is_terminal_without_retry() {
        let server = Server::http("127.0.0.1:0").expect("test server should bind");
        let url = format!("http://{}", server.server_addr());
        let (request_tx, request_rx) = std_mpsc::channel();
        let server_thread = thread::spawn(move || {
            let request = server.recv().expect("request should arrive");
            request_tx.send(()).expect("request should be counted");
            request
                .respond(Response::from_string("limit reached").with_status_code(402))
                .expect("402 should be sent");
        });

        let client = openai_test_client(url);
        let error = client
            .call_streaming(vec![user_message()], &CallOptions::default())
            .await
            .expect_err("payment failure must be terminal");

        server_thread.join().expect("server thread should finish");
        request_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("one request should be recorded");
        assert!(request_rx.try_recv().is_err());
        assert!(error.to_string().contains("402 Payment Required"));
    }
}
