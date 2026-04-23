//! Krusty ACP agent.
//!
//! Keeps the protocol trait implementation separate from model/runtime helpers
//! so the top-level agent type stays focused on shared state.

mod model_selection;
mod protocol;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use agent_client_protocol::{
    AgentCapabilities, AvailableCommand, AvailableCommandsUpdate, ClientCapabilities,
    Implementation, McpCapabilities, PromptCapabilities, SessionCapabilities, SessionId,
    SessionNotification, SessionUpdate,
};
use tokio::sync::{mpsc, RwLock};

use super::error::AcpError;
use super::processor::PromptProcessor;
use super::session::{SessionManager, SessionState};
use crate::ai::providers::ProviderId;
use crate::storage::credentials::ActiveProviderStore;
use crate::storage::{Database, Preferences};
use crate::tools::ToolRegistry;

/// Current model configuration.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub provider: ProviderId,
    pub model_id: String,
}

/// (model_id, provider, actual_model_id, api_key, display_name)
type AvailableModelRecord = (String, ProviderId, String, String, String);

fn persist_shared_current_model(provider: ProviderId, model_id: &str) {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return;
    }

    let db_path = crate::paths::config_dir().join("krusty.db");
    match Database::new(&db_path) {
        Ok(db) => {
            if let Err(error) = Preferences::new(db).set_current_model(model_id) {
                tracing::warn!(
                    "Failed to persist ACP current model preference '{}': {}",
                    model_id,
                    error
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                "Failed to open preferences database for ACP model persistence: {}",
                error
            );
        }
    }

    if let Err(error) = ActiveProviderStore::save(provider) {
        tracing::warn!(
            "Failed to persist ACP active provider {:?}: {}",
            provider,
            error
        );
    }
}

fn negotiate_protocol_version(
    requested: agent_client_protocol::ProtocolVersion,
) -> agent_client_protocol::ProtocolVersion {
    use agent_client_protocol::ProtocolVersion;

    if requested < ProtocolVersion::V1 || requested > ProtocolVersion::LATEST {
        ProtocolVersion::LATEST
    } else {
        requested
    }
}

/// Krusty's ACP agent implementation.
pub struct KrustyAgent {
    /// Session manager.
    sessions: Arc<SessionManager>,
    /// Tool registry.
    tools: Arc<ToolRegistry>,
    /// Client capabilities (received during init).
    client_capabilities: RwLock<Option<ClientCapabilities>>,
    /// Authenticated API key.
    api_key: RwLock<Option<String>>,
    /// Prompt processor for AI integration.
    processor: RwLock<PromptProcessor>,
    /// Channel for sending notifications to the connection.
    notification_tx: RwLock<Option<mpsc::Sender<SessionNotification>>>,
    /// Current model configuration (provider + model).
    current_model: RwLock<Option<ModelConfig>>,
    /// Available model configurations from all providers.
    available_models: RwLock<Vec<AvailableModelRecord>>,
}

impl KrustyAgent {
    /// Create a new Krusty ACP agent.
    pub fn new() -> Self {
        let tools = Arc::new(ToolRegistry::new());
        Self {
            sessions: Arc::new(SessionManager::new()),
            tools: tools.clone(),
            client_capabilities: RwLock::new(None),
            api_key: RwLock::new(None),
            processor: RwLock::new(PromptProcessor::new(tools)),
            notification_tx: RwLock::new(None),
            current_model: RwLock::new(None),
            available_models: RwLock::new(Vec::new()),
        }
    }

    /// Create with custom tool registry.
    pub fn with_tools(tools: Arc<ToolRegistry>) -> Self {
        Self {
            sessions: Arc::new(SessionManager::new()),
            tools: tools.clone(),
            client_capabilities: RwLock::new(None),
            api_key: RwLock::new(None),
            processor: RwLock::new(PromptProcessor::new(tools)),
            notification_tx: RwLock::new(None),
            current_model: RwLock::new(None),
            available_models: RwLock::new(Vec::new()),
        }
    }

    /// Set the notification channel sender.
    pub async fn set_notification_channel(&self, tx: mpsc::Sender<SessionNotification>) {
        *self.notification_tx.write().await = Some(tx);
    }

    /// Initialize the AI client with an API key.
    pub async fn init_ai_client(&self, api_key: String, provider: ProviderId) {
        self.init_ai_client_with_model(api_key, provider, None)
            .await;
    }

    /// Initialize the AI client with an API key and optional model override.
    pub async fn init_ai_client_with_model(
        &self,
        api_key: String,
        provider: ProviderId,
        model: Option<String>,
    ) {
        let selected_model = model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty());
        self.processor
            .write()
            .await
            .init_ai_client(api_key, provider, selected_model.clone());
        *self.current_model.write().await =
            selected_model.map(|model_id| ModelConfig { provider, model_id });
    }

    fn agent_capabilities(&self) -> AgentCapabilities {
        let mut caps = AgentCapabilities::new();

        let mut prompt_caps = PromptCapabilities::new();
        prompt_caps.image = false;
        prompt_caps.audio = false;
        prompt_caps.embedded_context = true;
        caps.prompt_capabilities = prompt_caps;

        caps.load_session = true;
        caps.session_capabilities = SessionCapabilities::new();

        let mut mcp_caps = McpCapabilities::new();
        mcp_caps.http = false;
        mcp_caps.sse = false;
        caps.mcp_capabilities = mcp_caps;

        caps
    }

    fn agent_info(&self) -> Implementation {
        Implementation::new("krusty", env!("CARGO_PKG_VERSION"))
    }

    /// Get a session by ID.
    pub fn get_session(&self, id: &SessionId) -> Result<Arc<SessionState>, AcpError> {
        self.sessions.get_session(id)
    }

    /// Get the session manager.
    pub fn sessions(&self) -> &SessionManager {
        &self.sessions
    }

    /// Get the tool registry.
    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    /// Check if authenticated.
    pub async fn is_authenticated(&self) -> bool {
        self.api_key.read().await.is_some()
    }

    /// Get the API key (if authenticated).
    pub async fn get_api_key(&self) -> Option<String> {
        self.api_key.read().await.clone()
    }

    /// Get available slash commands.
    pub fn get_available_commands(&self) -> Vec<AvailableCommand> {
        vec![
            AvailableCommand::new("compact", "Summarize the conversation to reduce context"),
            AvailableCommand::new("clear", "Clear the conversation history"),
            AvailableCommand::new("help", "Show available commands and usage"),
            AvailableCommand::new("model", "Show or change the current AI model"),
            AvailableCommand::new("mode", "Switch between code and plan modes"),
        ]
    }

    /// Send available commands notification to the client.
    pub async fn send_available_commands(&self, session_id: &SessionId) {
        let notification_tx = self.notification_tx.read().await;
        if let Some(tx) = notification_tx.as_ref() {
            let commands = self.get_available_commands();
            let update = AvailableCommandsUpdate::new(commands);
            let notification = SessionNotification::new(
                session_id.clone(),
                SessionUpdate::AvailableCommandsUpdate(update),
            );
            if let Err(e) = tx.send(notification).await {
                tracing::warn!("Failed to send available commands: {}", e);
            } else {
                tracing::info!("Sent available commands update");
            }
        }
    }
}

impl Default for KrustyAgent {
    fn default() -> Self {
        Self::new()
    }
}
