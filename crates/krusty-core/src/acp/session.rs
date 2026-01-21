//! ACP session management
//!
//! Manages session state for ACP connections. Each session maintains:
//! - Working directory context
//! - MCP server configurations
//! - Conversation history
//! - Cancellation state

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use agent_client_protocol::{McpServer, SessionId};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tracing::{debug, info};

use super::error::AcpError;
use crate::ai::types::ModelMessage;
use crate::tools::ToolContext;

/// Session state for a single ACP session
pub struct SessionState {
    /// Session identifier
    pub id: SessionId,
    /// Working directory for this session
    pub cwd: PathBuf,
    /// MCP server configurations passed by the client
    pub mcp_servers: Vec<McpServer>,
    /// Current session mode (e.g., "code", "architect", "ask")
    pub mode: RwLock<Option<String>>,
    /// Conversation messages
    pub messages: RwLock<Vec<ModelMessage>>,
    /// Whether this session has been cancelled
    cancelled: AtomicBool,
    /// Tool context for this session
    pub tool_context: RwLock<Option<ToolContext>>,
}

impl SessionState {
    /// Create a new session state
    pub fn new(id: SessionId, cwd: Option<PathBuf>, mcp_servers: Option<Vec<McpServer>>) -> Self {
        let working_dir =
            cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));

        debug!("Creating session {} with cwd: {:?}", id, working_dir);

        Self {
            id,
            cwd: working_dir,
            mcp_servers: mcp_servers.unwrap_or_default(),
            mode: RwLock::new(None),
            messages: RwLock::new(Vec::new()),
            cancelled: AtomicBool::new(false),
            tool_context: RwLock::new(None),
        }
    }

    /// Cancel this session
    pub fn cancel(&self) {
        debug!("Cancelling session {}", self.id);
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if session is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Reset cancellation state (for new prompts)
    pub fn reset_cancellation(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Set the session mode
    pub async fn set_mode(&self, mode: Option<String>) {
        *self.mode.write().await = mode;
    }

    /// Get the current mode
    pub async fn get_mode(&self) -> Option<String> {
        self.mode.read().await.clone()
    }

    /// Add a message to the conversation
    pub async fn add_message(&self, message: ModelMessage) {
        self.messages.write().await.push(message);
    }

    /// Get all messages
    pub async fn get_messages(&self) -> Vec<ModelMessage> {
        self.messages.read().await.clone()
    }

    /// Clear messages (for session reset)
    pub async fn clear_messages(&self) {
        self.messages.write().await.clear();
    }

    /// Get conversation history (alias for get_messages)
    pub async fn history(&self) -> Vec<ModelMessage> {
        self.get_messages().await
    }

    /// Add a user message to the conversation
    pub async fn add_user_message(&self, text: String) {
        use crate::ai::types::{Content, Role};
        self.add_message(ModelMessage {
            role: Role::User,
            content: vec![Content::Text { text }],
        })
        .await;
    }

    /// Add an assistant message to the conversation
    pub async fn add_assistant_message(&self, text: String) {
        use crate::ai::types::{Content, Role};
        self.add_message(ModelMessage {
            role: Role::Assistant,
            content: vec![Content::Text { text }],
        })
        .await;
    }

    /// Add a tool call to the conversation history
    pub async fn add_tool_call(&self, id: String, name: String, input: serde_json::Value) {
        use crate::ai::types::{Content, Role};
        self.add_message(ModelMessage {
            role: Role::Assistant,
            content: vec![Content::ToolUse { id, name, input }],
        })
        .await;
    }

    /// Add a tool result to the conversation history
    pub async fn add_tool_result(&self, tool_use_id: &str, output: String, is_error: bool) {
        use crate::ai::types::{Content, Role};
        self.add_message(ModelMessage {
            role: Role::Tool,
            content: vec![Content::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                output: serde_json::Value::String(output),
                is_error: if is_error { Some(true) } else { None },
            }],
        })
        .await;
    }
}

/// Manager for all ACP sessions
pub struct SessionManager {
    /// Active sessions indexed by session ID
    sessions: DashMap<SessionId, Arc<SessionState>>,
    /// Counter for generating session IDs
    next_id: AtomicU64,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a new session
    pub fn create_session(
        &self,
        cwd: Option<PathBuf>,
        mcp_servers: Option<Vec<McpServer>>,
    ) -> Arc<SessionState> {
        let id = SessionId::from(self.next_id.fetch_add(1, Ordering::SeqCst).to_string());
        let session = Arc::new(SessionState::new(id.clone(), cwd, mcp_servers));

        info!("Created new session: {}", id);
        self.sessions.insert(id, Arc::clone(&session));

        session
    }

    /// Get an existing session
    pub fn get_session(&self, id: &SessionId) -> Result<Arc<SessionState>, AcpError> {
        self.sessions
            .get(id)
            .map(|s| Arc::clone(&s))
            .ok_or_else(|| AcpError::SessionNotFound(id.to_string()))
    }

    /// Check if a session exists
    pub fn has_session(&self, id: &SessionId) -> bool {
        self.sessions.contains_key(id)
    }

    /// Remove a session
    pub fn remove_session(&self, id: &SessionId) -> Option<Arc<SessionState>> {
        info!("Removing session: {}", id);
        self.sessions.remove(id).map(|(_, s)| s)
    }

    /// Get all session IDs
    pub fn session_ids(&self) -> Vec<SessionId> {
        self.sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get session count
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Cancel a session
    pub fn cancel_session(&self, id: &SessionId) -> Result<(), AcpError> {
        let session = self.get_session(id)?;
        session.cancel();
        Ok(())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let manager = SessionManager::new();
        let session = manager.create_session(Some(PathBuf::from("/tmp")), None);

        assert_eq!(session.cwd, PathBuf::from("/tmp"));
        assert!(!session.is_cancelled());
        assert_eq!(manager.session_count(), 1);
    }

    #[test]
    fn test_session_cancellation() {
        let manager = SessionManager::new();
        let session = manager.create_session(None, None);

        assert!(!session.is_cancelled());
        session.cancel();
        assert!(session.is_cancelled());
    }

    #[test]
    fn test_session_lookup() {
        let manager = SessionManager::new();
        let session = manager.create_session(None, None);
        let id = session.id.clone();

        assert!(manager.has_session(&id));
        assert!(manager.get_session(&id).is_ok());

        let fake_id = SessionId::from("nonexistent".to_string());
        assert!(!manager.has_session(&fake_id));
        assert!(manager.get_session(&fake_id).is_err());
    }
}
