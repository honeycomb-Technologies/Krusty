//! Chat state management
//!
//! Groups conversation and streaming-related state.

use crate::ai::types::ModelMessage;

/// Chat and conversation state
///
/// Groups fields related to the conversation, messages, and streaming status.
#[derive(Default)]
pub struct ChatState {
    /// Display messages (role, content) for rendering
    pub messages: Vec<(String, String)>,
    /// Full conversation history for API
    pub conversation: Vec<ModelMessage>,
    /// True while streaming response from AI API
    pub is_streaming: bool,
    /// True while tools are executing
    pub is_executing_tools: bool,
    /// Current activity description for status display
    pub current_activity: Option<String>,
    /// Cache for streaming assistant message index (avoids O(n) scan per delta)
    pub streaming_assistant_idx: Option<usize>,
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start streaming from AI - sets is_streaming flag and clears caches
    pub fn start_streaming(&mut self) {
        self.is_streaming = true;
        self.current_activity = Some("thinking".to_string());
        self.streaming_assistant_idx = None;
    }

    /// Stop streaming from AI - clears is_streaming flag and related caches
    pub fn stop_streaming(&mut self) {
        self.is_streaming = false;
        self.current_activity = None;
        self.streaming_assistant_idx = None;
    }

    /// Start tool execution - sets is_executing_tools flag
    pub fn start_tool_execution(&mut self) {
        self.is_executing_tools = true;
    }

    /// Stop tool execution - clears is_executing_tools flag
    pub fn stop_tool_execution(&mut self) {
        self.is_executing_tools = false;
        self.current_activity = None;
    }

    /// Check if busy (streaming OR executing tools)
    pub fn is_busy(&self) -> bool {
        self.is_streaming || self.is_executing_tools
    }
}
