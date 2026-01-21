//! KrustyAgent - ACP Agent trait implementation
//!
//! This is the core ACP agent that handles all protocol methods.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    Agent, AgentCapabilities, AuthenticateRequest, AuthenticateResponse, CancelNotification,
    ClientCapabilities, ContentBlock, Error as AcpSchemaError, ExtNotification, ExtRequest,
    ExtResponse, Implementation, InitializeRequest, InitializeResponse, LoadSessionRequest,
    LoadSessionResponse, McpCapabilities, NewSessionRequest, NewSessionResponse,
    PromptCapabilities, PromptRequest, PromptResponse, Result as AcpResult, SessionCapabilities,
    SessionId, SessionNotification, SetSessionModeRequest, SetSessionModeResponse,
};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use super::bridge::NotificationBridge;
use super::error::AcpError;
use super::processor::PromptProcessor;
use super::session::{SessionManager, SessionState};
use crate::ai::providers::ProviderId;
use crate::tools::ToolRegistry;

/// ACP protocol version supported by this agent (10 is current)
#[allow(dead_code)]
pub const PROTOCOL_VERSION_NUM: u16 = 10;

/// Krusty's ACP Agent implementation
pub struct KrustyAgent {
    /// Session manager
    sessions: Arc<SessionManager>,
    /// Tool registry
    tools: Arc<ToolRegistry>,
    /// Client capabilities (received during init)
    client_capabilities: RwLock<Option<ClientCapabilities>>,
    /// Authenticated API key
    api_key: RwLock<Option<String>>,
    /// Prompt processor for AI integration
    processor: RwLock<PromptProcessor>,
    /// Channel for sending notifications to the connection
    notification_tx: RwLock<Option<mpsc::UnboundedSender<SessionNotification>>>,
    /// Working directory (reserved for future use)
    #[allow(dead_code)]
    cwd: PathBuf,
}

impl KrustyAgent {
    /// Create a new Krusty ACP agent
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let tools = Arc::new(ToolRegistry::new());
        Self {
            sessions: Arc::new(SessionManager::new()),
            tools: tools.clone(),
            client_capabilities: RwLock::new(None),
            api_key: RwLock::new(None),
            processor: RwLock::new(PromptProcessor::new(tools, cwd.clone())),
            notification_tx: RwLock::new(None),
            cwd,
        }
    }

    /// Create with custom tool registry
    pub fn with_tools(tools: Arc<ToolRegistry>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            sessions: Arc::new(SessionManager::new()),
            tools: tools.clone(),
            client_capabilities: RwLock::new(None),
            api_key: RwLock::new(None),
            processor: RwLock::new(PromptProcessor::new(tools, cwd.clone())),
            notification_tx: RwLock::new(None),
            cwd,
        }
    }

    /// Set the notification channel sender
    pub async fn set_notification_channel(&self, tx: mpsc::UnboundedSender<SessionNotification>) {
        *self.notification_tx.write().await = Some(tx);
    }

    /// Initialize the AI client with an API key
    pub async fn init_ai_client(&self, api_key: String, provider: ProviderId) {
        self.processor.write().await.init_ai_client(api_key, provider, None);
    }

    /// Initialize the AI client with an API key and optional model override
    pub async fn init_ai_client_with_model(&self, api_key: String, provider: ProviderId, model: Option<String>) {
        self.processor.write().await.init_ai_client(api_key, provider, model);
    }

    /// Get agent capabilities to advertise
    fn agent_capabilities(&self) -> AgentCapabilities {
        let mut caps = AgentCapabilities::new();

        // Prompt capabilities
        let mut prompt_caps = PromptCapabilities::new();
        prompt_caps.image = false;
        prompt_caps.audio = false;
        prompt_caps.embedded_context = true;
        caps.prompt_capabilities = prompt_caps;

        // Session capabilities
        caps.load_session = true;
        caps.session_capabilities = SessionCapabilities::new();

        // MCP capabilities
        let mut mcp_caps = McpCapabilities::new();
        mcp_caps.http = false;
        mcp_caps.sse = false;
        caps.mcp_capabilities = mcp_caps;

        caps
    }

    /// Get agent implementation info
    fn agent_info(&self) -> Implementation {
        Implementation::new("krusty", env!("CARGO_PKG_VERSION"))
    }

    /// Get a session by ID
    pub fn get_session(&self, id: &SessionId) -> Result<Arc<SessionState>, AcpError> {
        self.sessions.get_session(id)
    }

    /// Get the session manager
    pub fn sessions(&self) -> &SessionManager {
        &self.sessions
    }

    /// Get the tool registry
    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    /// Check if authenticated
    pub async fn is_authenticated(&self) -> bool {
        self.api_key.read().await.is_some()
    }

    /// Get the API key (if authenticated)
    pub async fn get_api_key(&self) -> Option<String> {
        self.api_key.read().await.clone()
    }
}

impl Default for KrustyAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait(?Send)]
impl Agent for KrustyAgent {
    /// Handle initialize request
    async fn initialize(&self, request: InitializeRequest) -> AcpResult<InitializeResponse> {
        info!(
            "ACP initialize: protocol_version={}, client={:?}",
            request.protocol_version,
            request.client_info.as_ref().map(|i| &i.name)
        );

        // Store client capabilities
        *self.client_capabilities.write().await = Some(request.client_capabilities);

        // Negotiate protocol version (use client's version, we support up to PROTOCOL_VERSION_NUM)
        let protocol_version = request.protocol_version;

        let mut response = InitializeResponse::new(protocol_version);
        response.agent_capabilities = self.agent_capabilities();
        response.agent_info = Some(self.agent_info());

        Ok(response)
    }

    /// Handle authenticate request
    async fn authenticate(&self, request: AuthenticateRequest) -> AcpResult<AuthenticateResponse> {
        info!("ACP authenticate: method={}", request.method_id);

        // We support API key authentication
        // AuthMethodId has Display, so use to_string() for comparison
        if request.method_id.to_string() != "api_key" {
            return Err(AcpSchemaError::invalid_params());
        }

        // Accept the authentication - mark as authenticated
        *self.api_key.write().await = Some("authenticated".to_string());

        info!("Authentication successful");

        Ok(AuthenticateResponse::new())
    }

    /// Handle new session request
    async fn new_session(&self, request: NewSessionRequest) -> AcpResult<NewSessionResponse> {
        // NewSessionRequest.cwd is PathBuf (not Option), mcp_servers is Vec (not Option)
        let cwd = request.cwd;
        let mcp_servers = request.mcp_servers;

        info!(
            "ACP new_session: cwd={:?}, mcp_servers={}",
            cwd,
            mcp_servers.len()
        );

        // Pass as Option to our session manager which handles defaults
        let session = self.sessions.create_session(
            Some(cwd),
            if mcp_servers.is_empty() {
                None
            } else {
                Some(mcp_servers)
            },
        );

        Ok(NewSessionResponse::new(session.id.clone()))
    }

    /// Handle load session request
    async fn load_session(&self, request: LoadSessionRequest) -> AcpResult<LoadSessionResponse> {
        info!("ACP load_session: id={}", request.session_id);

        // Check if session exists
        if !self.sessions.has_session(&request.session_id) {
            // Create a new session with the requested ID
            // In a full implementation, we'd load from storage
            warn!(
                "Session {} not found, creating new session",
                request.session_id
            );

            let _session = self.sessions.create_session(None, None);
        }

        // LoadSessionResponse::new() takes no arguments
        Ok(LoadSessionResponse::new())
    }

    /// Handle prompt request
    async fn prompt(&self, request: PromptRequest) -> AcpResult<PromptResponse> {
        info!(
            "ACP prompt: session={}, content_blocks={}",
            request.session_id,
            request.prompt.len()
        );

        // Get the session
        let session = self
            .sessions
            .get_session(&request.session_id)
            .map_err(|_e| AcpSchemaError::invalid_params())?;

        // Reset cancellation state
        session.reset_cancellation();

        // Validate prompt has content
        let prompt_text = extract_prompt_text(&request.prompt);
        if prompt_text.is_empty() {
            return Err(AcpSchemaError::invalid_params());
        }

        // Get the notification channel
        let notification_tx = self.notification_tx.read().await;
        let Some(tx) = notification_tx.as_ref() else {
            error!("No notification channel available");
            return Err(AcpSchemaError::internal_error());
        };

        // Create a bridge for this request
        let bridge = NotificationBridge::new(tx.clone());

        // Process the prompt with the PromptProcessor
        let processor = self.processor.read().await;
        let stop_reason = processor
            .process_prompt(&session, request.prompt, &bridge)
            .await
            .map_err(|e| {
                error!("Prompt processing error: {}", e);
                match e {
                    AcpError::NotAuthenticated(_) => AcpSchemaError::invalid_params(),
                    _ => AcpSchemaError::internal_error(),
                }
            })?;

        Ok(PromptResponse::new(stop_reason))
    }

    /// Handle cancel notification
    async fn cancel(&self, request: CancelNotification) -> AcpResult<()> {
        info!("ACP cancel: session={}", request.session_id);

        if let Err(e) = self.sessions.cancel_session(&request.session_id) {
            warn!("Failed to cancel session: {}", e);
        }

        Ok(())
    }

    /// Handle set session mode request
    async fn set_session_mode(
        &self,
        request: SetSessionModeRequest,
    ) -> AcpResult<SetSessionModeResponse> {
        info!(
            "ACP set_session_mode: session={}, mode={:?}",
            request.session_id, request.mode_id
        );

        let session = self
            .sessions
            .get_session(&request.session_id)
            .map_err(|_e| AcpSchemaError::invalid_params())?;
        session.set_mode(Some(request.mode_id.to_string())).await;

        Ok(SetSessionModeResponse::new())
    }

    /// Handle extension method (custom methods)
    async fn ext_method(&self, request: ExtRequest) -> AcpResult<ExtResponse> {
        debug!("ACP ext_method: {}", request.method);
        Err(AcpSchemaError::method_not_found())
    }

    /// Handle extension notification
    async fn ext_notification(&self, notification: ExtNotification) -> AcpResult<()> {
        debug!("ACP ext_notification: {}", notification.method);
        // Ignore unknown notifications
        Ok(())
    }
}

/// Extract text content from ACP content blocks
fn extract_prompt_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text(text) = block {
                Some(text.text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_creation() {
        let agent = KrustyAgent::new();
        assert_eq!(agent.sessions().session_count(), 0);
    }

    #[tokio::test]
    async fn test_new_session() {
        let agent = KrustyAgent::new();

        let request = NewSessionRequest::new("/tmp");
        let response = agent.new_session(request).await.unwrap();

        assert!(agent.sessions().has_session(&response.session_id));
    }
}
