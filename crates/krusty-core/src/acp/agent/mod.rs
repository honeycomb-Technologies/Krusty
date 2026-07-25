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
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use tokio::sync::{mpsc, Mutex, RwLock};

use super::bridge::AcpOutbound;
use super::error::AcpError;
use super::processor::PromptProcessor;
use super::session::{SessionManager, SessionState};
use crate::ai::models::{ModelKey, ModelMetadata, ResolvedModelRuntime};
use crate::ai::providers::ProviderId;
use crate::storage::credentials::ActiveProviderStore;
use crate::storage::{Database, Preferences, SessionManager as StorageSessionManager};
use crate::tools::ToolRegistry;

/// Current model configuration.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub key: ModelKey,
}

/// ACP picker row plus the exact transport that advertised it.
///
/// The credential remains private and this type intentionally does not derive
/// `Debug`, so model-picker diagnostics cannot print secrets accidentally.
#[derive(Clone)]
struct AvailableModelRecord {
    acp_model_id: String,
    metadata: ModelMetadata,
    runtime: ResolvedModelRuntime,
    credential: String,
    account_id: Option<String>,
}

impl AvailableModelRecord {
    fn new(metadata: ModelMetadata, credential: String, account_id: Option<String>) -> Self {
        let runtime = metadata.resolve_runtime();
        let acp_model_id = acp_model_id_for_key(&runtime.key);
        Self {
            acp_model_id,
            metadata,
            runtime,
            credential,
            account_id,
        }
    }

    fn key(&self) -> &ModelKey {
        &self.runtime.key
    }
}

const ACP_MODEL_KEY_PREFIX: &str = "krusty:model-key:";

/// ACP model identifiers are opaque. Encode the complete executable key so
/// rows that share a provider and wire slug cannot collide in the picker.
fn acp_model_id_for_key(key: &ModelKey) -> String {
    let encoded = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(key).expect("serializing a ModelKey cannot fail"));
    format!("{ACP_MODEL_KEY_PREFIX}{encoded}")
}

fn decode_acp_model_id(model_id: &str) -> Option<ModelKey> {
    let encoded = model_id.strip_prefix(ACP_MODEL_KEY_PREFIX)?;
    let bytes = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn persist_current_model_preference(
    preferences: &Preferences,
    key: &ModelKey,
) -> anyhow::Result<()> {
    preferences.set_current_model_key(key)
}

fn persist_shared_current_model(key: &ModelKey) {
    if key.model_id.trim().is_empty() {
        return;
    }

    let db_path = crate::paths::config_dir().join("krusty.db");
    match Database::new(&db_path) {
        Ok(db) => {
            if let Err(error) = persist_current_model_preference(&Preferences::new(db), key) {
                tracing::warn!(
                    "Failed to persist exact ACP current model preference '{}': {}",
                    key.model_id,
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

    if let Err(error) = ActiveProviderStore::save(key.provider) {
        tracing::warn!(
            "Failed to persist ACP active provider {:?}: {}",
            key.provider,
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

fn default_session_manager() -> SessionManager {
    let db_path = crate::paths::config_dir().join("krusty.db");
    match Database::new(&db_path) {
        Ok(db) => {
            SessionManager::with_storage(Arc::new(Mutex::new(StorageSessionManager::new(db))))
        }
        Err(error) => {
            tracing::error!(
                path = %db_path.display(),
                "ACP session persistence unavailable: {}",
                error
            );
            SessionManager::new()
        }
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
    notification_tx: RwLock<Option<mpsc::Sender<AcpOutbound>>>,
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
            sessions: Arc::new(default_session_manager()),
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
            sessions: Arc::new(default_session_manager()),
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
    pub async fn set_notification_channel(&self, tx: mpsc::Sender<AcpOutbound>) {
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
        let resolved_key = {
            let mut processor = self.processor.write().await;
            processor.init_ai_client(api_key, provider, selected_model);
            processor
                .default_ai_client()
                .map(|client| client.resolved_model().key.clone())
        };
        *self.current_model.write().await = resolved_key.map(|key| ModelConfig { key });
    }

    /// Retain an exact persisted default before catalog discovery. No client is
    /// constructed from the key alone because doing so would have to infer the
    /// capability row that originally supplied it.
    pub async fn set_current_model_key(&self, key: ModelKey) {
        *self.current_model.write().await = Some(ModelConfig { key });
    }

    fn agent_capabilities(&self) -> AgentCapabilities {
        let mut caps = AgentCapabilities::new();

        let mut prompt_caps = PromptCapabilities::new();
        prompt_caps.image = false;
        prompt_caps.audio = false;
        prompt_caps.embedded_context = true;
        caps.prompt_capabilities = prompt_caps;

        caps.load_session = self.sessions.storage().is_some();
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
        // Do not advertise slash commands until ACP dispatch implements them.
        Vec::new()
    }

    /// Send available commands notification to the client.
    pub async fn send_available_commands(&self, session_id: &SessionId) {
        let notification_tx = self.notification_tx.read().await;
        if let Some(tx) = notification_tx.as_ref() {
            let commands = self.get_available_commands();
            if commands.is_empty() {
                return;
            }
            let update = AvailableCommandsUpdate::new(commands);
            let notification = SessionNotification::new(
                session_id.clone(),
                SessionUpdate::AvailableCommandsUpdate(update),
            );
            if let Err(e) = tx.send(AcpOutbound::Notification(notification)).await {
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
