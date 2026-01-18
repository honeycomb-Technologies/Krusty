//! SSE (Server-Sent Events) stream processing utilities
//!
//! Handles parsing of SSE streams from AI providers

use bytes::Bytes;
use serde_json::Value;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::stream_buffer::StreamBuffer;
use super::streaming::StreamPart;
use super::types::{
    AiToolCall, Citation, ContextEditingMetrics, FinishReason, Usage, WebFetchContent,
    WebSearchResult,
};

/// Common SSE stream processor that handles partial lines and buffering
pub struct SseStreamProcessor {
    /// Accumulated partial line from previous chunks
    partial_line: String,
    /// Stream buffer for smooth text streaming
    stream_buffer: StreamBuffer,
    /// Channel to send processed stream parts
    tx: mpsc::UnboundedSender<StreamPart>,
    /// When the stream started
    stream_start: Instant,
    /// Event counter for logging
    event_count: usize,
    /// Bytes received counter
    bytes_received: usize,
}

impl SseStreamProcessor {
    /// Create a new SSE stream processor
    pub fn new(
        tx: mpsc::UnboundedSender<StreamPart>,
        buffer_tx: mpsc::UnboundedSender<String>,
    ) -> Self {
        info!("SSE stream processor created");
        Self {
            partial_line: String::new(),
            stream_buffer: StreamBuffer::new(buffer_tx),
            tx,
            stream_start: Instant::now(),
            event_count: 0,
            bytes_received: 0,
        }
    }

    /// Process a chunk of bytes from the SSE stream
    pub async fn process_chunk<P: SseParser>(
        &mut self,
        bytes: Bytes,
        parser: &P,
    ) -> anyhow::Result<()> {
        self.bytes_received += bytes.len();
        let text = String::from_utf8_lossy(&bytes);

        // Combine with any partial line from previous chunk
        // Use push_str to avoid format!() allocation when partial_line is empty
        let combined = if self.partial_line.is_empty() {
            text.into_owned()
        } else {
            let mut combined = std::mem::take(&mut self.partial_line);
            combined.push_str(&text);
            combined
        };

        debug!(
            "SSE chunk received: {} bytes (total: {} bytes)",
            bytes.len(),
            self.bytes_received
        );

        let has_trailing_newline = combined.ends_with('\n');
        let mut lines_iter = combined.lines().peekable();

        // Process lines - use peekable to detect last line without collecting
        while let Some(line) = lines_iter.next() {
            // If this is the last line and there's no trailing newline, it's partial
            if lines_iter.peek().is_none() && !has_trailing_newline {
                self.partial_line = line.to_string();
                break;
            }

            // Skip empty lines and SSE comments
            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            // Process SSE event line
            if let Some(data) = line.strip_prefix("data: ") {
                self.process_sse_data(data, parser).await?;
            }
        }

        Ok(())
    }

    /// Process SSE data using the provider-specific parser
    pub async fn process_sse_data<P: SseParser>(
        &mut self,
        data: &str,
        parser: &P,
    ) -> anyhow::Result<()> {
        self.event_count += 1;
        let elapsed = self.stream_start.elapsed();

        // Handle end-of-stream marker
        if data == "[DONE]" {
            info!(
                "SSE stream [DONE] marker received after {:?}, {} events, {} bytes",
                elapsed, self.event_count, self.bytes_received
            );
            self.stream_buffer.flush().await;
            let _ = self.tx.send(StreamPart::Finish {
                reason: FinishReason::Stop,
            });
            return Ok(());
        }

        // Parse JSON and convert to stream events
        if let Ok(json) = serde_json::from_str::<Value>(data) {
            // Log the raw event type for debugging
            let event_type = json
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            debug!(
                "SSE event #{} at {:?}: type={}",
                self.event_count, elapsed, event_type
            );

            match parser.parse_event(&json).await? {
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
                    let _ = self.tx.send(StreamPart::TextDeltaWithCitations {
                        delta: text,
                        citations,
                    });
                }
                SseEvent::ToolCallStart { id, name } => {
                    info!(
                        "SSE ToolCallStart: id={}, name={} at {:?}",
                        id, name, elapsed
                    );
                    let _ = self.tx.send(StreamPart::ToolCallStart { id, name });
                }
                SseEvent::ToolCallDelta { id, delta } => {
                    debug!("  -> ToolCallDelta: id={}, {} chars", id, delta.len());
                    let _ = self.tx.send(StreamPart::ToolCallDelta { id, delta });
                }
                SseEvent::ToolCallComplete(tool_call) => {
                    info!(
                        "SSE ToolCallComplete: id={}, name={} at {:?}",
                        tool_call.id, tool_call.name, elapsed
                    );
                    let _ = self.tx.send(StreamPart::ToolCallComplete { tool_call });
                }
                // Server-executed tools
                SseEvent::ServerToolStart { id, name } => {
                    info!(
                        "SSE ServerToolStart: id={}, name={} at {:?}",
                        id, name, elapsed
                    );
                    let _ = self.tx.send(StreamPart::ServerToolStart { id, name });
                }
                SseEvent::ServerToolDelta { id, delta } => {
                    debug!("  -> ServerToolDelta: id={}, {} chars", id, delta.len());
                    let _ = self.tx.send(StreamPart::ServerToolDelta { id, delta });
                }
                SseEvent::ServerToolComplete { id, name, input } => {
                    info!(
                        "SSE ServerToolComplete: id={}, name={} at {:?}",
                        id, name, elapsed
                    );
                    let _ = self
                        .tx
                        .send(StreamPart::ServerToolComplete { id, name, input });
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
                    let _ = self.tx.send(StreamPart::WebSearchResults {
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
                    let _ = self.tx.send(StreamPart::WebFetchResult {
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
                    let _ = self.tx.send(StreamPart::ServerToolError {
                        tool_use_id,
                        error_code,
                    });
                }
                // Extended thinking
                SseEvent::ThinkingStart { index } => {
                    info!("SSE ThinkingStart: index={} at {:?}", index, elapsed);
                    let _ = self.tx.send(StreamPart::ThinkingStart { index });
                }
                SseEvent::ThinkingDelta { index, thinking } => {
                    debug!(
                        "  -> ThinkingDelta: index={}, {} chars",
                        index,
                        thinking.len()
                    );
                    let _ = self.tx.send(StreamPart::ThinkingDelta { index, thinking });
                }
                SseEvent::SignatureDelta { index, signature } => {
                    debug!(
                        "  -> SignatureDelta: index={}, {} chars",
                        index,
                        signature.len()
                    );
                    let _ = self
                        .tx
                        .send(StreamPart::SignatureDelta { index, signature });
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
                    let _ = self.tx.send(StreamPart::ThinkingComplete {
                        index,
                        thinking,
                        signature,
                    });
                }
                SseEvent::Finish { reason } => {
                    info!(
                        "SSE Finish: reason={:?} at {:?} ({} events, {} bytes)",
                        reason, elapsed, self.event_count, self.bytes_received
                    );
                    self.stream_buffer.flush().await;
                    let _ = self.tx.send(StreamPart::Finish { reason });
                }
                SseEvent::Usage(usage) => {
                    info!("SSE Usage: prompt={}, completion={}, total={}, cache_read={}, cache_created={}",
                        usage.prompt_tokens, usage.completion_tokens, usage.total_tokens,
                        usage.cache_read_input_tokens, usage.cache_creation_input_tokens);
                    let _ = self.tx.send(StreamPart::Usage { usage });
                }
                SseEvent::ContextEdited(metrics) => {
                    info!(
                        "SSE ContextEdited: cleared {} tokens ({} tool uses, {} thinking turns)",
                        metrics.cleared_input_tokens,
                        metrics.cleared_tool_uses,
                        metrics.cleared_thinking_turns
                    );
                    let _ = self.tx.send(StreamPart::ContextEdited { metrics });
                }
                SseEvent::Skip => {
                    // Event should be ignored
                    debug!("  -> Skip event");
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

    /// Finish processing and ensure all buffers are flushed
    pub async fn finish(&mut self) {
        let elapsed = self.stream_start.elapsed();
        info!(
            "SSE stream processor finishing: {:?} elapsed, {} events, {} bytes total",
            elapsed, self.event_count, self.bytes_received
        );
        self.stream_buffer.finish().await;
    }
}

/// Events that can be parsed from SSE data
pub enum SseEvent {
    TextDelta(String),
    TextDeltaWithCitations {
        text: String,
        citations: Vec<Citation>,
    },
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallDelta {
        id: String,
        delta: String,
    },
    ToolCallComplete(AiToolCall),
    // Server-executed tools (web_search, web_fetch)
    ServerToolStart {
        id: String,
        name: String,
    },
    ServerToolDelta {
        id: String,
        delta: String,
    },
    ServerToolComplete {
        id: String,
        name: String,
        input: Value,
    },
    WebSearchResults {
        tool_use_id: String,
        results: Vec<WebSearchResult>,
    },
    WebFetchResult {
        tool_use_id: String,
        content: WebFetchContent,
    },
    ServerToolError {
        tool_use_id: String,
        error_code: String,
    },
    // Extended thinking
    ThinkingStart {
        index: usize,
    },
    ThinkingDelta {
        index: usize,
        thinking: String,
    },
    SignatureDelta {
        index: usize,
        signature: String,
    },
    ThinkingComplete {
        index: usize,
        thinking: String,
        signature: String,
    },
    Finish {
        reason: FinishReason,
    },
    Usage(Usage),
    ContextEdited(ContextEditingMetrics),
    Skip,
}

/// Trait for provider-specific SSE parsing logic
#[async_trait::async_trait]
pub trait SseParser: Send + Sync {
    /// Parse a JSON event into an SSE event
    async fn parse_event(&self, json: &Value) -> anyhow::Result<SseEvent>;
}

/// Common helper to parse finish reasons
pub fn parse_finish_reason(reason_str: &str) -> FinishReason {
    match reason_str {
        "stop" | "end_turn" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolCalls,
        _ => FinishReason::Other(reason_str.to_string()),
    }
}

/// Create standard streaming channels with buffer processing
pub fn create_streaming_channels() -> (
    mpsc::UnboundedSender<StreamPart>,
    mpsc::UnboundedReceiver<StreamPart>,
    mpsc::UnboundedSender<String>,
    mpsc::UnboundedReceiver<String>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, buffer_rx) = mpsc::unbounded_channel::<String>();
    (tx, rx, buffer_tx, buffer_rx)
}

/// Spawn a task to convert buffered text into StreamParts
pub fn spawn_buffer_processor(
    mut buffer_rx: mpsc::UnboundedReceiver<String>,
    tx: mpsc::UnboundedSender<StreamPart>,
) {
    tokio::spawn(async move {
        while let Some(text) = buffer_rx.recv().await {
            let _ = tx.send(StreamPart::TextDelta { delta: text });
        }
    });
}

/// Tool call accumulator for providers that stream tool calls in parts
#[derive(Debug, Clone)]
pub struct ToolCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub is_complete: bool,
}

/// Server tool accumulator for web_search/web_fetch
#[derive(Debug, Clone)]
pub struct ServerToolAccumulator {
    pub id: String,
    pub name: String,
    pub input_json: String,
    pub is_complete: bool,
}

impl ServerToolAccumulator {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            input_json: String::new(),
            is_complete: false,
        }
    }

    pub fn add_input(&mut self, delta: &str) {
        self.input_json.push_str(delta);
    }

    pub fn complete(&mut self) -> Value {
        self.is_complete = true;
        if self.input_json.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str::<Value>(&self.input_json)
                .unwrap_or_else(|_| serde_json::json!({"raw": self.input_json.clone()}))
        }
    }
}

/// Thinking block accumulator for extended thinking
#[derive(Debug, Clone)]
pub struct ThinkingAccumulator {
    pub thinking: String,
    pub signature: String,
    pub is_complete: bool,
}

impl ThinkingAccumulator {
    pub fn new() -> Self {
        Self {
            thinking: String::new(),
            signature: String::new(),
            is_complete: false,
        }
    }

    pub fn add_thinking(&mut self, delta: &str) {
        self.thinking.push_str(delta);
    }

    pub fn add_signature(&mut self, delta: &str) {
        self.signature.push_str(delta);
    }

    pub fn complete(&mut self) -> (String, String) {
        self.is_complete = true;
        (self.thinking.clone(), self.signature.clone())
    }
}

impl Default for ThinkingAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolCallAccumulator {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            arguments: String::new(),
            is_complete: false,
        }
    }

    pub fn add_arguments(&mut self, delta: &str) {
        self.arguments.push_str(delta);
    }

    pub fn try_complete(&mut self) -> Option<AiToolCall> {
        if !self.arguments.is_empty() {
            if let Ok(parsed) = serde_json::from_str::<Value>(&self.arguments) {
                self.is_complete = true;
                return Some(AiToolCall {
                    id: self.id.clone(),
                    name: self.name.clone(),
                    arguments: parsed,
                });
            }
        }
        None
    }

    pub fn force_complete(&mut self) -> AiToolCall {
        self.is_complete = true;
        AiToolCall {
            id: self.id.clone(),
            name: self.name.clone(),
            arguments: if self.arguments.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str::<Value>(&self.arguments)
                    .unwrap_or_else(|_| serde_json::json!({"raw": self.arguments.clone()}))
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // parse_finish_reason tests
    #[test]
    fn test_parse_finish_reason_stop() {
        assert!(matches!(parse_finish_reason("stop"), FinishReason::Stop));
    }

    #[test]
    fn test_parse_finish_reason_end_turn() {
        assert!(matches!(parse_finish_reason("end_turn"), FinishReason::Stop));
    }

    #[test]
    fn test_parse_finish_reason_max_tokens() {
        assert!(matches!(parse_finish_reason("max_tokens"), FinishReason::Length));
    }

    #[test]
    fn test_parse_finish_reason_tool_use() {
        assert!(matches!(parse_finish_reason("tool_use"), FinishReason::ToolCalls));
    }

    #[test]
    fn test_parse_finish_reason_unknown() {
        match parse_finish_reason("something_else") {
            FinishReason::Other(s) => assert_eq!(s, "something_else"),
            _ => panic!("Expected FinishReason::Other"),
        }
    }

    // ToolCallAccumulator tests
    #[test]
    fn test_tool_call_accumulator_new() {
        let acc = ToolCallAccumulator::new("id_123".to_string(), "my_tool".to_string());
        assert_eq!(acc.id, "id_123");
        assert_eq!(acc.name, "my_tool");
        assert!(acc.arguments.is_empty());
        assert!(!acc.is_complete);
    }

    #[test]
    fn test_tool_call_accumulator_add_arguments() {
        let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
        acc.add_arguments("{\"key\":");
        acc.add_arguments("\"value\"}");
        assert_eq!(acc.arguments, "{\"key\":\"value\"}");
    }

    #[test]
    fn test_tool_call_accumulator_try_complete_incomplete_json() {
        let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
        acc.add_arguments("{\"incomplete\":");
        assert!(acc.try_complete().is_none());
        assert!(!acc.is_complete);
    }

    #[test]
    fn test_tool_call_accumulator_try_complete_valid_json() {
        let mut acc = ToolCallAccumulator::new("id_1".to_string(), "read".to_string());
        acc.add_arguments("{\"path\": \"/tmp/test.txt\"}");
        let result = acc.try_complete();
        assert!(result.is_some());
        let tool_call = result.unwrap();
        assert_eq!(tool_call.id, "id_1");
        assert_eq!(tool_call.name, "read");
        assert!(acc.is_complete);
    }

    #[test]
    fn test_tool_call_accumulator_force_complete_valid_json() {
        let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
        acc.add_arguments("{\"a\": 1}");
        let result = acc.force_complete();
        assert_eq!(result.arguments["a"], 1);
        assert!(acc.is_complete);
    }

    #[test]
    fn test_tool_call_accumulator_force_complete_invalid_json() {
        let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
        acc.add_arguments("not valid json");
        let result = acc.force_complete();
        assert_eq!(result.arguments["raw"], "not valid json");
    }

    #[test]
    fn test_tool_call_accumulator_force_complete_empty() {
        let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
        let result = acc.force_complete();
        assert_eq!(result.arguments, serde_json::json!({}));
    }

    #[test]
    fn test_tool_call_accumulator_try_complete_empty_returns_none() {
        let mut acc = ToolCallAccumulator::new("id".to_string(), "tool".to_string());
        assert!(acc.try_complete().is_none());
    }

    // ServerToolAccumulator tests
    #[test]
    fn test_server_tool_accumulator_new() {
        let acc = ServerToolAccumulator::new("st_123".to_string(), "web_search".to_string());
        assert_eq!(acc.id, "st_123");
        assert_eq!(acc.name, "web_search");
        assert!(acc.input_json.is_empty());
        assert!(!acc.is_complete);
    }

    #[test]
    fn test_server_tool_accumulator_add_input() {
        let mut acc = ServerToolAccumulator::new("id".to_string(), "web_search".to_string());
        acc.add_input("{\"query\":");
        acc.add_input("\"rust async\"}");
        assert_eq!(acc.input_json, "{\"query\":\"rust async\"}");
    }

    #[test]
    fn test_server_tool_accumulator_complete_valid_json() {
        let mut acc = ServerToolAccumulator::new("id".to_string(), "web_search".to_string());
        acc.add_input("{\"query\": \"test\"}");
        let result = acc.complete();
        assert_eq!(result["query"], "test");
        assert!(acc.is_complete);
    }

    #[test]
    fn test_server_tool_accumulator_complete_invalid_json() {
        let mut acc = ServerToolAccumulator::new("id".to_string(), "tool".to_string());
        acc.add_input("malformed {json");
        let result = acc.complete();
        assert_eq!(result["raw"], "malformed {json");
    }

    #[test]
    fn test_server_tool_accumulator_complete_empty() {
        let mut acc = ServerToolAccumulator::new("id".to_string(), "tool".to_string());
        let result = acc.complete();
        assert_eq!(result, serde_json::json!({}));
    }

    // ThinkingAccumulator tests
    #[test]
    fn test_thinking_accumulator_new() {
        let acc = ThinkingAccumulator::new();
        assert!(acc.thinking.is_empty());
        assert!(acc.signature.is_empty());
        assert!(!acc.is_complete);
    }

    #[test]
    fn test_thinking_accumulator_default() {
        let acc = ThinkingAccumulator::default();
        assert!(acc.thinking.is_empty());
    }

    #[test]
    fn test_thinking_accumulator_add_thinking() {
        let mut acc = ThinkingAccumulator::new();
        acc.add_thinking("Let me think about ");
        acc.add_thinking("this problem...");
        assert_eq!(acc.thinking, "Let me think about this problem...");
    }

    #[test]
    fn test_thinking_accumulator_add_signature() {
        let mut acc = ThinkingAccumulator::new();
        acc.add_signature("sig_part1");
        acc.add_signature("_part2");
        assert_eq!(acc.signature, "sig_part1_part2");
    }

    #[test]
    fn test_thinking_accumulator_complete() {
        let mut acc = ThinkingAccumulator::new();
        acc.add_thinking("My analysis is...");
        acc.add_signature("abcdef123");
        let (thinking, signature) = acc.complete();
        assert_eq!(thinking, "My analysis is...");
        assert_eq!(signature, "abcdef123");
        assert!(acc.is_complete);
    }

    // SseStreamProcessor tests
    #[tokio::test]
    async fn test_sse_processor_done_marker() {
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
        let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
        let mut processor = SseStreamProcessor::new(tx, buffer_tx);

        struct MockParser;
        #[async_trait::async_trait]
        impl SseParser for MockParser {
            async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
                Ok(SseEvent::Skip)
            }
        }

        processor.process_sse_data("[DONE]", &MockParser).await.unwrap();
        let part = rx.recv().await.unwrap();
        assert!(matches!(part, StreamPart::Finish { reason: FinishReason::Stop }));
    }

    #[tokio::test]
    async fn test_sse_processor_text_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
        let (buffer_tx, mut buffer_rx) = mpsc::unbounded_channel::<String>();
        let mut processor = SseStreamProcessor::new(tx, buffer_tx);

        struct TextDeltaParser;
        #[async_trait::async_trait]
        impl SseParser for TextDeltaParser {
            async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
                // Send more than 64 chars to trigger immediate buffer flush
                Ok(SseEvent::TextDelta(
                    "This is a longer text that exceeds the buffer chunk size of 64 characters to ensure immediate flushing.".to_string()
                ))
            }
        }

        processor.process_sse_data("{}", &TextDeltaParser).await.unwrap();
        let text = buffer_rx.recv().await.unwrap();
        assert!(!text.is_empty());
        processor.finish().await;
        drop(processor);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_sse_processor_tool_call_start() {
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
        let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
        let mut processor = SseStreamProcessor::new(tx, buffer_tx);

        struct ToolStartParser;
        #[async_trait::async_trait]
        impl SseParser for ToolStartParser {
            async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
                Ok(SseEvent::ToolCallStart {
                    id: "tool_123".to_string(),
                    name: "read".to_string(),
                })
            }
        }

        processor.process_sse_data("{}", &ToolStartParser).await.unwrap();
        let part = rx.recv().await.unwrap();
        match part {
            StreamPart::ToolCallStart { id, name } => {
                assert_eq!(id, "tool_123");
                assert_eq!(name, "read");
            }
            _ => panic!("Expected ToolCallStart"),
        }
    }

    #[tokio::test]
    async fn test_sse_processor_skip_empty_json() {
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
        let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
        let mut processor = SseStreamProcessor::new(tx, buffer_tx);

        struct SkipParser;
        #[async_trait::async_trait]
        impl SseParser for SkipParser {
            async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
                Ok(SseEvent::Skip)
            }
        }

        processor.process_sse_data("", &SkipParser).await.unwrap();
        processor.process_sse_data("   ", &SkipParser).await.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_sse_processor_thinking_events() {
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamPart>();
        let (buffer_tx, _buffer_rx) = mpsc::unbounded_channel::<String>();
        let mut processor = SseStreamProcessor::new(tx, buffer_tx);

        struct ThinkingStartParser;
        #[async_trait::async_trait]
        impl SseParser for ThinkingStartParser {
            async fn parse_event(&self, _json: &Value) -> anyhow::Result<SseEvent> {
                Ok(SseEvent::ThinkingStart { index: 0 })
            }
        }

        processor.process_sse_data("{}", &ThinkingStartParser).await.unwrap();
        let part = rx.recv().await.unwrap();
        assert!(matches!(part, StreamPart::ThinkingStart { index: 0 }));
    }
}
