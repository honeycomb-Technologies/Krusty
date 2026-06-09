use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use agent_client_protocol::{McpServer, SessionId};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use crate::ai::types::{ModelMessage, Role};
use crate::storage::{SessionManager as StorageSessionManager, SessionRecoveryState};
use crate::tools::registry::PermissionMode;
use crate::tools::{FileObservationTracker, ToolContext};

use super::super::error::AcpError;

/// Thread-safe wrapper for storage session manager.
pub type StorageHandle = Arc<Mutex<StorageSessionManager>>;

/// Session state for a single ACP session.
pub struct SessionState {
    /// Session identifier.
    pub id: SessionId,
    /// Working directory for this session.
    pub cwd: PathBuf,
    /// MCP server configurations passed by the client.
    pub mcp_servers: Vec<McpServer>,
    /// Current session mode (e.g., "code", "architect", "ask").
    pub mode: RwLock<Option<String>>,
    /// Conversation messages.
    pub messages: RwLock<Vec<ModelMessage>>,
    /// Whether this session has been cancelled.
    cancelled: AtomicBool,
    /// Tool context for this session.
    pub tool_context: RwLock<Option<ToolContext>>,
    /// File observations shared across tool calls in this ACP session.
    pub file_observations: Arc<FileObservationTracker>,
    /// Storage session ID for persistence (links to SQLite storage).
    storage_session_id: RwLock<Option<String>>,
    /// Reference to storage manager for persisting messages.
    storage: Option<StorageHandle>,
    /// Interrupted-turn recovery state loaded from storage.
    recovery_state: RwLock<Option<SessionRecoveryState>>,
}

impl SessionState {
    /// Create a new session state.
    pub fn new(id: SessionId, cwd: Option<PathBuf>, mcp_servers: Option<Vec<McpServer>>) -> Self {
        Self::with_storage(id, cwd, mcp_servers, None)
    }

    /// Create a new session state with optional storage backend.
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
            file_observations: Arc::new(FileObservationTracker::default()),
            storage_session_id: RwLock::new(None),
            storage,
            recovery_state: RwLock::new(None),
        }
    }

    /// Cancel this session.
    pub fn cancel(&self) {
        debug!("Cancelling session {}", self.id);
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if session is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Reset cancellation state (for new prompts).
    pub fn reset_cancellation(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Set the session mode.
    pub async fn set_mode(&self, mode: Option<String>) {
        *self.mode.write().await = mode;
    }

    /// Get the current mode.
    pub async fn get_mode(&self) -> Option<String> {
        self.mode.read().await.clone()
    }

    /// Add a message to the conversation and persist to storage if available.
    pub async fn add_message(&self, message: ModelMessage) {
        self.messages.write().await.push(message.clone());
        self.persist_message(&message).await;
    }

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

    /// Initialize storage session (creates a new persistent session).
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

    /// Link to an existing storage session.
    pub async fn link_storage_session(&self, storage_id: String) {
        *self.storage_session_id.write().await = Some(storage_id);
    }

    /// Load messages from storage into this session.
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
                Ok(content) => messages.push(ModelMessage { role, content }),
                Err(e) => warn!("Failed to deserialize message content: {}", e),
            }
        }

        drop(messages);
        *self.storage_session_id.write().await = Some(storage_session_id.to_string());
        *self.recovery_state.write().await = recovery_state;

        info!(
            "Loaded {} messages from storage session {}",
            self.messages.read().await.len(),
            storage_session_id
        );

        Ok(())
    }

    /// Get the storage session ID if linked.
    pub async fn get_storage_session_id(&self) -> Option<String> {
        self.storage_session_id.read().await.clone()
    }

    /// Get the current persisted permission mode for this session.
    pub async fn permission_mode(&self) -> PermissionMode {
        let Some(ref storage) = self.storage else {
            return PermissionMode::default();
        };
        let Some(session_id) = self.storage_session_id.read().await.clone() else {
            return PermissionMode::default();
        };

        let storage = storage.lock().await;
        storage
            .get_session(&session_id)
            .ok()
            .flatten()
            .map(|session| session.permission_mode)
            .unwrap_or_default()
    }

    /// Get all messages.
    pub async fn get_messages(&self) -> Vec<ModelMessage> {
        self.messages.read().await.clone()
    }

    /// Clear messages (for session reset).
    pub async fn clear_messages(&self) {
        self.messages.write().await.clear();
    }

    /// Get conversation history (alias for get_messages).
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

    /// Add a user message to the conversation.
    pub async fn add_user_message(&self, text: String) {
        use crate::ai::types::Content;
        self.add_message(ModelMessage {
            role: Role::User,
            content: vec![Content::Text { text }],
        })
        .await;
    }

    /// Add a user message with multiple content blocks.
    pub async fn add_user_message_content(&self, content: Vec<crate::ai::types::Content>) {
        self.add_message(ModelMessage {
            role: Role::User,
            content,
        })
        .await;
    }

    /// Add an assistant message to the conversation.
    pub async fn add_assistant_message(&self, text: String) {
        use crate::ai::types::Content;
        self.add_message(ModelMessage {
            role: Role::Assistant,
            content: vec![Content::Text { text }],
        })
        .await;
    }

    /// Add a tool call to the conversation history.
    pub async fn add_tool_call(&self, id: String, name: String, input: serde_json::Value) {
        use crate::ai::types::Content;
        self.add_message(ModelMessage {
            role: Role::Assistant,
            content: vec![Content::ToolUse { id, name, input }],
        })
        .await;
    }

    /// Add a tool result to the conversation history.
    pub async fn add_tool_result(&self, tool_use_id: &str, output: String, is_error: bool) {
        use crate::ai::types::Content;
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

    /// Add system context to the conversation.
    pub async fn add_system_context(&self, context: String) {
        use crate::ai::types::Content;
        self.add_message(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: context }],
        })
        .await;
    }
}
