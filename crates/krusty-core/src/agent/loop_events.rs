//! Canonical event protocol for the agentic loop.
//!
//! `LoopEvent` is the single source of truth for everything the orchestrator
//! emits. Transport layers (TUI, HTTP/SSE server) consume these events and
//! map them to their own presentation format.
//!
//! `LoopInput` represents external inputs that the platform provides back to
//! the running orchestrator (tool approvals, user responses, cancellation).

use serde::Serialize;

use crate::ai::types::{Citation, WebFetchContent, WebSearchResult};

/// Events emitted by the agentic orchestrator.
///
/// Each variant represents a discrete state change in the agentic loop.
/// Consumers (TUI, server) map these to their own presentation format.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoopEvent {
    // ── Streaming ──────────────────────────────────────────────────────
    /// Text content delta from AI response.
    TextDelta { delta: String },

    /// Text content delta with web citations.
    TextDeltaWithCitations {
        delta: String,
        citations: Vec<Citation>,
    },

    /// Extended thinking delta.
    ThinkingDelta { thinking: String },

    /// Extended thinking block completed.
    ThinkingComplete { thinking: String, signature: String },

    // ── Tool lifecycle ─────────────────────────────────────────────────
    /// AI is starting to stream a tool call (arguments not yet complete).
    ToolCallStart { id: String, name: String },

    /// Tool call arguments fully received from AI.
    ToolCallComplete {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    /// Tool is being executed.
    ToolExecuting { id: String, name: String },

    /// Streaming output delta from a running tool (e.g. bash output).
    ToolOutputDelta { id: String, delta: String },

    /// Tool execution completed with result.
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },

    // ── Interaction ────────────────────────────────────────────────────
    /// Orchestrator is waiting for user input (AskUser or PlanConfirm).
    AwaitingInput {
        tool_call_id: String,
        tool_name: String,
    },

    /// Tool requires user approval before execution (supervised mode).
    ToolApprovalRequired {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    /// Tool was approved by user.
    ToolApproved { id: String },

    /// Tool was denied by user.
    ToolDenied { id: String },

    // ── Server-side tools (web search/fetch) ──────────────────────────
    /// Server-side tool started (web_search, web_fetch).
    ServerToolStart { id: String, name: String },

    /// Server-side tool completed.
    ServerToolComplete { id: String, name: String },

    /// Web search results received.
    WebSearchResults {
        tool_use_id: String,
        results: Vec<WebSearchResult>,
    },

    /// Web fetch result received.
    WebFetchResult {
        tool_use_id: String,
        content: WebFetchContent,
    },

    /// Server-side tool error.
    ServerToolError {
        tool_use_id: String,
        error_code: String,
    },

    // ── Mode + Plan ────────────────────────────────────────────────────
    /// Work mode changed (build ↔ plan).
    ModeChange {
        mode: String,
        reason: Option<String>,
    },

    /// Plan tasks detected/updated.
    PlanUpdate { tasks: Vec<PlanTaskInfo> },

    /// Plan detected in AI response, awaiting user confirmation.
    PlanComplete {
        tool_call_id: String,
        title: String,
        task_count: usize,
    },

    // ── Turn lifecycle ─────────────────────────────────────────────────
    /// An agentic turn completed.
    TurnComplete { turn: usize, has_more: bool },

    /// Token usage for this turn.
    Usage {
        prompt_tokens: usize,
        completion_tokens: usize,
    },

    /// Session title generated.
    TitleGenerated { title: String },

    /// Agentic loop finished.
    Finished { session_id: String },

    /// Error occurred.
    Error { error: String },
}

/// Simple plan task info for event transport.
#[derive(Debug, Clone, Serialize)]
pub struct PlanTaskInfo {
    pub description: String,
    pub completed: bool,
}

/// External inputs the platform provides back to the orchestrator.
#[derive(Debug, Clone)]
pub enum LoopInput {
    /// User approved or denied a tool execution.
    ToolApproval {
        tool_call_id: String,
        approved: bool,
    },

    /// User responded to an AskUser or PlanConfirm prompt.
    UserResponse {
        tool_call_id: String,
        response: String,
    },

    /// User requested cancellation.
    Cancel,
}
