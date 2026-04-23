/// Agent execution state
#[derive(Debug, Clone)]
pub struct AgentState {
    /// Current state: "idle", "streaming", "tool_executing", "awaiting_input", "error"
    pub state: String,
    /// When the agent started processing
    pub started_at: Option<String>,
    /// Last event timestamp
    pub last_event_at: Option<String>,
}
