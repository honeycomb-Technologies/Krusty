use anyhow::Result;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use url::Url;

use super::super::super::config::CallOptions;
use super::super::super::core::AiClient;
use super::super::shared::{ensure_success_stream_response, log_request_metrics, start_sse_stream};
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::model_profile::SystemPromptSections;
use crate::ai::parsers::OpenAIParser;
use crate::ai::sse::{create_streaming_channels, SseStreamProcessor};
use crate::ai::streaming::StreamPart;
use crate::ai::types::ModelMessage;

use super::request::{assistant_fingerprint_from_response, prepare_codex_ws_request};
use super::session::{CodexContinuation, CodexSessionGuard, CodexWebSocket};

enum CodexPayloadState {
    Continue,
    Complete {
        response_id: Option<String>,
        assistant_fingerprint: Option<String>,
    },
    Error,
}

fn codex_cache_stable_instructions(prompt_sections: &SystemPromptSections) -> String {
    let mut sections = Vec::new();

    if !prompt_sections.base_prompt.is_empty() {
        sections.push(prompt_sections.base_prompt.as_str());
    }
    if !prompt_sections.project_context.is_empty() {
        sections.push(prompt_sections.project_context.as_str());
    }

    sections.join("\n\n---\n\n")
}

async fn process_codex_ws_payload(
    payload: &str,
    parser: &OpenAIParser,
    processor: &mut SseStreamProcessor,
    tx_err: &mpsc::UnboundedSender<StreamPart>,
) -> CodexPayloadState {
    if let Ok(json) = serde_json::from_str::<Value>(payload) {
        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if matches!(event_type, "error" | "response.failed") || event_type.contains("error") {
            let detail = AiClient::codex_ws_error_message(&json)
                .unwrap_or_else(|| "unknown websocket error".to_string());
            let _ = tx_err.send(StreamPart::Error {
                error: format!("Codex websocket API error: {}", detail),
            });
            return CodexPayloadState::Error;
        }
        if matches!(event_type, "response.done" | "response.completed") {
            if let Err(e) = processor.process_sse_data(payload, parser).await {
                let _ = tx_err.send(StreamPart::Error {
                    error: format!("Codex websocket parsing error: {}", e),
                });
                return CodexPayloadState::Error;
            }
            let response = json.get("response").unwrap_or(&json);
            return CodexPayloadState::Complete {
                response_id: response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                assistant_fingerprint: assistant_fingerprint_from_response(response),
            };
        }
    }

    if let Err(e) = processor.process_sse_data(payload, parser).await {
        let _ = tx_err.send(StreamPart::Error {
            error: format!("Codex websocket parsing error: {}", e),
        });
        return CodexPayloadState::Error;
    }

    CodexPayloadState::Continue
}

fn codex_ws_error_code(payload: &str) -> Option<String> {
    let json = serde_json::from_str::<Value>(payload).ok()?;
    json.pointer("/error/code")
        .or_else(|| json.pointer("/response/error/code"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(super) async fn call_streaming_chatgpt_codex_ws(
    client: &AiClient,
    messages: Vec<ModelMessage>,
    options: &CallOptions,
    call_start: Instant,
) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
    let format_handler = OpenAIFormat::new(client.config().api_format);
    let prompt_sections = client.system_prompt_sections(
        &client.config().model,
        &messages,
        options.system_prompt.as_deref(),
        options.tools.as_deref(),
    );
    let system_prompt = codex_cache_stable_instructions(&prompt_sections);
    let volatile_context = (!prompt_sections.session_context.trim().is_empty())
        .then_some(prompt_sections.session_context.as_str());

    let max_tokens = options.max_tokens.unwrap_or(client.config().max_tokens);
    let body = client.build_chatgpt_codex_body(
        &messages,
        &system_prompt,
        volatile_context,
        max_tokens,
        options,
        &format_handler,
    );

    let ws_url = resolve_codex_ws_url(&client.config().api_url())?;
    let session_key = options
        .session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty())
        .map(|session_id| {
            format!(
                "{}:{}:{}",
                client.provider_id(),
                client.config().model,
                session_id
            )
        });
    let mut state = if let Some(session_key) = session_key.as_deref() {
        client
            .codex_ws_pool
            .session(session_key)
            .await
            .lock_owned()
            .await
    } else {
        CodexSessionGuard::ephemeral().await
    };

    if !state.can_reuse_connection() {
        state.reset();
    }

    let prepared = prepare_codex_ws_request(
        body,
        &messages,
        volatile_context,
        state.continuation.as_ref(),
    );

    if state.connection.is_none() {
        match connect_codex_websocket(client, &ws_url, options).await {
            Ok(connection) => {
                state.connection = Some(connection);
                state.connected_at = Some(Instant::now());
                info!(
                    "ChatGPT Codex websocket connected in {:?}",
                    call_start.elapsed()
                );
            }
            Err(e) => {
                warn!(
                    "ChatGPT Codex websocket connect failed ({}), falling back to HTTP streaming",
                    e
                );
                let full_body = prepared.full_body;
                drop(state);
                return call_streaming_chatgpt_codex_http(client, full_body, call_start).await;
            }
        }
    }

    let mut create_payload = AiClient::codex_ws_create_payload(prepared.websocket_body.clone());
    let mut sent_delta = prepared.used_continuation;
    let send_result = state
        .connection
        .as_mut()
        .expect("Codex connection initialized")
        .send(Message::Text(create_payload.to_string()))
        .await;

    if let Err(error) = send_result {
        warn!(
            "ChatGPT Codex websocket send failed ({}); reconnecting with full context",
            error
        );
        state.reset();
        match connect_codex_websocket(client, &ws_url, options).await {
            Ok(mut connection) => {
                create_payload = AiClient::codex_ws_create_payload(prepared.full_body.clone());
                if let Err(retry_error) = connection
                    .send(Message::Text(create_payload.to_string()))
                    .await
                {
                    warn!(
                        "ChatGPT Codex websocket retry failed ({}), falling back to HTTP streaming",
                        retry_error
                    );
                    let full_body = prepared.full_body;
                    drop(state);
                    return call_streaming_chatgpt_codex_http(client, full_body, call_start).await;
                }
                state.connection = Some(connection);
                state.connected_at = Some(Instant::now());
                sent_delta = false;
            }
            Err(reconnect_error) => {
                warn!(
                    "ChatGPT Codex websocket reconnect failed ({}), falling back to HTTP streaming",
                    reconnect_error
                );
                let full_body = prepared.full_body;
                drop(state);
                return call_streaming_chatgpt_codex_http(client, full_body, call_start).await;
            }
        }
    }

    info!(
        transport = "websocket",
        request_mode = if sent_delta { "delta" } else { "full" },
        request_fingerprint = %prepared.request_fingerprint,
        "ChatGPT Codex request sent"
    );
    let measured_body = if sent_delta {
        &prepared.websocket_body
    } else {
        &prepared.full_body
    };
    log_request_metrics(
        "codex_stream",
        &prompt_sections,
        &messages,
        options.tools.as_deref(),
        options.system_prompt.is_some(),
        if sent_delta {
            "websocket_delta"
        } else {
            "websocket_full"
        },
        options
            .session_id
            .as_deref()
            .is_some_and(|session_id| !session_id.is_empty()),
        serde_json::to_vec(measured_body).map_or(0, |value| value.len()),
    );

    let (tx, rx) = create_streaming_channels();
    let tx_err = tx.clone();

    let mut processor = SseStreamProcessor::new(tx).with_transform_context(
        client.provider_id(),
        client.config().api_format,
        client.config().model.clone(),
    );
    let parser = OpenAIParser::new();
    let ws_idle_timeout =
        Duration::from_secs(prompt_sections.profile.stream_idle_timeout_secs.max(30));

    tokio::spawn(async move {
        let mut retried_full = !sent_delta;
        let mut completed = false;

        loop {
            let next = tokio::select! {
                _ = tx_err.closed() => break,
                next = tokio::time::timeout(
                    ws_idle_timeout,
                    state
                        .connection
                        .as_mut()
                        .expect("Codex connection initialized")
                        .next(),
                ) => next,
            };
            let msg = match next {
                Ok(Some(message)) => message,
                Ok(None) => {
                    let _ = tx_err.send(StreamPart::Error {
                        error: "Codex websocket ended before response completion".to_string(),
                    });
                    break;
                }
                Err(_) => {
                    let _ = tx_err.send(StreamPart::Error {
                        error: format!(
                            "Codex websocket produced no events for {} seconds",
                            ws_idle_timeout.as_secs()
                        ),
                    });
                    break;
                }
            };

            match msg {
                Ok(Message::Text(text)) => {
                    let payload = text.to_string();
                    if sent_delta
                        && !retried_full
                        && codex_ws_error_code(&payload).as_deref()
                            == Some("previous_response_not_found")
                    {
                        retried_full = true;
                        state.continuation = None;
                        let full_payload =
                            AiClient::codex_ws_create_payload(prepared.full_body.clone());
                        let retry = state
                            .connection
                            .as_mut()
                            .expect("Codex connection initialized")
                            .send(Message::Text(full_payload.to_string()))
                            .await;
                        if let Err(error) = retry {
                            let _ = tx_err.send(StreamPart::Error {
                                error: format!(
                                    "Codex websocket full-context retry failed: {}",
                                    error
                                ),
                            });
                            break;
                        }
                        continue;
                    }
                    match process_codex_ws_payload(&payload, &parser, &mut processor, &tx_err).await
                    {
                        CodexPayloadState::Continue => {}
                        CodexPayloadState::Complete {
                            response_id,
                            assistant_fingerprint,
                        } => {
                            completed = true;
                            state.continuation = response_id.map(|response_id| CodexContinuation {
                                response_id,
                                request_fingerprint: prepared.request_fingerprint.clone(),
                                message_fingerprints: prepared.message_fingerprints.clone(),
                                assistant_fingerprint,
                                volatile_context_fingerprint: prepared
                                    .volatile_context_fingerprint
                                    .clone(),
                            });
                            break;
                        }
                        CodexPayloadState::Error => break,
                    }
                }
                Ok(Message::Binary(bytes)) => {
                    let payload = String::from_utf8_lossy(&bytes);
                    match process_codex_ws_payload(
                        payload.as_ref(),
                        &parser,
                        &mut processor,
                        &tx_err,
                    )
                    .await
                    {
                        CodexPayloadState::Continue => {}
                        CodexPayloadState::Complete {
                            response_id,
                            assistant_fingerprint,
                        } => {
                            completed = true;
                            state.continuation = response_id.map(|response_id| CodexContinuation {
                                response_id,
                                request_fingerprint: prepared.request_fingerprint.clone(),
                                message_fingerprints: prepared.message_fingerprints.clone(),
                                assistant_fingerprint,
                                volatile_context_fingerprint: prepared
                                    .volatile_context_fingerprint
                                    .clone(),
                            });
                            break;
                        }
                        CodexPayloadState::Error => break,
                    }
                }
                Ok(Message::Close(frame)) => {
                    let (code, reason) = frame
                        .as_ref()
                        .map(|f| (f.code.to_string(), f.reason.to_string()))
                        .unwrap_or_else(|| {
                            ("no close code".to_string(), "no close reason".to_string())
                        });
                    let _ = tx_err.send(StreamPart::Error {
                        error: format!(
                            "Codex websocket closed before completion (websocket-only mode): code={}, reason={}",
                            code, reason
                        ),
                    });
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Frame(_)) => {}
                Err(e) => {
                    let _ = tx_err.send(StreamPart::Error {
                        error: format!("Codex websocket stream error: {}", e),
                    });
                    break;
                }
            }
        }

        if !completed {
            state.reset();
        }
        processor.finish().await;
    });

    Ok(rx)
}

async fn connect_codex_websocket(
    client: &AiClient,
    ws_url: &Url,
    options: &CallOptions,
) -> Result<CodexWebSocket> {
    let mut request = client.build_websocket_request(
        ws_url.as_str(),
        &[
            ("OpenAI-Beta", "responses_websockets=2026-02-06"),
            ("originator", "krusty"),
        ],
    )?;
    if let Some(session_id) = &options.session_id {
        match session_id.parse::<tokio_tungstenite::tungstenite::http::HeaderValue>() {
            Ok(value) => {
                request.headers_mut().insert("session_id", value);
            }
            Err(error) => {
                warn!(
                    "Invalid Codex session_id header '{}': {}",
                    session_id, error
                );
            }
        }
    }

    info!("Connecting ChatGPT Codex websocket: {}", ws_url);
    let (connection, _) = connect_async(request).await?;
    Ok(connection)
}

async fn call_streaming_chatgpt_codex_http(
    client: &AiClient,
    body: Value,
    call_start: Instant,
) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
    let request = client
        .build_request(&client.config().api_url())
        .header("OpenAI-Beta", "responses=experimental");

    info!("Falling back to ChatGPT Codex HTTP streaming");
    let response = request.json(&body).send().await?;
    let response = ensure_success_stream_response(
        response,
        call_start,
        "ChatGPT Codex HTTP response",
        "ChatGPT Codex HTTP fallback error",
    )
    .await?;

    Ok(start_sse_stream(
        response,
        OpenAIParser::new(),
        "Codex HTTP",
        client.provider_id(),
        client.config().api_format,
        &client.config().model,
    ))
}

fn resolve_codex_ws_url(api_url: &str) -> Result<Url> {
    let mut url = Url::parse(api_url)
        .map_err(|e| anyhow::anyhow!("Invalid Codex API URL '{}': {}", api_url, e))?;

    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| anyhow::anyhow!("Failed to set websocket scheme for '{}'", api_url))?;

    Ok(url)
}
