use serde_json::Value;
use tracing::{debug, info, warn};

use super::super::events::{SseEvent, SseParser};
use super::SseStreamProcessor;
use crate::ai::streaming::StreamPart;
use crate::ai::types::FinishReason;

impl SseStreamProcessor {
    /// Process SSE data using the provider-specific parser.
    pub async fn process_sse_data<P: SseParser>(
        &mut self,
        data: &str,
        parser: &P,
    ) -> anyhow::Result<()> {
        self.event_count += 1;
        let elapsed = self.stream_start.elapsed();

        if data == "[DONE]" {
            info!(
                "SSE stream [DONE] marker received after {:?}, {} events, {} bytes",
                elapsed, self.event_count, self.bytes_received
            );
            self.stream_buffer.flush().await;
            self.dispatch_part(StreamPart::Finish {
                reason: FinishReason::Stop,
            });
            return Ok(());
        }

        if let Ok(json) = serde_json::from_str::<Value>(data) {
            let event_type = json
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            debug!(
                "SSE event #{} at {:?}: type={}",
                self.event_count, elapsed, event_type
            );

            for event in parser.parse_events(&json).await? {
                match event {
                    SseEvent::TextDelta(text) => {
                        debug!("  -> TextDelta: {} chars", text.len());
                        self.stream_buffer.process_chunk(text).await;
                    }
                    SseEvent::TextDeltaWithCitations { text, citations } => {
                        debug!(
                            "  -> TextDeltaWithCitations: {} chars, {} citations",
                            text.len(),
                            citations.len()
                        );
                        self.dispatch_part(StreamPart::TextDeltaWithCitations {
                            delta: text,
                            citations,
                        });
                    }
                    SseEvent::ToolCallStart { id, name } => {
                        info!(
                            "SSE ToolCallStart: id={}, name={} at {:?}",
                            id, name, elapsed
                        );
                        self.dispatch_part(StreamPart::ToolCallStart { id, name });
                    }
                    SseEvent::ToolCallDelta { id, delta } => {
                        debug!("  -> ToolCallDelta: id={}, {} chars", id, delta.len());
                        self.dispatch_part(StreamPart::ToolCallDelta { id, delta });
                    }
                    SseEvent::ToolCallComplete(tool_call) => {
                        info!(
                            "SSE ToolCallComplete: id={}, name={} at {:?}",
                            tool_call.id, tool_call.name, elapsed
                        );
                        self.dispatch_part(StreamPart::ToolCallComplete { tool_call });
                    }
                    SseEvent::ServerToolStart { id, name } => {
                        info!(
                            "SSE ServerToolStart: id={}, name={} at {:?}",
                            id, name, elapsed
                        );
                        self.dispatch_part(StreamPart::ServerToolStart { id, name });
                    }
                    SseEvent::ServerToolDelta { id, delta } => {
                        debug!("  -> ServerToolDelta: id={}, {} chars", id, delta.len());
                        self.dispatch_part(StreamPart::ServerToolDelta { id, delta });
                    }
                    SseEvent::ServerToolComplete { id, name, input } => {
                        info!(
                            "SSE ServerToolComplete: id={}, name={} at {:?}",
                            id, name, elapsed
                        );
                        self.dispatch_part(StreamPart::ServerToolComplete { id, name, input });
                    }
                    SseEvent::WebSearchResults {
                        tool_use_id,
                        results,
                    } => {
                        info!(
                            "SSE WebSearchResults: {} results for {} at {:?}",
                            results.len(),
                            tool_use_id,
                            elapsed
                        );
                        self.dispatch_part(StreamPart::WebSearchResults {
                            tool_use_id,
                            results,
                        });
                    }
                    SseEvent::WebFetchResult {
                        tool_use_id,
                        content,
                    } => {
                        info!(
                            "SSE WebFetchResult: url={} for {} at {:?}",
                            content.url, tool_use_id, elapsed
                        );
                        self.dispatch_part(StreamPart::WebFetchResult {
                            tool_use_id,
                            content,
                        });
                    }
                    SseEvent::ServerToolError {
                        tool_use_id,
                        error_code,
                    } => {
                        warn!(
                            "SSE ServerToolError: {} for {} at {:?}",
                            error_code, tool_use_id, elapsed
                        );
                        self.dispatch_part(StreamPart::ServerToolError {
                            tool_use_id,
                            error_code,
                        });
                    }
                    SseEvent::ThinkingStart { index } => {
                        info!("SSE ThinkingStart: index={} at {:?}", index, elapsed);
                        self.dispatch_part(StreamPart::ThinkingStart { index });
                    }
                    SseEvent::ThinkingDelta { index, thinking } => {
                        debug!(
                            "  -> ThinkingDelta: index={}, {} chars",
                            index,
                            thinking.len()
                        );
                        self.dispatch_part(StreamPart::ThinkingDelta { index, thinking });
                    }
                    SseEvent::SignatureDelta { index, signature } => {
                        debug!(
                            "  -> SignatureDelta: index={}, {} chars",
                            index,
                            signature.len()
                        );
                        self.dispatch_part(StreamPart::SignatureDelta { index, signature });
                    }
                    SseEvent::ThinkingComplete {
                        index,
                        thinking,
                        signature,
                    } => {
                        info!(
                        "SSE ThinkingComplete: index={}, thinking={} chars, sig={} chars at {:?}",
                        index,
                        thinking.len(),
                        signature.len(),
                        elapsed
                    );
                        self.dispatch_part(StreamPart::ThinkingComplete {
                            index,
                            thinking,
                            signature,
                        });
                    }
                    SseEvent::Finish { reason, usage } => {
                        info!(
                            "SSE Finish: reason={:?} at {:?} ({} events, {} bytes)",
                            reason, elapsed, self.event_count, self.bytes_received
                        );
                        self.stream_buffer.flush().await;
                        if let Some(usage) = usage {
                            self.emit_usage(usage, "from finish");
                        }
                        self.dispatch_part(StreamPart::Finish { reason });
                    }
                    SseEvent::FinishWithToolCalls { tool_calls, usage } => {
                        info!(
                            "SSE FinishWithToolCalls: {} tool calls at {:?} ({} events, {} bytes)",
                            tool_calls.len(),
                            elapsed,
                            self.event_count,
                            self.bytes_received
                        );
                        self.stream_buffer.flush().await;
                        for tool_call in tool_calls {
                            info!(
                                "  -> Completing tool call: id={}, name={}",
                                tool_call.id, tool_call.name
                            );
                            self.dispatch_part(StreamPart::ToolCallComplete { tool_call });
                        }
                        if let Some(usage) = usage {
                            self.emit_usage(usage, "from finish");
                        }
                        self.dispatch_part(StreamPart::Finish {
                            reason: FinishReason::ToolCalls,
                        });
                    }
                    SseEvent::Usage(usage) => {
                        self.emit_usage(usage, "event");
                    }
                    SseEvent::ContextEdited(metrics) => {
                        info!(
                        "SSE ContextEdited: cleared {} tokens ({} tool uses, {} thinking turns)",
                        metrics.cleared_input_tokens,
                        metrics.cleared_tool_uses,
                        metrics.cleared_thinking_turns
                    );
                        self.dispatch_part(StreamPart::ContextEdited { metrics });
                    }
                    SseEvent::Skip => {
                        debug!("  -> Skip event");
                    }
                }
            }
        } else if !data.is_empty() && !data.trim().is_empty() {
            warn!(
                "Failed to parse SSE JSON (event #{}): {}",
                self.event_count, data
            );
        }

        Ok(())
    }
}
