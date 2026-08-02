//! Streaming types for AI responses

use serde::{Deserialize, Serialize};

use super::types::{
    AiToolCall, Citation, ContextEditingMetrics, FinishReason, Usage, WebFetchContent,
    WebSearchResult,
};

/// Parts that can be streamed from a model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamPart {
    /// Start of stream
    #[serde(rename = "start")]
    Start { model: String, provider: String },

    /// Text delta
    #[serde(rename = "text_delta")]
    TextDelta { delta: String },

    /// Text delta with citations
    #[serde(rename = "text_delta_with_citations")]
    TextDeltaWithCitations {
        delta: String,
        citations: Vec<Citation>,
    },

    /// Tool call start (client-side tools)
    #[serde(rename = "tool_call_start")]
    ToolCallStart { id: String, name: String },

    /// Tool call arguments delta
    #[serde(rename = "tool_call_delta")]
    ToolCallDelta { id: String, delta: String },

    /// Tool call complete
    #[serde(rename = "tool_call_complete")]
    ToolCallComplete { tool_call: AiToolCall },

    /// Server tool use start (web_search, web_fetch)
    #[serde(rename = "server_tool_start")]
    ServerToolStart { id: String, name: String },

    /// Server tool use delta (query/url being streamed)
    #[serde(rename = "server_tool_delta")]
    ServerToolDelta { id: String, delta: String },

    /// Server tool use complete
    #[serde(rename = "server_tool_complete")]
    ServerToolComplete {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// Web search results received
    #[serde(rename = "web_search_results")]
    WebSearchResults {
        tool_use_id: String,
        results: Vec<WebSearchResult>,
    },

    /// Web fetch result received
    #[serde(rename = "web_fetch_result")]
    WebFetchResult {
        tool_use_id: String,
        content: WebFetchContent,
    },

    /// Web search/fetch error
    #[serde(rename = "server_tool_error")]
    ServerToolError {
        tool_use_id: String,
        error_code: String,
    },

    /// Thinking block start (extended thinking)
    #[serde(rename = "thinking_start")]
    ThinkingStart { index: usize },

    /// Thinking content delta
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { index: usize, thinking: String },

    /// Signature delta (required for thinking block integrity)
    #[serde(rename = "signature_delta")]
    SignatureDelta { index: usize, signature: String },

    /// Thinking block complete
    #[serde(rename = "thinking_complete")]
    ThinkingComplete {
        index: usize,
        thinking: String,
        signature: String,
    },

    /// Usage information
    #[serde(rename = "usage")]
    Usage { usage: Usage },

    /// Finish
    #[serde(rename = "finish")]
    Finish { reason: FinishReason },

    /// Error
    #[serde(rename = "error")]
    Error { error: String },

    /// Context was edited (old thinking/tools cleared)
    #[serde(rename = "context_edited")]
    ContextEdited { metrics: ContextEditingMetrics },
}
