//! Stream processing for the agentic loop.
//!
//! Consumes `StreamPart` events from `AiClient::call_streaming()` and:
//! - Accumulates text, thinking blocks, and tool calls
//! - Emits `LoopEvent`s for each meaningful state change
//! - Handles stream timeout (120s of no data)

use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::streaming::StreamPart;
use crate::ai::types::AiToolCall;

use super::loop_events::LoopEvent;

const STREAM_TIMEOUT: Duration = Duration::from_secs(120);

/// Accumulated thinking block from the AI response.
pub(crate) struct ThinkingBlock {
    pub thinking: String,
    pub signature: String,
}

/// Result of processing a complete AI stream.
pub(crate) struct StreamResult {
    pub text: String,
    pub thinking_blocks: Vec<ThinkingBlock>,
    pub tool_calls: Vec<AiToolCall>,
    pub total_tokens: usize,
}

/// Process an AI streaming response, emitting LoopEvents as chunks arrive.
///
/// Returns the accumulated result once the stream completes or times out.
pub(crate) async fn process_stream(
    mut api_rx: mpsc::UnboundedReceiver<StreamPart>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> StreamResult {
    let mut text_buffer = String::new();
    let mut thinking_blocks = Vec::new();
    let mut tool_calls = Vec::new();
    let mut total_tokens = 0usize;

    loop {
        let part = match tokio::time::timeout(STREAM_TIMEOUT, api_rx.recv()).await {
            Ok(Some(part)) => part,
            Ok(None) => break,
            Err(_) => {
                let _ = event_tx.send(LoopEvent::Error {
                    error: "AI stream timeout: no data received for 120 seconds".to_string(),
                });
                break;
            }
        };

        match &part {
            StreamPart::TextDelta { delta } => {
                text_buffer.push_str(delta);
                let _ = event_tx.send(LoopEvent::TextDelta {
                    delta: delta.clone(),
                });
            }
            StreamPart::ThinkingDelta { thinking, .. } => {
                let _ = event_tx.send(LoopEvent::ThinkingDelta {
                    thinking: thinking.clone(),
                });
            }
            StreamPart::ThinkingComplete {
                thinking,
                signature,
                ..
            } => {
                thinking_blocks.push(ThinkingBlock {
                    thinking: thinking.clone(),
                    signature: signature.clone(),
                });
                let _ = event_tx.send(LoopEvent::ThinkingComplete {
                    thinking: thinking.clone(),
                    signature: signature.clone(),
                });
            }
            StreamPart::ToolCallStart { id, name } => {
                let _ = event_tx.send(LoopEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                });
            }
            StreamPart::ToolCallComplete { tool_call } => {
                tool_calls.push(tool_call.clone());
                let _ = event_tx.send(LoopEvent::ToolCallComplete {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                });
            }
            StreamPart::Usage { usage } => {
                total_tokens = usage.prompt_tokens + usage.completion_tokens;
                let _ = event_tx.send(LoopEvent::Usage {
                    prompt_tokens: total_tokens,
                    completion_tokens: usage.completion_tokens,
                });
            }
            StreamPart::TextDeltaWithCitations { delta, citations } => {
                text_buffer.push_str(delta);
                let _ = event_tx.send(LoopEvent::TextDeltaWithCitations {
                    delta: delta.clone(),
                    citations: citations.clone(),
                });
            }
            StreamPart::ServerToolStart { id, name } => {
                let _ = event_tx.send(LoopEvent::ServerToolStart {
                    id: id.clone(),
                    name: name.clone(),
                });
            }
            StreamPart::ServerToolComplete { id, name, .. } => {
                let _ = event_tx.send(LoopEvent::ServerToolComplete {
                    id: id.clone(),
                    name: name.clone(),
                });
            }
            StreamPart::WebSearchResults {
                tool_use_id,
                results,
            } => {
                let _ = event_tx.send(LoopEvent::WebSearchResults {
                    tool_use_id: tool_use_id.clone(),
                    results: results.clone(),
                });
            }
            StreamPart::WebFetchResult {
                tool_use_id,
                content,
            } => {
                let _ = event_tx.send(LoopEvent::WebFetchResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content.clone(),
                });
            }
            StreamPart::ServerToolError {
                tool_use_id,
                error_code,
            } => {
                let _ = event_tx.send(LoopEvent::ServerToolError {
                    tool_use_id: tool_use_id.clone(),
                    error_code: error_code.clone(),
                });
            }
            StreamPart::Error { error } => {
                let _ = event_tx.send(LoopEvent::Error {
                    error: error.clone(),
                });
            }
            _ => {}
        }
    }

    StreamResult {
        text: text_buffer,
        thinking_blocks,
        tool_calls,
        total_tokens,
    }
}
