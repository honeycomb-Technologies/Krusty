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
use super::super::shared::{
    ensure_success_stream_response, log_system_prompt_layers, start_sse_stream,
};
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::parsers::OpenAIParser;
use crate::ai::sse::{create_streaming_channels, spawn_buffer_processor, SseStreamProcessor};
use crate::ai::streaming::StreamPart;
use crate::ai::types::ModelMessage;

enum CodexPayloadState {
    Continue,
    Complete,
    Error,
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
            return CodexPayloadState::Complete;
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
    log_system_prompt_layers(
        "codex_stream",
        &prompt_sections,
        options.system_prompt.is_some(),
    );
    let system_prompt = prompt_sections.combined();

    let max_tokens = options.max_tokens.unwrap_or(client.config().max_tokens);
    let body = client.build_chatgpt_codex_body(
        &messages,
        &system_prompt,
        max_tokens,
        options,
        &format_handler,
    );

    let ws_url = resolve_codex_ws_url(&client.config().api_url())?;
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
            Err(e) => {
                warn!("Invalid Codex session_id header '{}': {}", session_id, e);
            }
        }
    }

    info!("Connecting ChatGPT Codex websocket: {}", ws_url);
    let (mut ws_stream, _) = match connect_async(request).await {
        Ok(pair) => pair,
        Err(e) => {
            warn!(
                "ChatGPT Codex websocket connect failed ({}), falling back to HTTP streaming",
                e
            );
            return call_streaming_chatgpt_codex_http(client, body, call_start).await;
        }
    };
    info!(
        "ChatGPT Codex websocket connected in {:?}",
        call_start.elapsed()
    );

    let create_payload = AiClient::codex_ws_create_payload(body.clone());
    if let Err(e) = ws_stream
        .send(Message::Text(create_payload.to_string()))
        .await
    {
        warn!(
            "ChatGPT Codex websocket send failed ({}), falling back to HTTP streaming",
            e
        );
        return call_streaming_chatgpt_codex_http(client, body, call_start).await;
    }

    let first_ws_message =
        (tokio::time::timeout(Duration::from_secs(2), ws_stream.next()).await).unwrap_or_default();

    if matches!(
        first_ws_message,
        Some(Ok(Message::Close(_))) | Some(Err(_)) | None
    ) {
        warn!("ChatGPT Codex websocket closed before first event, falling back to HTTP streaming");
        return call_streaming_chatgpt_codex_http(client, body, call_start).await;
    }

    let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
    spawn_buffer_processor(buffer_rx, tx.clone());
    let tx_err = tx.clone();

    let mut processor = SseStreamProcessor::new(tx, buffer_tx).with_transform_context(
        client.provider_id(),
        client.config().api_format,
        client.config().model.clone(),
    );
    let parser = OpenAIParser::new();

    tokio::spawn(async move {
        let (_write, mut read) = ws_stream.split();

        let mut pending_first = first_ws_message;

        loop {
            let msg = if let Some(msg) = pending_first.take() {
                msg
            } else {
                match read.next().await {
                    Some(msg) => msg,
                    None => break,
                }
            };

            match msg {
                Ok(Message::Text(text)) => {
                    let payload = text.to_string();
                    match process_codex_ws_payload(&payload, &parser, &mut processor, &tx_err).await
                    {
                        CodexPayloadState::Continue => {}
                        CodexPayloadState::Complete => break,
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
                        CodexPayloadState::Complete => break,
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

        processor.finish().await;
    });

    Ok(rx)
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
