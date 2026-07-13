//! Stream processing for the agentic loop.
//!
//! Consumes `StreamPart` events from `AiClient::call_streaming()` and:
//! - Accumulates text, thinking blocks, and tool calls
//! - Emits `LoopEvent`s for each meaningful state change
//! - Handles stream timeout (configurable idle timeout)

use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::streaming::StreamPart;
use crate::ai::types::{AiToolCall, FinishReason, Usage};
use serde_json::Value;

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
    pub arguments: Value,
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
    pub prompt_tokens: usize,
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
    let mut usage = Usage::default();
    let mut stop_reason = None;
    let mut received_finish = false;
    let mut last_checkpoint_text_len = 0usize;

    loop {
        let receive_timeout = if received_finish {
            // OpenAI-compatible streams can emit their usage-only frame after
            // the finish reason. Drain already queued telemetry without making
            // every provider keep the turn open for the full idle timeout.
            Duration::from_millis(250)
        } else {
            idle_timeout
        };
        let part = match tokio::time::timeout(receive_timeout, api_rx.recv()).await {
            Ok(Some(part)) => part,
            Ok(None) if received_finish => break,
            Ok(None) => {
                let error = "AI stream ended without a finish signal".to_string();
                let _ = event_tx.send(LoopEvent::Error {
                    error: error.clone(),
                });
                last_error = Some(error);
                stop_reason = Some(LoopStopReason::ProviderError);
                break;
            }
            Err(_) if received_finish => break,
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

        if received_finish && !matches!(&part, StreamPart::Usage { .. } | StreamPart::Finish { .. })
        {
            let error = "AI provider emitted content after its finish signal".to_string();
            let _ = event_tx.send(LoopEvent::Error {
                error: error.clone(),
            });
            last_error = Some(error);
            stop_reason = Some(LoopStopReason::ProviderError);
            break;
        }

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
                        arguments: Value::Null,
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
                if let Some(existing) = recovery_tool_calls
                    .iter_mut()
                    .find(|call| call.id == tool_call.id)
                {
                    existing.name = tool_call.name.clone();
                    existing.arguments = tool_call.arguments.clone();
                } else {
                    recovery_tool_calls.push(StreamToolCallSummary {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
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
            StreamPart::Finish { reason } => {
                let incomplete_tool_calls = recovery_tool_calls
                    .iter()
                    .filter(|pending| !tool_calls.iter().any(|call| call.id == pending.id))
                    .count();
                match reason {
                    FinishReason::Stop | FinishReason::ToolCalls if incomplete_tool_calls > 0 => {
                        let error = format!(
                            "AI response ended with {incomplete_tool_calls} incomplete tool call(s); none were executed"
                        );
                        let _ = event_tx.send(LoopEvent::Error {
                            error: error.clone(),
                        });
                        last_error = Some(error);
                        stop_reason = Some(LoopStopReason::ProviderError);
                    }
                    FinishReason::ToolCalls if tool_calls.is_empty() => {
                        let error = "AI provider reported tool calls but supplied no complete calls; none were executed".to_string();
                        let _ = event_tx.send(LoopEvent::Error {
                            error: error.clone(),
                        });
                        last_error = Some(error);
                        stop_reason = Some(LoopStopReason::ProviderError);
                    }
                    FinishReason::Stop | FinishReason::ToolCalls => {}
                    FinishReason::Length => {
                        let error = "AI response reached its output-token limit; incomplete tool calls were not executed".to_string();
                        let _ = event_tx.send(LoopEvent::Error {
                            error: error.clone(),
                        });
                        last_error = Some(error);
                        stop_reason = Some(LoopStopReason::ProviderError);
                    }
                    FinishReason::ContentFilter => {
                        let error =
                            "AI response was blocked by the provider content filter".to_string();
                        let _ = event_tx.send(LoopEvent::Error {
                            error: error.clone(),
                        });
                        last_error = Some(error);
                        stop_reason = Some(LoopStopReason::ProviderError);
                    }
                    FinishReason::Other(reason) => {
                        let error = format!("AI response ended unexpectedly: {reason}");
                        let _ = event_tx.send(LoopEvent::Error {
                            error: error.clone(),
                        });
                        last_error = Some(error);
                        stop_reason = Some(LoopStopReason::ProviderError);
                    }
                }
                if stop_reason.is_some() {
                    break;
                }
                received_finish = true;
            }
            StreamPart::Usage { usage: snapshot } => {
                // Usage snapshots can split input and output across events.
                // Merge them before publishing so downstream cache telemetry and
                // context budgeting always see the complete turn so far.
                usage.merge_snapshot(snapshot);
                let _ = event_tx.send(LoopEvent::Usage {
                    prompt_tokens: usage.prompt_tokens,
                    completion_tokens: usage.completion_tokens,
                    cache_creation_input_tokens: usage.cache_creation_input_tokens,
                    cache_read_input_tokens: usage.cache_read_input_tokens,
                    total_tokens: usage.total_tokens,
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
        total_tokens: usage.total_tokens,
        prompt_tokens: usage.input_tokens(),
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
    use crate::agent::loop_events::{LoopEvent, LoopStopReason};
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
        api_tx
            .send(StreamPart::Finish {
                reason: crate::ai::types::FinishReason::Stop,
            })
            .expect("finish should send");
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
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                total_tokens: 165,
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
        api_tx
            .send(StreamPart::Finish {
                reason: crate::ai::types::FinishReason::Stop,
            })
            .expect("finish should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert_eq!(result.recovery_checkpoint.thinking, "step one");
    }

    #[tokio::test]
    async fn usage_snapshots_merge_input_cache_and_output() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::Usage {
                usage: Usage {
                    prompt_tokens: 100,
                    completion_tokens: 0,
                    total_tokens: 1_000,
                    cache_creation_input_tokens: 200,
                    cache_read_input_tokens: 700,
                },
            })
            .expect("start usage should send");
        api_tx
            .send(StreamPart::Usage {
                usage: Usage {
                    prompt_tokens: 0,
                    completion_tokens: 50,
                    total_tokens: 50,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            })
            .expect("delta usage should send");
        api_tx
            .send(StreamPart::Finish {
                reason: crate::ai::types::FinishReason::Stop,
            })
            .expect("finish should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert_eq!(result.prompt_tokens, 1_000);
        assert_eq!(result.total_tokens, 1_050);
    }

    #[tokio::test]
    async fn usage_frame_after_finish_is_drained() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::Finish {
                reason: crate::ai::types::FinishReason::Stop,
            })
            .expect("finish should send");
        api_tx
            .send(StreamPart::Usage {
                usage: Usage {
                    prompt_tokens: 25,
                    completion_tokens: 10,
                    total_tokens: 100,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 65,
                },
            })
            .expect("trailing usage should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert_eq!(result.prompt_tokens, 90);
        assert_eq!(result.total_tokens, 100);
        assert!(result.stop_reason.is_none());
    }

    #[tokio::test]
    async fn length_finish_is_terminal_and_never_executes_partial_tools() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::ToolCallStart {
                id: "partial".to_string(),
                name: "bash".to_string(),
            })
            .expect("tool start should send");
        api_tx
            .send(StreamPart::Finish {
                reason: crate::ai::types::FinishReason::Length,
            })
            .expect("finish should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, Some(LoopStopReason::ProviderError));
        assert!(result
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("output-token limit")));
    }

    #[tokio::test]
    async fn nominal_finish_rejects_incomplete_tool_calls() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::ToolCallStart {
                id: "partial".to_string(),
                name: "edit".to_string(),
            })
            .expect("tool start should send");
        api_tx
            .send(StreamPart::Finish {
                reason: crate::ai::types::FinishReason::Stop,
            })
            .expect("finish should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, Some(LoopStopReason::ProviderError));
        assert!(result
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("incomplete tool call")));
    }

    #[tokio::test]
    async fn channel_close_without_finish_is_provider_error() {
        let (api_tx, api_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();

        api_tx
            .send(StreamPart::TextDelta {
                delta: "partial".to_string(),
            })
            .expect("text should send");
        drop(api_tx);

        let result = process_stream(api_rx, &event_tx, Duration::from_secs(1), |_| {}).await;
        assert_eq!(result.stop_reason, Some(LoopStopReason::ProviderError));
        assert_eq!(
            result.last_error.as_deref(),
            Some("AI stream ended without a finish signal")
        );
    }
}
