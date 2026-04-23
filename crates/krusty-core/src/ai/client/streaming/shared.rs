use anyhow::Result;
use futures::StreamExt;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::ai::model_profile::SystemPromptSections;
use crate::ai::providers::ProviderId;
use crate::ai::sse::{
    create_streaming_channels, spawn_buffer_processor, SseParser, SseStreamProcessor,
};
use crate::ai::streaming::StreamPart;
/// Spawn a stream processing task for an HTTP SSE response.
///
/// Handles the common pattern of reading bytes from a response stream,
/// parsing SSE events, and forwarding them through channels. Sends an
/// explicit error signal if the stream fails, ensuring the receiver
/// never waits on a silently-dead channel.
fn spawn_sse_stream_task<S, P>(
    stream: S,
    mut processor: SseStreamProcessor,
    parser: P,
    tx_err: mpsc::UnboundedSender<StreamPart>,
    label: &'static str,
) where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
    P: SseParser + 'static,
{
    tokio::spawn(async move {
        tokio::pin!(stream);
        let mut chunk_count: u64 = 0;
        let mut had_error = false;

        while let Some(chunk) = stream.next().await {
            chunk_count += 1;
            match chunk {
                Ok(bytes) => {
                    if let Err(e) = processor.process_chunk(bytes, &parser).await {
                        warn!("{} chunk #{} parse error: {}", label, chunk_count, e);
                        let _ = tx_err.send(StreamPart::Error {
                            error: format!("{} parse error: {}", label, e),
                        });
                        had_error = true;
                        break;
                    }
                }
                Err(e) => {
                    error!("{} read error at chunk #{}: {}", label, chunk_count, e);
                    let _ = tx_err.send(StreamPart::Error {
                        error: format!("{} read error: {}", label, e),
                    });
                    had_error = true;
                    break;
                }
            }
        }

        if !had_error {
            info!("{} stream ended after {} chunks", label, chunk_count);
        }
        processor.finish().await;
    });
}

pub(super) fn log_system_prompt_layers(
    label: &str,
    sections: &SystemPromptSections,
    custom_prompt: bool,
) {
    debug!(
        stream_kind = label,
        prompt_family = ?sections.profile.prompt_family,
        base_chars = sections.base_prompt.len(),
        project_chars = sections.project_context.len(),
        session_chars = sections.session_context.len(),
        custom_prompt,
        "Built system prompt layers"
    );
}

pub(super) async fn ensure_success_stream_response(
    response: reqwest::Response,
    call_start: Instant,
    response_label: &str,
    error_label: &str,
) -> Result<reqwest::Response> {
    let status = response.status();
    info!(
        "{}: {} in {:?}",
        response_label,
        status,
        call_start.elapsed()
    );

    if status.is_success() {
        return Ok(response);
    }

    let error_text = response
        .text()
        .await
        .unwrap_or_else(|_| "Unknown error".to_string());
    error!("{}: {} - {}", error_label, status, error_text);
    Err(anyhow::anyhow!(
        "{}: {} - {}",
        error_label,
        status,
        error_text
    ))
}

pub(super) fn start_sse_stream<P>(
    response: reqwest::Response,
    parser: P,
    label: &'static str,
    provider_id: ProviderId,
    api_format: crate::ai::models::ApiFormat,
    model_id: &str,
) -> mpsc::UnboundedReceiver<StreamPart>
where
    P: SseParser + 'static,
{
    let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
    spawn_buffer_processor(buffer_rx, tx.clone());

    let processor = SseStreamProcessor::new(tx.clone(), buffer_tx).with_transform_context(
        provider_id,
        api_format,
        model_id.to_string(),
    );
    spawn_sse_stream_task(response.bytes_stream(), processor, parser, tx, label);

    rx
}
