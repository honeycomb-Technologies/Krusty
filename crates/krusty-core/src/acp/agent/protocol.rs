use agent_client_protocol::{
    Agent, AuthenticateRequest, AuthenticateResponse, ContentBlock, Error as AcpSchemaError,
    ExtRequest, ExtResponse, InitializeRequest, InitializeResponse, LoadSessionRequest,
    LoadSessionResponse, ModelId, ModelInfo as AcpModelInfo, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionMode, SessionModeState, SessionModelState,
    SetSessionModeRequest, SetSessionModeResponse, SetSessionModelRequest, SetSessionModelResponse,
};

use super::{negotiate_protocol_version, KrustyAgent};
use crate::acp::bridge::NotificationBridge;
use crate::acp::error::AcpError;
use crate::acp::workspace_context::build_workspace_context;

#[async_trait::async_trait(?Send)]
impl Agent for KrustyAgent {
    async fn initialize(
        &self,
        request: InitializeRequest,
    ) -> agent_client_protocol::Result<InitializeResponse> {
        tracing::info!(
            "ACP initialize: protocol_version={}, client={:?}",
            request.protocol_version,
            request.client_info.as_ref().map(|i| &i.name)
        );

        *self.client_capabilities.write().await = Some(request.client_capabilities);

        let requested_version = request.protocol_version;
        let protocol_version = negotiate_protocol_version(requested_version.clone());
        if protocol_version != requested_version {
            tracing::warn!(
                "ACP client requested unsupported protocol version {}, responding with {}",
                requested_version,
                protocol_version
            );
        }

        let mut response = InitializeResponse::new(protocol_version);
        response.agent_capabilities = self.agent_capabilities();
        response.agent_info = Some(self.agent_info());
        Ok(response)
    }

    async fn authenticate(
        &self,
        request: AuthenticateRequest,
    ) -> agent_client_protocol::Result<AuthenticateResponse> {
        tracing::info!("ACP authenticate: method={}", request.method_id);

        if request.method_id.to_string() != "api_key" {
            return Err(AcpSchemaError::invalid_params());
        }

        *self.api_key.write().await = Some("authenticated".to_string());
        tracing::info!("Authentication successful");
        Ok(AuthenticateResponse::new())
    }

    async fn new_session(
        &self,
        request: NewSessionRequest,
    ) -> agent_client_protocol::Result<NewSessionResponse> {
        let cwd = request.cwd;
        let mcp_servers = request.mcp_servers;

        tracing::info!(
            "ACP new_session: cwd={:?}, mcp_servers={}",
            cwd,
            mcp_servers.len()
        );

        let workspace_context = build_workspace_context(&cwd);
        let session = self.sessions.create_session(
            Some(cwd),
            if mcp_servers.is_empty() {
                None
            } else {
                Some(mcp_servers)
            },
        );

        session.add_system_context(workspace_context).await;
        tracing::info!("Injected workspace context for session cwd");

        let detected_models = self.detect_available_models().await;
        let mut response = NewSessionResponse::new(session.id.clone());

        let available_modes = vec![
            SessionMode::new("code", "Code").description("Write and edit code directly"),
            SessionMode::new("plan", "Plan").description("Plan changes before implementing"),
        ];
        response = response.modes(SessionModeState::new("code", available_modes));

        if !detected_models.is_empty() {
            {
                let mut available = self.available_models.write().await;
                *available = detected_models.clone();
            }

            let model_infos: Vec<AcpModelInfo> = detected_models
                .iter()
                .map(
                    |(model_id, provider, _actual_model, _api_key, display_name)| {
                        let name = format!("[{}] {}", provider, display_name);
                        AcpModelInfo::new(ModelId::new(model_id.clone()), name)
                    },
                )
                .collect();

            let current_model = self.current_model.read().await.clone();
            let current_model_id = current_model.as_ref().and_then(|selected| {
                detected_models
                    .iter()
                    .find(|(_, provider, actual_model, _, _)| {
                        *provider == selected.provider && *actual_model == selected.model_id
                    })
                    .map(|(model_id, _, _, _, _)| model_id.clone())
            });

            if let Some(current_model_id) = current_model_id {
                response = response.models(SessionModelState::new(
                    ModelId::new(current_model_id),
                    model_infos,
                ));
                tracing::info!(
                    "Session created with {} available models and shared current model",
                    detected_models.len()
                );
            } else {
                if current_model.is_some() {
                    *self.current_model.write().await = None;
                }
                tracing::info!(
                    "Session created with {} available models and no current model selected",
                    detected_models.len()
                );
            }
        } else {
            tracing::warn!("No models detected - configure API keys to enable AI features");
        }

        self.send_available_commands(&session.id).await;
        Ok(response)
    }

    async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> agent_client_protocol::Result<LoadSessionResponse> {
        tracing::info!("ACP load_session: id={}", request.session_id);

        if self.sessions.has_session(&request.session_id) {
            tracing::info!("Session {} found in memory", request.session_id);
            return Ok(LoadSessionResponse::new());
        }

        let session_id_str = request.session_id.to_string();
        let has_stored_state = if let Some(storage) = self.sessions.storage() {
            let storage_lock = storage.lock().await;
            let has_messages = storage_lock
                .load_session_messages(&session_id_str)
                .map(|messages| !messages.is_empty())
                .unwrap_or(false);
            let has_recovery = storage_lock
                .load_recovery_state(&session_id_str)
                .map(|recovery| recovery.is_some())
                .unwrap_or(false);
            let has_session = storage_lock
                .get_session(&session_id_str)
                .map(|session| session.is_some())
                .unwrap_or(false);
            has_messages || has_recovery || has_session
        } else {
            false
        };

        if has_stored_state {
            tracing::info!("Loading session {} from storage", session_id_str);
            match self
                .sessions
                .create_session_from_storage(&session_id_str, None, None)
                .await
            {
                Ok(session) => {
                    tracing::info!(
                        "Session {} restored from storage with {} messages",
                        session.id,
                        session.get_messages().await.len()
                    );
                    return Ok(LoadSessionResponse::new());
                }
                Err(e) => tracing::warn!("Failed to restore session from storage: {}", e),
            }
        }

        tracing::warn!(
            "Session {} not found in memory or storage, creating new session",
            request.session_id
        );
        let _session = self.sessions.create_session(None, None);
        Ok(LoadSessionResponse::new())
    }

    async fn prompt(
        &self,
        request: PromptRequest,
    ) -> agent_client_protocol::Result<PromptResponse> {
        tracing::info!(
            "ACP prompt: session={}, content_blocks={}",
            request.session_id,
            request.prompt.len()
        );

        let session = self
            .sessions
            .get_session(&request.session_id)
            .map_err(|_e| AcpSchemaError::invalid_params())?;

        session.reset_cancellation();

        let prompt_text = extract_prompt_text(&request.prompt);
        if prompt_text.is_empty() {
            return Err(AcpSchemaError::invalid_params());
        }

        let notification_tx = self.notification_tx.read().await;
        let Some(tx) = notification_tx.as_ref() else {
            tracing::error!("No notification channel available");
            return Err(AcpSchemaError::internal_error());
        };

        let bridge = NotificationBridge::new(tx.clone());
        let processor = self.processor.read().await;
        let stop_reason = processor
            .process_prompt(&session, request.prompt, &bridge)
            .await
            .map_err(|e| {
                tracing::error!("Prompt processing error: {}", e);
                match e {
                    AcpError::NotAuthenticated(_) => AcpSchemaError::invalid_params(),
                    _ => AcpSchemaError::internal_error(),
                }
            })?;

        Ok(PromptResponse::new(stop_reason))
    }

    async fn cancel(
        &self,
        request: agent_client_protocol::CancelNotification,
    ) -> agent_client_protocol::Result<()> {
        tracing::info!("ACP cancel: session={}", request.session_id);
        if let Err(e) = self.sessions.cancel_session(&request.session_id) {
            tracing::warn!("Failed to cancel session: {}", e);
        }
        Ok(())
    }

    async fn set_session_mode(
        &self,
        request: SetSessionModeRequest,
    ) -> agent_client_protocol::Result<SetSessionModeResponse> {
        tracing::info!(
            "ACP set_session_mode: session={}, mode={:?}",
            request.session_id,
            request.mode_id
        );

        let session = self
            .sessions
            .get_session(&request.session_id)
            .map_err(|_e| AcpSchemaError::invalid_params())?;
        session.set_mode(Some(request.mode_id.to_string())).await;

        Ok(SetSessionModeResponse::new())
    }

    async fn ext_method(&self, request: ExtRequest) -> agent_client_protocol::Result<ExtResponse> {
        tracing::debug!("ACP ext_method: {}", request.method);
        Err(AcpSchemaError::method_not_found())
    }

    async fn ext_notification(
        &self,
        notification: agent_client_protocol::ExtNotification,
    ) -> agent_client_protocol::Result<()> {
        tracing::debug!("ACP ext_notification: {}", notification.method);
        Ok(())
    }

    async fn set_session_model(
        &self,
        request: SetSessionModelRequest,
    ) -> agent_client_protocol::Result<SetSessionModelResponse> {
        tracing::info!(
            "ACP set_session_model: session={}, model={:?}",
            request.session_id,
            request.model_id
        );

        let _session = self
            .sessions
            .get_session(&request.session_id)
            .map_err(|_e| AcpSchemaError::invalid_params())?;

        let model_id_str = request.model_id.to_string();
        self.set_model(&model_id_str).await.map_err(|e| {
            tracing::error!("Failed to set model: {}", e);
            AcpSchemaError::invalid_params()
        })?;

        tracing::info!("Model switched to: {}", model_id_str);
        Ok(SetSessionModelResponse::new())
    }
}

fn extract_prompt_text(content: &[ContentBlock]) -> String {
    let mut prompt_text = String::new();
    for block in content {
        if let ContentBlock::Text(text) = block {
            if !prompt_text.is_empty() {
                prompt_text.push('\n');
            }
            prompt_text.push_str(&text.text);
        }
    }
    prompt_text
}
