use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use agent_client_protocol::{McpServer, SessionId};
use dashmap::DashMap;
use tracing::info;

use super::super::error::AcpError;
use super::{SessionState, StorageHandle};
use crate::storage::{SessionType, WorkspaceMode};
use crate::tools::registry::PermissionMode;

/// Manager for all ACP sessions.
pub struct SessionManager {
    /// Active sessions indexed by session ID.
    sessions: DashMap<SessionId, Arc<SessionState>>,
    /// Counter for generating session IDs.
    next_id: AtomicU64,
    /// Optional storage backend for session persistence.
    storage: Option<StorageHandle>,
}

impl SessionManager {
    /// Create a new session manager without storage.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            next_id: AtomicU64::new(1),
            storage: None,
        }
    }

    /// Create a new session manager with storage backend.
    pub fn with_storage(storage: StorageHandle) -> Self {
        Self {
            sessions: DashMap::new(),
            next_id: AtomicU64::new(1),
            storage: Some(storage),
        }
    }

    /// Get reference to storage handle if configured.
    pub fn storage(&self) -> Option<&StorageHandle> {
        self.storage.as_ref()
    }

    /// Create a new session.
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

    /// Create a durable ACP session whose public ACP ID is the canonical storage UUID.
    pub async fn create_persisted_session(
        &self,
        cwd: Option<PathBuf>,
        mcp_servers: Option<Vec<McpServer>>,
    ) -> Result<Arc<SessionState>, AcpError> {
        let Some(storage) = self.storage.as_ref() else {
            return Ok(self.create_session(cwd, mcp_servers));
        };

        let working_dir =
            cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        let working_dir_text = working_dir.to_string_lossy().into_owned();
        let storage_session_id = {
            let storage = storage.lock().await;
            storage
                .create_session_for_user_with_config_and_permission(
                    "ACP Session",
                    None,
                    Some(&working_dir_text),
                    Some(&working_dir_text),
                    WorkspaceMode::Selected,
                    None,
                    None,
                    SessionType::Code,
                    PermissionMode::Supervised,
                )
                .map_err(|error| {
                    AcpError::InternalError(format!(
                        "Failed to create persistent ACP session: {}",
                        error
                    ))
                })?
        };

        let id = SessionId::from(storage_session_id.clone());
        let session = Arc::new(SessionState::with_storage(
            id.clone(),
            Some(working_dir),
            mcp_servers,
            self.storage.clone(),
        ));
        session.link_storage_session(storage_session_id).await;
        self.sessions.insert(id.clone(), Arc::clone(&session));
        info!("Created persistent ACP session: {}", id);
        Ok(session)
    }

    /// Create a session and restore from storage.
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

        let id = SessionId::from(storage_session_id.to_string());
        if self.sessions.contains_key(&id) {
            return Err(AcpError::SessionExists(id.to_string()));
        }
        let session = Arc::new(SessionState::with_storage(
            id.clone(),
            cwd,
            mcp_servers,
            self.storage.clone(),
        ));

        session.load_from_storage(storage_session_id).await?;

        info!(
            "Created session {} from storage session {}",
            id, storage_session_id
        );
        self.sessions.insert(id, Arc::clone(&session));
        Ok(session)
    }

    /// Get an existing session.
    pub fn get_session(&self, id: &SessionId) -> Result<Arc<SessionState>, AcpError> {
        self.sessions
            .get(id)
            .map(|s| Arc::clone(&s))
            .ok_or_else(|| AcpError::SessionNotFound(id.to_string()))
    }

    /// Check if a session exists.
    pub fn has_session(&self, id: &SessionId) -> bool {
        self.sessions.contains_key(id)
    }

    /// Remove a session.
    pub fn remove_session(&self, id: &SessionId) -> Option<Arc<SessionState>> {
        info!("Removing session: {}", id);
        self.sessions.remove(id).map(|(_, s)| s)
    }

    /// Get all session IDs.
    pub fn session_ids(&self) -> Vec<SessionId> {
        self.sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get session count.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Cancel a session.
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
