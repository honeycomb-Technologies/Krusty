use serde_json::Value;

use crate::ai::retry::safe_provider_code;
use crate::ai::types::{
    AiToolCall, Citation, ContextEditingMetrics, FinishReason, Usage, WebFetchContent,
    WebSearchResult,
};

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
        /// Usage info from the API (if provided in finish event)
        usage: Option<Usage>,
    },
    /// Finish with accumulated tool calls (OpenAI format)
    /// Used when finish_reason is "tool_calls" and we have accumulated tool call data
    FinishWithToolCalls {
        tool_calls: Vec<AiToolCall>,
        /// Usage info from the API (if provided in finish event)
        usage: Option<Usage>,
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

    /// Parse a provider frame that contains multiple logical stream events.
    ///
    /// Most SSE formats emit one logical event per frame. Providers such as
    /// Gemini can combine final content, tool calls, usage, and a finish reason
    /// in one frame, so they can override this without complicating the common
    /// stream processor.
    async fn parse_events(&self, json: &Value) -> anyhow::Result<Vec<SseEvent>> {
        Ok(vec![self.parse_event(json).await?])
    }
}

/// Common helper to parse finish reasons
pub fn parse_finish_reason(reason_str: &str) -> FinishReason {
    match reason_str {
        "stop" | "end_turn" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolCalls,
        _ => FinishReason::Other(safe_provider_code(reason_str)),
    }
}
