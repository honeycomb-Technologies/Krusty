//! KrustyAgent - ACP Agent trait implementation
//!
//! This is the core ACP agent that handles all protocol methods.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    Agent, AgentCapabilities, AuthenticateRequest, AuthenticateResponse, CancelNotification,
    ClientCapabilities, ContentBlock, Error as AcpSchemaError, ExtNotification, ExtRequest,
    ExtResponse, Implementation, InitializeRequest, InitializeResponse, LoadSessionRequest,
    LoadSessionResponse, McpCapabilities, ModelId, ModelInfo as AcpModelInfo, NewSessionRequest,
    NewSessionResponse, PromptCapabilities, PromptRequest, PromptResponse, Result as AcpResult,
    SessionCapabilities, SessionId, SessionMode, SessionModeState,
    SessionModelState, SessionNotification, SetSessionModeRequest, SetSessionModeResponse,
    SetSessionModelRequest, SetSessionModelResponse,
};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use super::bridge::NotificationBridge;
use super::error::AcpError;
use super::processor::PromptProcessor;
use super::session::{SessionManager, SessionState};
use crate::ai::opencodezen;
use crate::ai::openrouter;
use crate::ai::providers::{get_provider, ProviderId};
use crate::storage::credentials::CredentialStore;
use crate::tools::ToolRegistry;

/// ACP protocol version supported by this agent (10 is current)
#[allow(dead_code)]
pub const PROTOCOL_VERSION_NUM: u16 = 10;

/// Current model configuration
#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub provider: ProviderId,
    pub model_id: String,
}

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
    /// Current model configuration (provider + model)
    current_model: RwLock<Option<ModelConfig>>,
    /// Available model configurations from all providers
    /// (model_id, provider, actual_model_id, api_key, display_name)
    available_models: RwLock<Vec<(String, ProviderId, String, String, String)>>,
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
            current_model: RwLock::new(None),
            available_models: RwLock::new(Vec::new()),
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
            current_model: RwLock::new(None),
            available_models: RwLock::new(Vec::new()),
            cwd,
        }
    }

    /// Detect all available models from configured providers
    /// Returns: Vec<(model_id, provider, actual_model_id, api_key, display_name)>
    pub async fn detect_available_models(&self) -> Vec<(String, ProviderId, String, String, String)> {
        let mut models = Vec::new();

        // Load credential store
        let store = match CredentialStore::load() {
            Ok(store) => store,
            Err(e) => {
                warn!("Failed to load credential store: {}", e);
                return models;
            }
        };

        // Get configured providers as a set for quick lookup
        let configured: std::collections::HashSet<_> = store.configured_providers().into_iter().collect();
        info!("Found {} configured providers", configured.len());

        // Iterate in the canonical order: Anthropic first, OpenRouter last
        // This matches the TUI model selection order
        for &provider in ProviderId::all() {
            if !configured.contains(&provider) {
                continue;
            }

            if let Some(api_key) = store.get(&provider) {
                match provider {
                    // Dynamic providers - fetch models from API
                    ProviderId::OpenRouter => {
                        info!("Fetching models from OpenRouter...");
                        match openrouter::fetch_models(&api_key).await {
                            Ok(fetched) => {
                                for model in fetched {
                                    let model_id = format!("{}:{}", provider.storage_key(), model.id);
                                    models.push((
                                        model_id,
                                        provider,
                                        model.id.clone(),
                                        api_key.clone(),
                                        model.display_name.clone(),
                                    ));
                                }
                                info!("Added {} models from OpenRouter", models.iter().filter(|(_, p, _, _, _)| *p == ProviderId::OpenRouter).count());
                            }
                            Err(e) => {
                                warn!("Failed to fetch OpenRouter models: {}", e);
                                // Fallback to static models if available
                                if let Some(provider_config) = get_provider(provider) {
                                    for model_info in &provider_config.models {
                                        let model_id = format!("{}:{}", provider.storage_key(), model_info.id);
                                        models.push((model_id, provider, model_info.id.clone(), api_key.clone(), model_info.display_name.clone()));
                                    }
                                }
                            }
                        }
                    }
                    ProviderId::OpenCodeZen => {
                        info!("Fetching models from OpenCode Zen...");
                        match opencodezen::fetch_models(&api_key).await {
                            Ok(fetched) => {
                                for model in fetched {
                                    let model_id = format!("{}:{}", provider.storage_key(), model.id);
                                    models.push((
                                        model_id,
                                        provider,
                                        model.id.clone(),
                                        api_key.clone(),
                                        model.display_name.clone(),
                                    ));
                                }
                                info!("Added {} models from OpenCode Zen", models.iter().filter(|(_, p, _, _, _)| *p == ProviderId::OpenCodeZen).count());
                            }
                            Err(e) => {
                                warn!("Failed to fetch OpenCode Zen models: {}", e);
                                // Fallback to static models
                                if let Some(provider_config) = get_provider(provider) {
                                    for model_info in &provider_config.models {
                                        let model_id = format!("{}:{}", provider.storage_key(), model_info.id);
                                        models.push((model_id, provider, model_info.id.clone(), api_key.clone(), model_info.display_name.clone()));
                                    }
                                }
                            }
                        }
                    }
                    // Static providers - use hardcoded models
                    _ => {
                        if let Some(provider_config) = get_provider(provider) {
                            for model_info in &provider_config.models {
                                let model_id = format!("{}:{}", provider.storage_key(), model_info.id);
                                models.push((
                                    model_id,
                                    provider,
                                    model_info.id.clone(),
                                    api_key.clone(),
                                    model_info.display_name.clone(),
                                ));
                                debug!("Added model: {} from {:?}", model_info.display_name, provider);
                            }
                        }
                    }
                }
            }
        }

        info!("Total models available: {}", models.len());
        models
    }

    /// Set the current model and reinitialize the processor
    pub async fn set_model(&self, model_id: &str) -> Result<(), AcpError> {
        let available = self.available_models.read().await;

        // Find the model in available models
        let model_config = available
            .iter()
            .find(|(id, _, _, _, _)| id == model_id)
            .ok_or_else(|| AcpError::ProtocolError(format!("Model not found: {}", model_id)))?;

        let (_, provider, actual_model_id, api_key, _display_name) = model_config.clone();

        info!("Switching to model: {} (provider: {:?})", actual_model_id, provider);

        // Update current model
        *self.current_model.write().await = Some(ModelConfig {
            provider,
            model_id: actual_model_id.clone(),
        });

        // Reinitialize the processor with the new model
        self.processor.write().await.init_ai_client(api_key, provider, Some(actual_model_id));

        Ok(())
    }

    /// Get the current model ID
    pub async fn current_model_id(&self) -> Option<String> {
        self.current_model.read().await.as_ref().map(|m| {
            format!("{}:{}", m.provider.storage_key(), m.model_id)
        })
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

        // Detect available models from all configured providers
        let detected_models = self.detect_available_models().await;

        // Build the response with model and mode state
        let mut response = NewSessionResponse::new(session.id.clone());

        // Set up available modes (plan and code)
        let available_modes = vec![
            SessionMode::new("code", "Code").description("Write and edit code directly"),
            SessionMode::new("plan", "Plan").description("Plan changes before implementing"),
        ];
        let mode_state = SessionModeState::new("code", available_modes);
        response = response.modes(mode_state);

        // Set up available models if any are detected
        if !detected_models.is_empty() {
            // Store models for later use (for set_model lookups)
            {
                let mut available = self.available_models.write().await;
                *available = detected_models.clone();
            }

            // Convert to ACP ModelInfo format with provider categories
            // Group by provider for better UI organization
            let model_infos: Vec<AcpModelInfo> = detected_models
                .iter()
                .map(|(model_id, provider, _actual_model, _api_key, display_name)| {
                    // Format: [Provider] Display Name
                    let name = format!("[{}] {}", provider, display_name);
                    AcpModelInfo::new(ModelId::new(model_id.clone()), name)
                })
                .collect();

            // Set the first model as current
            let current_model_id = detected_models[0].0.clone();

            // Initialize the processor with the first model
            let (_, provider, actual_model, api_key, _) = &detected_models[0];
            self.processor.write().await.init_ai_client(
                api_key.clone(),
                *provider,
                Some(actual_model.clone()),
            );

            // Store current model config
            *self.current_model.write().await = Some(ModelConfig {
                provider: *provider,
                model_id: actual_model.clone(),
            });

            let model_state = SessionModelState::new(
                ModelId::new(current_model_id),
                model_infos,
            );
            response = response.models(model_state);

            info!("Session created with {} available models", detected_models.len());
        } else {
            warn!("No models detected - configure API keys to enable AI features");
        }

        Ok(response)
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

    /// Handle set session model request
    async fn set_session_model(
        &self,
        request: SetSessionModelRequest,
    ) -> AcpResult<SetSessionModelResponse> {
        info!(
            "ACP set_session_model: session={}, model={:?}",
            request.session_id, request.model_id
        );

        // Verify session exists
        let _session = self
            .sessions
            .get_session(&request.session_id)
            .map_err(|_e| AcpSchemaError::invalid_params())?;

        // Switch to the requested model
        let model_id_str = request.model_id.to_string();
        self.set_model(&model_id_str)
            .await
            .map_err(|e| {
                error!("Failed to set model: {}", e);
                AcpSchemaError::invalid_params()
            })?;

        info!("Model switched to: {}", model_id_str);
        Ok(SetSessionModelResponse::new())
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
