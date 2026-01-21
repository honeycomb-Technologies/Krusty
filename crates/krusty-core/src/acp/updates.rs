//! ACP session update streaming
//!
//! Handles streaming of session updates to the client during prompt processing.

use agent_client_protocol::{
    ContentBlock, ContentChunk, Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus, SessionId,
    SessionUpdate, TextContent, ToolCallUpdate,
};
use tokio::sync::mpsc;
use tracing::error;

/// Update sender for streaming session updates
#[allow(dead_code)]
pub struct UpdateSender {
    session_id: SessionId,
    tx: mpsc::UnboundedSender<SessionUpdate>,
}

#[allow(dead_code)]
impl UpdateSender {
    /// Create a new update sender
    pub fn new(session_id: SessionId, tx: mpsc::UnboundedSender<SessionUpdate>) -> Self {
        Self { session_id, tx }
    }

    /// Send an agent message chunk (streaming model output)
    pub fn send_agent_text(&self, text: &str) {
        let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text)));
        self.send(SessionUpdate::AgentMessageChunk(chunk));
    }

    /// Send a thought/reasoning chunk
    pub fn send_thought(&self, text: &str) {
        let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text)));
        self.send(SessionUpdate::AgentThoughtChunk(chunk));
    }

    /// Send a tool call update
    pub fn send_tool_call(&self, update: ToolCallUpdate) {
        self.send(SessionUpdate::ToolCallUpdate(update));
    }

    /// Send a plan update
    pub fn send_plan(&self, entries: Vec<PlanEntry>) {
        let plan = Plan::new(entries);
        self.send(SessionUpdate::Plan(plan));
    }

    /// Send a raw session update
    pub fn send(&self, update: SessionUpdate) {
        if let Err(e) = self.tx.send(update) {
            error!("Failed to send session update: {}", e);
        }
    }

    /// Get the session ID
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

/// Get a string describing the update type (for logging)
#[allow(dead_code)]
fn update_type(update: &SessionUpdate) -> &'static str {
    match update {
        SessionUpdate::UserMessageChunk(_) => "UserMessageChunk",
        SessionUpdate::AgentMessageChunk(_) => "AgentMessageChunk",
        SessionUpdate::AgentThoughtChunk(_) => "AgentThoughtChunk",
        SessionUpdate::ToolCall(_) => "ToolCall",
        SessionUpdate::ToolCallUpdate(_) => "ToolCallUpdate",
        SessionUpdate::Plan(_) => "Plan",
        SessionUpdate::AvailableCommandsUpdate(_) => "AvailableCommandsUpdate",
        SessionUpdate::CurrentModeUpdate(_) => "CurrentModeUpdate",
        _ => "Unknown",
    }
}

/// Convert Krusty plan items to ACP plan entries
#[allow(dead_code)]
pub fn plan_items_to_entries(items: &[(String, bool)]) -> Vec<PlanEntry> {
    items
        .iter()
        .map(|(content, completed)| {
            PlanEntry::new(
                content.clone(),
                PlanEntryPriority::Medium,
                if *completed {
                    PlanEntryStatus::Completed
                } else {
                    PlanEntryStatus::Pending
                },
            )
        })
        .collect()
}

/// Create a plan entry
#[allow(dead_code)]
pub fn create_plan_entry(content: &str, status: PlanEntryStatus) -> PlanEntry {
    PlanEntry::new(content.to_string(), PlanEntryPriority::Medium, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_conversion() {
        let items = vec![("Task 1".to_string(), true), ("Task 2".to_string(), false)];

        let entries = plan_items_to_entries(&items);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "Task 1");
        assert!(matches!(entries[0].status, PlanEntryStatus::Completed));
        assert_eq!(entries[1].content, "Task 2");
        assert!(matches!(entries[1].status, PlanEntryStatus::Pending));
    }
}
