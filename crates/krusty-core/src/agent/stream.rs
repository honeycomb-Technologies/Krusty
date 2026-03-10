//! Stream processing for the agentic loop.
//!
//! Consumes `StreamPart` events from `AiClient::call_streaming()` and:
//! - Accumulates text, thinking blocks, and tool calls
//! - Emits `LoopEvent`s for each meaningful state change
//! - Handles stream timeout (configurable idle timeout)

use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::streaming::StreamPart;
use crate::ai::types::AiToolCall;

use super::loop_events::{LoopEvent, LoopStopReason};

const RECOVERY_CHECKPOINT_CHAR_INTERVAL: usize = 256;

/// Accumulated thinking block from the AI response.
pub(crate) struct ThinkingBlock {
    pub thinking: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamToolCallSummary {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamCheckpoint {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<StreamToolCallSummary>,
}

/// Result of processing a complete AI stream.
pub(crate) struct StreamResult {
    pub text: String,
    pub thinking_blocks: Vec<ThinkingBlock>,
    pub tool_calls: Vec<AiToolCall>,
    pub recovery_checkpoint: StreamCheckpoint,
    pub last_error: Option<String>,
    pub total_tokens: usize,
    pub stop_reason: Option<LoopStopReason>,
}

/// Process an AI streaming response, emitting LoopEvents as chunks arrive.
///
/// Returns the accumulated result once the stream completes or times out.
pub(crate) async fn process_stream(
    mut api_rx: mpsc::UnboundedReceiver<StreamPart>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    idle_timeout: Duration,
    mut on_checkpoint: impl FnMut(&StreamCheckpoint),
) -> StreamResult {
    let mut text_buffer = String::new();
    let mut thinking_buffer = String::new();
    let mut thinking_blocks = Vec::new();
    let mut tool_calls = Vec::new();
    let mut recovery_tool_calls = Vec::new();
    let mut last_error = None;
    let mut total_tokens = 0usize;
    let mut stop_reason = None;
    let mut last_checkpoint_text_len = 0usize;

    loop {
        let part = match tokio::time::timeout(idle_timeout, api_rx.recv()).await {
            Ok(Some(part)) => part,
            Ok(None) => break,
            Err(_) => {
                let _ = event_tx.send(LoopEvent::Error {
                    error: format!(
                        "AI stream timeout: no data received for {} seconds",
                        idle_timeout.as_secs()
                    ),
                });
                last_error = Some(format!(
                    "AI stream timeout: no data received for {} seconds",
                    idle_timeout.as_secs()
                ));
                stop_reason = Some(LoopStopReason::StreamIdleTimeout);
                break;
            }
        };

        match &part {
            StreamPart::TextDelta { delta } => {
                text_buffer.push_str(delta);
                let _ = event_tx.send(LoopEvent::TextDelta {
                    delta: delta.clone(),
                });
                maybe_checkpoint(
                    &text_buffer,
                    &thinking_buffer,
                    &recovery_tool_calls,
                    &mut last_checkpoint_text_len,
                    &mut on_checkpoint,
                    false,
                );
            }
            StreamPart::ThinkingDelta { thinking, .. } => {
                thinking_buffer.push_str(thinking);
                let _ = event_tx.send(LoopEvent::ThinkingDelta {
                    thinking: thinking.clone(),
                });
                maybe_checkpoint(
                    &text_buffer,
                    &thinking_buffer,
                    &recovery_tool_calls,
                    &mut last_checkpoint_text_len,
                    &mut on_checkpoint,
                    false,
                );
            }
            StreamPart::ThinkingComplete {
                thinking,
                signature,
                ..
            } => {
                if thinking_buffer.is_empty() {
                    thinking_buffer = thinking.clone();
                }
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
                if !recovery_tool_calls.iter().any(|call| call.id == *id) {
                    recovery_tool_calls.push(StreamToolCallSummary {
                        id: id.clone(),
                        name: name.clone(),
                    });
                }
                let _ = event_tx.send(LoopEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                });
                maybe_checkpoint(
                    &text_buffer,
                    &thinking_buffer,
                    &recovery_tool_calls,
                    &mut last_checkpoint_text_len,
                    &mut on_checkpoint,
                    true,
                );
            }
            StreamPart::ToolCallComplete { tool_call } => {
                tool_calls.push(tool_call.clone());
                if !recovery_tool_calls
                    .iter()
                    .any(|call| call.id == tool_call.id)
                {
                    recovery_tool_calls.push(StreamToolCallSummary {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                    });
                }
                let _ = event_tx.send(LoopEvent::ToolCallComplete {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                });
                maybe_checkpoint(
                    &text_buffer,
                    &thinking_buffer,
                    &recovery_tool_calls,
                    &mut last_checkpoint_text_len,
                    &mut on_checkpoint,
                    true,
                );
            }
            StreamPart::Usage { usage } => {
                total_tokens = if usage.total_tokens > 0 {
                    usage.total_tokens
                } else {
                    usage.prompt_tokens + usage.completion_tokens
                };
                let _ = event_tx.send(LoopEvent::Usage {
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                });
            }
            StreamPart::TextDeltaWithCitations { delta, citations } => {
                text_buffer.push_str(delta);
                let _ = event_tx.send(LoopEvent::TextDeltaWithCitations {
                    delta: delta.clone(),
                    citations: citations.clone(),
                });
                maybe_checkpoint(
                    &text_buffer,
                    &thinking_buffer,
                    &recovery_tool_calls,
                    &mut last_checkpoint_text_len,
                    &mut on_checkpoint,
                    false,
                );
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
                last_error = Some(error.clone());
                stop_reason = Some(LoopStopReason::ProviderError);
                break;
            }
            _ => {}
        }
    }

    let recovery_checkpoint = StreamCheckpoint {
        text: text_buffer.clone(),
        thinking: thinking_buffer,
        tool_calls: recovery_tool_calls,
    };
    on_checkpoint(&recovery_checkpoint);

    StreamResult {
        text: text_buffer,
        thinking_blocks,
        tool_calls,
        recovery_checkpoint,
        last_error,
        total_tokens,
        stop_reason,
    }
}

fn maybe_checkpoint(
    text: &str,
    thinking: &str,
    tool_calls: &[StreamToolCallSummary],
    last_checkpoint_text_len: &mut usize,
    on_checkpoint: &mut impl FnMut(&StreamCheckpoint),
    force: bool,
) {
    if !force
        && text.len().saturating_sub(*last_checkpoint_text_len) < RECOVERY_CHECKPOINT_CHAR_INTERVAL
    {
        return;
    }

    *last_checkpoint_text_len = text.len();
    on_checkpoint(&StreamCheckpoint {
        text: text.to_string(),
        thinking: thinking.to_string(),
        tool_calls: tool_calls.to_vec(),
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::process_stream;
    use crate::agent::loop_events::LoopEvent;
    use crate::ai::streaming::StreamPart;
    use crate::ai::types::Usage;

    #[tokio::test]
    async fn usage_event_preserves_prompt_completion_split() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::Usage {
                usage: Usage {
                    prompt_tokens: 120,
                    completion_tokens: 45,
                    total_tokens: 165,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            })
            .expect("usage send should succeed");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert_eq!(result.total_tokens, 165);

        let usage_event = event_rx
            .recv()
            .await
            .expect("usage event should be emitted");
        match usage_event {
            LoopEvent::Usage {
                prompt_tokens: 120,
                completion_tokens: 45,
            } => {}
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn recovery_checkpoint_captures_partial_thinking() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::ThinkingDelta {
                index: 0,
                thinking: "step one".to_string(),
            })
            .expect("thinking delta should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert_eq!(result.recovery_checkpoint.thinking, "step one");
    }
}
