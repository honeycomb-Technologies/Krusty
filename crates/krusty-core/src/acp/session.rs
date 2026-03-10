//! ACP session management
//!
//! Manages session state for ACP connections. Each session maintains:
//! - Working directory context
//! - MCP server configurations
//! - Conversation history
//! - Cancellation state
//! - Optional persistence to SQLite via storage::SessionManager

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use agent_client_protocol::{McpServer, SessionId};
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use super::error::AcpError;
use crate::ai::types::{ModelMessage, Role};
use crate::storage::{SessionManager as StorageSessionManager, SessionRecoveryState};
use crate::tools::ToolContext;

/// Thread-safe wrapper for storage session manager
///
/// Uses tokio::sync::Mutex to avoid blocking the tokio runtime when
/// acquiring the lock in async contexts.
pub type StorageHandle = Arc<Mutex<StorageSessionManager>>;

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
    /// Storage session ID for persistence (links to SQLite storage)
    storage_session_id: RwLock<Option<String>>,
    /// Reference to storage manager for persisting messages
    storage: Option<StorageHandle>,
    /// Interrupted-turn recovery state loaded from storage, consumed on first resumed prompt.
    recovery_state: RwLock<Option<SessionRecoveryState>>,
}

impl SessionState {
    /// Create a new session state
    pub fn new(id: SessionId, cwd: Option<PathBuf>, mcp_servers: Option<Vec<McpServer>>) -> Self {
        Self::with_storage(id, cwd, mcp_servers, None)
    }

    /// Create a new session state with optional storage backend
    pub fn with_storage(
        id: SessionId,
        cwd: Option<PathBuf>,
        mcp_servers: Option<Vec<McpServer>>,
        storage: Option<StorageHandle>,
    ) -> Self {
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
            storage_session_id: RwLock::new(None),
            storage,
            recovery_state: RwLock::new(None),
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

    /// Add a message to the conversation and persist to storage if available
    pub async fn add_message(&self, message: ModelMessage) {
        self.messages.write().await.push(message.clone());
        self.persist_message(&message).await;
    }

    /// Persist a message to storage (if storage is configured)
    async fn persist_message(&self, message: &ModelMessage) {
        if let Some(ref storage) = self.storage {
            let storage_id = self.storage_session_id.read().await;
            if let Some(ref session_id) = *storage_id {
                let role = match message.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                match serde_json::to_string(&message.content) {
                    Ok(content_json) => {
                        let storage = storage.lock().await;
                        if let Err(e) = storage.save_message(session_id, role, &content_json) {
                            warn!("Failed to persist message to storage: {}", e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to serialize message content: {}", e);
                    }
                }
            }
        }
    }

    /// Initialize storage session (creates a new persistent session)
    pub async fn init_storage_session(&self, title: &str) -> Option<String> {
        if let Some(ref storage) = self.storage {
            let working_dir = self.cwd.to_string_lossy();
            let result = {
                let storage = storage.lock().await;
                storage.create_session(title, None, Some(&working_dir))
            };
            match result {
                Ok(id) => {
                    *self.storage_session_id.write().await = Some(id.clone());
                    info!("Created storage session {} for ACP session {}", id, self.id);
                    Some(id)
                }
                Err(e) => {
                    warn!("Failed to create storage session: {}", e);
                    None
                }
            }
        } else {
            None
        }
    }

    /// Link to an existing storage session
    pub async fn link_storage_session(&self, storage_id: String) {
        *self.storage_session_id.write().await = Some(storage_id);
    }

    /// Load messages from storage into this session
    pub async fn load_from_storage(&self, storage_session_id: &str) -> Result<(), AcpError> {
        let storage = self.storage.as_ref().ok_or_else(|| {
            AcpError::InternalError("No storage configured for session".to_string())
        })?;

        let (raw_messages, recovery_state) = {
            let storage = storage.lock().await;
            let raw_messages = storage
                .load_session_messages(storage_session_id)
                .map_err(|e| {
                    AcpError::InternalError(format!("Failed to load messages from storage: {}", e))
                })?;
            let recovery_state = storage
                .load_recovery_state(storage_session_id)
                .map_err(|e| {
                    AcpError::InternalError(format!(
                        "Failed to load recovery state from storage: {}",
                        e
                    ))
                })?;
            (raw_messages, recovery_state)
        };

        let mut messages = self.messages.write().await;
        messages.clear();

        for (role_str, content_json) in raw_messages {
            let role = match role_str.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => {
                    warn!("Unknown role '{}' in stored message, skipping", role_str);
                    continue;
                }
            };

            match serde_json::from_str(&content_json) {
                Ok(content) => {
                    messages.push(ModelMessage { role, content });
                }
                Err(e) => {
                    warn!("Failed to deserialize message content: {}", e);
                }
            }
        }

        // Link to this storage session
        drop(messages); // Release write lock before acquiring another
        *self.storage_session_id.write().await = Some(storage_session_id.to_string());
        *self.recovery_state.write().await = recovery_state;

        info!(
            "Loaded {} messages from storage session {}",
            self.messages.read().await.len(),
            storage_session_id
        );

        Ok(())
    }

    /// Get the storage session ID if linked
    pub async fn get_storage_session_id(&self) -> Option<String> {
        self.storage_session_id.read().await.clone()
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

    /// Consume any persisted recovery state and convert it into an ephemeral prompt notice.
    pub async fn take_recovery_notice(&self) -> Option<ModelMessage> {
        self.recovery_state
            .write()
            .await
            .take()
            .map(|recovery_state| ModelMessage {
                role: Role::System,
                content: vec![crate::ai::types::Content::Text {
                    text: format!("[RECOVERY NOTICE] {}", recovery_state.notice()),
                }],
            })
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

    /// Add a user message with multiple content blocks
    pub async fn add_user_message_content(&self, content: Vec<crate::ai::types::Content>) {
        use crate::ai::types::Role;
        self.add_message(ModelMessage {
            role: Role::User,
            content,
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

    /// Add system context to the conversation (injected into system prompt)
    /// This is used to provide workspace context to the AI
    pub async fn add_system_context(&self, context: String) {
        use crate::ai::types::{Content, Role};
        self.add_message(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: context }],
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
    /// Optional storage backend for session persistence
    storage: Option<StorageHandle>,
}

impl SessionManager {
    /// Create a new session manager without storage
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            next_id: AtomicU64::new(1),
            storage: None,
        }
    }

    /// Create a new session manager with storage backend
    pub fn with_storage(storage: StorageHandle) -> Self {
        Self {
            sessions: DashMap::new(),
            next_id: AtomicU64::new(1),
            storage: Some(storage),
        }
    }

    /// Get reference to storage handle if configured
    pub fn storage(&self) -> Option<&StorageHandle> {
        self.storage.as_ref()
    }

    /// Create a new session
    pub fn create_session(
        &self,
        cwd: Option<PathBuf>,
        mcp_servers: Option<Vec<McpServer>>,
    ) -> Arc<SessionState> {
        let id = SessionId::from(self.next_id.fetch_add(1, Ordering::SeqCst).to_string());
        let session = Arc::new(SessionState::with_storage(
            id.clone(),
            cwd,
            mcp_servers,
            self.storage.clone(),
        ));

        info!("Created new session: {}", id);
        self.sessions.insert(id, Arc::clone(&session));

        session
    }

    /// Create a session and restore from storage
    pub async fn create_session_from_storage(
        &self,
        storage_session_id: &str,
        cwd: Option<PathBuf>,
        mcp_servers: Option<Vec<McpServer>>,
    ) -> Result<Arc<SessionState>, AcpError> {
        if self.storage.is_none() {
            return Err(AcpError::InternalError(
                "No storage configured for session manager".to_string(),
            ));
        }

        let id = SessionId::from(self.next_id.fetch_add(1, Ordering::SeqCst).to_string());
        let session = Arc::new(SessionState::with_storage(
            id.clone(),
            cwd,
            mcp_servers,
            self.storage.clone(),
        ));

        // Load messages from storage
        session.load_from_storage(storage_session_id).await?;

        info!(
            "Created session {} from storage session {}",
            id, storage_session_id
        );
        self.sessions.insert(id, Arc::clone(&session));

        Ok(session)
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

    #[tokio::test]
    async fn test_session_with_storage() {
        use crate::storage::Database;
        use tempfile::tempdir;
        use tokio::sync::Mutex;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::new(&db_path).unwrap();
        let storage = Arc::new(Mutex::new(StorageSessionManager::new(db)));

        let manager = SessionManager::with_storage(storage);
        let session = manager.create_session(Some(PathBuf::from("/test")), None);

        // Initialize storage session
        let storage_id = session.init_storage_session("Test Session").await;
        assert!(storage_id.is_some());

        // Add a message
        session.add_user_message("Hello, world!".to_string()).await;

        // Verify storage session ID is set
        let stored_id = session.get_storage_session_id().await;
        assert_eq!(stored_id, storage_id);
    }

    #[tokio::test]
    async fn test_session_load_from_storage() {
        use crate::storage::Database;
        use crate::storage::{
            PartialAssistantState, RecoveryDecision, RecoveryStatus, SessionRecoveryState,
        };
        use tempfile::tempdir;
        use tokio::sync::Mutex;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::new(&db_path).unwrap();
        let storage = Arc::new(Mutex::new(StorageSessionManager::new(db)));

        // First, create a session and add messages
        let manager = SessionManager::with_storage(Arc::clone(&storage));
        let session1 = manager.create_session(Some(PathBuf::from("/test")), None);
        let storage_id = session1.init_storage_session("Test Session").await.unwrap();
        session1.add_user_message("First message".to_string()).await;
        session1.add_assistant_message("Response".to_string()).await;
        {
            let storage = storage.lock().await;
            storage
                .update_recovery_state(
                    &storage_id,
                    &SessionRecoveryState::new(
                        RecoveryStatus::Interrupted,
                        None,
                        Some("stream stopped".to_string()),
                        PartialAssistantState {
                            text: "Partial answer".to_string(),
                            thinking: String::new(),
                            tool_calls: Vec::new(),
                        },
                        RecoveryDecision::Resumable {
                            latest_user_objective: "Finish the answer".to_string(),
                        },
                    ),
                )
                .unwrap();
        }

        // Create a new session from storage
        let session2 = manager
            .create_session_from_storage(&storage_id, Some(PathBuf::from("/test")), None)
            .await
            .unwrap();

        // Verify messages were loaded
        let messages = session2.get_messages().await;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);

        let recovery_notice = session2
            .take_recovery_notice()
            .await
            .expect("expected recovery notice");
        assert_eq!(recovery_notice.role, Role::System);
        assert!(recovery_notice.content.iter().any(|content| matches!(
            content,
            crate::ai::types::Content::Text { text }
                if text.contains("Finish the answer")
        )));
    }
}
