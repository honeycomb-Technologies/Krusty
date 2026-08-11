//! Desktop-facing backend selection and capability boundary.

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    AgentBackend, AgentError, ApprovalChoice, CodexAppServerBackend, CommandExecParams,
    CommandExecResizeParams, CommandExecResizeResponse, CommandExecResponse,
    CommandExecTerminateParams, CommandExecTerminateResponse, CommandExecWriteParams,
    CommandExecWriteResponse, ConfigBatchWriteParams, ConfigRequirementsReadResponse,
    ConfigWriteResponse, ConsumeAccountRateLimitResetCreditParams,
    ConsumeAccountRateLimitResetCreditResponse, ExperimentalFeatureEnablementSetParams,
    ExperimentalFeatureEnablementSetResponse, ExperimentalFeatureListParams,
    ExperimentalFeatureListResponse, ExternalAgentConfigDetectParams,
    ExternalAgentConfigDetectResponse, ExternalAgentConfigImportHistoriesReadResponse,
    ExternalAgentConfigImportHistoryRecordParams, ExternalAgentConfigImportHistoryRecordResponse,
    ExternalAgentConfigImportParams, ExternalAgentConfigImportResponse, FsCopyParams,
    FsCopyResponse, FsCreateDirectoryParams, FsCreateDirectoryResponse, FsRemoveParams,
    FsRemoveResponse, FsUnwatchParams, FsUnwatchResponse, FsWatchParams, FsWatchResponse,
    FsWriteFileParams, FsWriteFileResponse, GetWorkspaceMessagesResponse, LifecycleNotification,
    LiveApprovalBridge, LiveTurnOutcome, McpServerConfigAddParams, McpServerOauthLoginParams,
    McpServerOauthLoginResponse, MitsuroServerBackend, ModelProviderCapabilitiesReadParams,
    ModelProviderCapabilitiesReadResponse, PendingApproval, PermissionProfileListParams,
    PermissionProfileListResponse, PluginInstallParams, PluginInstallResponse,
    PluginUninstallParams, PluginUninstallResponse, RemoteControlClientsListParams,
    RemoteControlClientsListResponse, RemoteControlClientsRevokeParams,
    RemoteControlClientsRevokeResponse, RemoteControlDisableParams, RemoteControlDisableResponse,
    RemoteControlEnableParams, RemoteControlEnableResponse, RemoteControlPairingStartParams,
    RemoteControlPairingStartResponse, RemoteControlPairingStatusParams,
    RemoteControlPairingStatusResponse, RemoteControlStatusReadResponse, Result,
    SendAddCreditsNudgeEmailParams, SendAddCreditsNudgeEmailResponse,
    ThreadBackgroundTerminalsCleanParams, ThreadBackgroundTerminalsCleanResponse,
    ThreadBackgroundTerminalsListParams, ThreadBackgroundTerminalsListResponse,
    ThreadBackgroundTerminalsTerminateParams, ThreadBackgroundTerminalsTerminateResponse,
    ThreadInjectItemsParams, ThreadInjectItemsResponse, ThreadRealtimeAppendAudioParams,
    ThreadRealtimeAppendAudioResponse, ThreadRealtimeAppendSpeechParams,
    ThreadRealtimeAppendSpeechResponse, ThreadRealtimeAppendTextParams,
    ThreadRealtimeAppendTextResponse, ThreadRealtimeListVoicesParams,
    ThreadRealtimeListVoicesResponse, ThreadRealtimeStartParams, ThreadRealtimeStartResponse,
    ThreadRealtimeStopParams, ThreadRealtimeStopResponse, ThreadSearchOccurrence,
    ThreadShellCommandParams, ThreadShellCommandResponse, ThreadTurnItemsView,
    ThreadTurnsListParams, ThreadTurnsSortDirection, TurnStartParams, TurnStreamEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    MitsuroHttp,
    CodexStdio,
    CodexWebSocket,
    Fixture,
}

impl BackendKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::MitsuroHttp => "mitsuro-http",
            Self::CodexStdio => "codex-stdio",
            Self::CodexWebSocket => "codex-ws",
            Self::Fixture => "fixture",
        }
    }

    pub fn from_id(value: &str) -> Option<Self> {
        match value {
            "mitsuro-http" => Some(Self::MitsuroHttp),
            "codex-stdio" => Some(Self::CodexStdio),
            "codex-ws" => Some(Self::CodexWebSocket),
            "fixture" => Some(Self::Fixture),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub sessions: bool,
    pub streaming_chat: bool,
    pub image_attachments: bool,
    pub audio_attachments: bool,
    pub realtime_voice: bool,
    pub skill_inputs: bool,
    pub mention_inputs: bool,
    pub workspace_selection: bool,
    pub access_modes: bool,
    pub steering: bool,
    pub manual_compaction: bool,
    pub review: bool,
    pub approvals: bool,
    pub models: bool,
    pub files: bool,
    pub file_mutations: bool,
    pub file_watches: bool,
    pub processes: bool,
    pub command_exec: bool,
    pub thread_shell_commands: bool,
    pub background_terminals: bool,
    pub tracked_process_kill: bool,
    pub extensions: bool,
    pub plugin_mutations: bool,
    pub environment_add: bool,
    pub mcp_oauth: bool,
    pub mcp_config_write: bool,
    pub hooks: bool,
    pub apps: bool,
    pub skill_config_write: bool,
    pub permission_profiles: bool,
    pub config_requirements: bool,
    pub model_provider_capabilities: bool,
    pub external_agent_import: bool,
    pub experimental_features: bool,
    pub memory_settings: bool,
    pub thread_settings: bool,
    pub thread_metadata: bool,
    pub item_pagination: bool,
    pub account_workspace_messages: bool,
    pub account_reset_credits: bool,
    pub account_credit_nudge: bool,
    pub remote_control: bool,
    pub hive: bool,
    pub hive_mutations: bool,
    pub schedules: bool,
    pub schedule_mutations: bool,
    pub sites: bool,
    pub archive: bool,
    pub fork: bool,
    pub side_conversations: bool,
    pub conversation_search: bool,
    pub paged_history: bool,
    pub edit_latest_message: bool,
}

impl BackendCapabilities {
    pub const fn codex() -> Self {
        Self {
            sessions: true,
            streaming_chat: true,
            image_attachments: true,
            audio_attachments: true,
            realtime_voice: true,
            skill_inputs: true,
            mention_inputs: true,
            workspace_selection: true,
            access_modes: true,
            steering: true,
            manual_compaction: true,
            review: true,
            approvals: true,
            models: true,
            files: true,
            file_mutations: true,
            file_watches: true,
            processes: true,
            command_exec: true,
            thread_shell_commands: true,
            background_terminals: true,
            tracked_process_kill: false,
            extensions: true,
            plugin_mutations: true,
            environment_add: true,
            mcp_oauth: true,
            mcp_config_write: true,
            hooks: true,
            apps: true,
            skill_config_write: true,
            permission_profiles: true,
            config_requirements: true,
            model_provider_capabilities: true,
            external_agent_import: true,
            experimental_features: true,
            memory_settings: true,
            thread_settings: true,
            thread_metadata: true,
            item_pagination: true,
            account_workspace_messages: true,
            account_reset_credits: true,
            account_credit_nudge: true,
            remote_control: true,
            hive: false,
            hive_mutations: false,
            schedules: false,
            schedule_mutations: false,
            sites: false,
            archive: true,
            fork: true,
            side_conversations: true,
            conversation_search: true,
            paged_history: true,
            edit_latest_message: true,
        }
    }

    pub const fn mitsuro() -> Self {
        Self {
            sessions: true,
            streaming_chat: true,
            image_attachments: true,
            audio_attachments: false,
            realtime_voice: false,
            skill_inputs: false,
            mention_inputs: false,
            workspace_selection: true,
            access_modes: true,
            steering: true,
            manual_compaction: false,
            review: false,
            approvals: true,
            models: true,
            files: true,
            file_mutations: false,
            file_watches: false,
            // The HTTP API can inspect/kill tracked background processes, but it
            // does not expose the interactive spawn/stdin/PTY contract used by
            // the native terminal panel.
            processes: false,
            command_exec: false,
            thread_shell_commands: false,
            background_terminals: false,
            tracked_process_kill: true,
            extensions: true,
            plugin_mutations: false,
            environment_add: false,
            mcp_oauth: false,
            mcp_config_write: false,
            hooks: false,
            apps: false,
            skill_config_write: false,
            permission_profiles: false,
            config_requirements: false,
            model_provider_capabilities: false,
            external_agent_import: false,
            experimental_features: false,
            memory_settings: false,
            thread_settings: false,
            thread_metadata: false,
            item_pagination: false,
            account_workspace_messages: false,
            account_reset_credits: false,
            account_credit_nudge: false,
            remote_control: false,
            hive: true,
            hive_mutations: true,
            schedules: true,
            schedule_mutations: true,
            sites: false,
            archive: false,
            fork: false,
            side_conversations: false,
            conversation_search: true,
            paged_history: true,
            edit_latest_message: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSelection {
    Auto,
    MitsuroHttp,
    CodexStdio,
    CodexWebSocket,
    Fixture,
}

impl BackendSelection {
    pub fn from_env() -> Result<Self> {
        let value = std::env::var("MITSURO_BACKEND")
            .unwrap_or_else(|_| "mitsuro-http".to_owned())
            .to_lowercase();
        match value.as_str() {
            "auto" => Ok(Self::Auto),
            "mitsuro" | "mitsuro-http" => Ok(Self::MitsuroHttp),
            "codex" | "codex-stdio" => Ok(Self::CodexStdio),
            "codex-ws" | "codex-websocket" => Ok(Self::CodexWebSocket),
            "fixture" => Ok(Self::Fixture),
            other => Err(AgentError::Other(format!(
                "unknown MITSURO_BACKEND={other}; expected auto, mitsuro-http, codex-stdio, codex-ws, or fixture"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BackendSessionId {
    pub backend: BackendKind,
    pub raw: String,
}

impl BackendSessionId {
    pub fn new(backend: BackendKind, raw: impl Into<String>) -> Self {
        Self {
            backend,
            raw: raw.into(),
        }
    }

    pub fn qualified(&self) -> String {
        format!("{}:{}", self.backend.id(), self.raw)
    }

    pub fn parse_qualified(value: &str) -> Result<Self> {
        let (backend, raw) = value.split_once(':').ok_or_else(|| {
            AgentError::Protocol(format!("invalid backend-qualified session id: {value}"))
        })?;
        let backend = BackendKind::from_id(backend).ok_or_else(|| {
            AgentError::Protocol(format!("unknown session backend in id: {value}"))
        })?;
        if raw.is_empty() {
            return Err(AgentError::Protocol(format!(
                "empty raw session id in: {value}"
            )));
        }
        Ok(Self::new(backend, raw))
    }
}

pub enum DesktopBackend {
    Codex(Arc<CodexAppServerBackend>),
    Mitsuro(Arc<MitsuroServerBackend>),
}

impl DesktopBackend {
    pub fn codex_stdio() -> Self {
        Self::Codex(Arc::new(CodexAppServerBackend::with_defaults()))
    }

    pub fn mitsuro_from_env() -> Result<Self> {
        Ok(Self::Mitsuro(Arc::new(MitsuroServerBackend::from_env()?)))
    }

    pub fn kind(&self) -> BackendKind {
        match self {
            Self::Codex(_) => BackendKind::CodexStdio,
            Self::Mitsuro(_) => BackendKind::MitsuroHttp,
        }
    }

    pub fn capabilities(&self) -> BackendCapabilities {
        match self {
            Self::Codex(_) => BackendCapabilities::codex(),
            Self::Mitsuro(_) => BackendCapabilities::mitsuro(),
        }
    }

    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        match self {
            Self::Codex(backend) => backend.block_on(future),
            Self::Mitsuro(_) => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Mitsuro desktop runtime")
                .block_on(future),
        }
    }

    pub async fn has_usable_auth(&self) -> bool {
        match self {
            Self::Codex(backend) => backend.has_usable_auth().await,
            // Successful Mitsuro health establishes that local access or the
            // configured bearer token is accepted.
            Self::Mitsuro(_) => true,
        }
    }

    pub async fn respond_approval(
        &self,
        pending: &PendingApproval,
        choice: ApprovalChoice,
    ) -> Result<()> {
        match self {
            Self::Codex(backend) => backend.respond_approval(pending, choice).await,
            Self::Mitsuro(backend) => {
                let tool_call_id = match &pending.request_id {
                    crate::JsonRpcId::String(id) => id,
                    crate::JsonRpcId::Number(_) => {
                        return Err(AgentError::Protocol(
                            "Mitsuro approval requires a string tool-call id".to_owned(),
                        ));
                    }
                };
                let session_id = pending.thread_id.as_deref().ok_or_else(|| {
                    AgentError::Protocol("Mitsuro approval is missing a session id".to_owned())
                })?;
                backend
                    .client()
                    .approve_tool(
                        session_id,
                        tool_call_id,
                        matches!(choice, ApprovalChoice::Approve),
                    )
                    .await
                    .map_err(|error| AgentError::Other(error.to_string()))?;
                Ok(())
            }
        }
    }

    /// Answer an interactive Codex server request. Mitsuro HTTP interactions
    /// use their own typed endpoints and never carry JSON-RPC ids.
    pub async fn respond_to_server_request(
        &self,
        id: crate::JsonRpcId,
        result: serde_json::Value,
    ) -> Result<()> {
        match self {
            Self::Codex(backend) => backend.respond_to_server_request(id, result).await,
            Self::Mitsuro(_) => Err(AgentError::Protocol(
                "Mitsuro HTTP does not expose Codex JSON-RPC server requests".to_owned(),
            )),
        }
    }

    /// Application-lifetime lifecycle stream for backends that expose one.
    ///
    /// Mitsuro HTTP currently projects lifecycle through its typed REST/SSE
    /// surfaces, so only the Codex app-server transport returns a receiver here.
    pub fn subscribe_lifecycle_events(
        &self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<LifecycleNotification>> {
        match self {
            Self::Codex(backend) => Some(backend.subscribe_lifecycle_events()),
            Self::Mitsuro(_) => None,
        }
    }

    fn ensure_realtime_session_origin(&self, session: &BackendSessionId) -> Result<()> {
        self.ensure_session_origin(session)
    }

    /// Search visible user/final-assistant messages in a real backend session.
    pub async fn search_thread_occurrences(
        &self,
        session: &BackendSessionId,
        search_term: impl Into<String>,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<crate::ThreadSearchOccurrencesResponse> {
        self.ensure_session_origin(session)?;
        let params = crate::ThreadSearchOccurrencesParams {
            thread_id: session.raw.clone(),
            search_term: search_term.into(),
            cursor,
            limit,
        };
        match self {
            Self::Codex(backend) => backend.thread_search_occurrences(params).await,
            Self::Mitsuro(backend) => backend.thread_search_occurrences(params).await,
        }
    }

    /// Read one real page from the backend session's turn history.
    pub async fn list_thread_turns(
        &self,
        session: &BackendSessionId,
        params: crate::ThreadTurnsListParams,
    ) -> Result<crate::ThreadTurnsListResponse> {
        self.ensure_session_origin(session)?;
        let params = crate::ThreadTurnsListParams {
            thread_id: session.raw.clone(),
            ..params
        };
        match self {
            Self::Codex(backend) => backend.thread_turns_list(params).await,
            Self::Mitsuro(backend) => backend.thread_turns_list(params).await,
        }
    }

    /// Hydrate a persisted search match and a small amount of surrounding history.
    ///
    /// The Codex reference requests pages in both directions from the occurrence's
    /// turn cursor. Mitsuro maps the same operation onto its real transcript so the
    /// GPUI can use one backend-neutral recovery path when a match is outside the
    /// currently rendered history window.
    pub async fn hydrate_thread_search_match(
        &self,
        session: &BackendSessionId,
        occurrence: &ThreadSearchOccurrence,
        page_limit: u32,
    ) -> Result<Vec<crate::ConversationMessage>> {
        self.ensure_session_origin(session)?;
        let params = |sort_direction| ThreadTurnsListParams {
            thread_id: session.raw.clone(),
            cursor: Some(occurrence.turn_cursor.clone()),
            limit: Some(page_limit.clamp(1, 20)),
            sort_direction: Some(sort_direction),
            items_view: Some(ThreadTurnItemsView::Full),
        };
        let (older, newer) = futures::try_join!(
            self.list_thread_turns(session, params(ThreadTurnsSortDirection::Desc),),
            self.list_thread_turns(session, params(ThreadTurnsSortDirection::Asc),)
        )?;

        let mut turns = older.data;
        turns.reverse();
        let mut seen_turn_ids = turns
            .iter()
            .filter_map(|turn| turn.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect::<std::collections::HashSet<_>>();
        for turn in newer.data {
            let duplicate = turn
                .get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| !seen_turn_ids.insert(id.to_owned()));
            if !duplicate {
                turns.push(turn);
            }
        }

        let messages = crate::product::conversation_messages_from_turn_values(turns);
        if !messages
            .iter()
            .any(|message| message.item_id.as_deref() == Some(occurrence.item_id.as_str()))
        {
            return Err(AgentError::Protocol(
                "persisted conversation search match is no longer available".to_owned(),
            ));
        }
        Ok(messages)
    }

    /// Remove completed turns from a Codex thread before resubmitting an edit.
    pub async fn rollback_thread(
        &self,
        session: &BackendSessionId,
        num_turns: u32,
    ) -> Result<crate::ThreadRollbackResponse> {
        self.ensure_session_origin(session)?;
        let params = crate::ThreadRollbackParams {
            thread_id: session.raw.clone(),
            num_turns,
        };
        match self {
            Self::Codex(backend) => backend.thread_rollback(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose destructive turn rollback".to_owned(),
            )),
        }
    }

    /// Append model-visible response items to a backend-qualified Codex thread.
    pub async fn inject_thread_items(
        &self,
        session: &BackendSessionId,
        items: Vec<serde_json::Value>,
    ) -> Result<ThreadInjectItemsResponse> {
        self.ensure_session_origin(session)?;
        match self {
            Self::Codex(backend) => {
                backend
                    .thread_inject_items(ThreadInjectItemsParams::new(session.raw.clone(), items))
                    .await
            }
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose model-history item injection".to_owned(),
            )),
        }
    }

    /// Start Codex's host-local user shell command for a backend-qualified thread.
    /// The empty response only acknowledges launch; output is delivered through the
    /// existing lifecycle notification stream.
    pub async fn start_thread_shell_command(
        &self,
        session: &BackendSessionId,
        command: impl Into<String>,
    ) -> Result<ThreadShellCommandResponse> {
        self.ensure_session_origin(session)?;
        match self {
            Self::Codex(backend) => {
                backend
                    .thread_shell_command(ThreadShellCommandParams::new(
                        session.raw.clone(),
                        command,
                    ))
                    .await
            }
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex host-local shell commands".to_owned(),
            )),
        }
    }

    pub async fn realtime_list_voices(&self) -> Result<ThreadRealtimeListVoicesResponse> {
        match self {
            Self::Codex(backend) => {
                backend
                    .realtime_list_voices(ThreadRealtimeListVoicesParams::default())
                    .await
            }
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose realtime voice sessions".to_owned(),
            )),
        }
    }

    pub async fn install_plugin(
        &self,
        params: PluginInstallParams,
    ) -> Result<PluginInstallResponse> {
        match self {
            Self::Codex(backend) => backend.plugin_install(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes extension inventory but not plugin installation".to_owned(),
            )),
        }
    }

    pub async fn mcp_oauth_login(
        &self,
        params: McpServerOauthLoginParams,
    ) -> Result<McpServerOauthLoginResponse> {
        match self {
            Self::Codex(backend) => backend.mcp_server_oauth_login(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex MCP OAuth login".to_owned(),
            )),
        }
    }

    pub async fn add_mcp_server(
        &self,
        params: McpServerConfigAddParams,
    ) -> Result<crate::ConfigWriteResponse> {
        match self {
            Self::Codex(backend) => {
                let response = backend
                    .config_value_write(params.config_write_params())
                    .await?;
                backend.config_mcp_server_reload().await?;
                Ok(response)
            }
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose MCP configuration writes".to_owned(),
            )),
        }
    }

    pub async fn list_hooks(
        &self,
        params: crate::HooksListParams,
    ) -> Result<crate::HooksListResponse> {
        match self {
            Self::Codex(backend) => backend.hooks_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose a lifecycle hook catalog".to_owned(),
            )),
        }
    }

    pub async fn list_apps(
        &self,
        params: crate::AppsListParams,
    ) -> Result<crate::AppsListResponse> {
        match self {
            Self::Codex(backend) => backend.apps_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose the Codex app catalog".to_owned(),
            )),
        }
    }

    pub async fn list_installed_apps(
        &self,
        params: crate::AppsInstalledParams,
    ) -> Result<crate::AppsInstalledResponse> {
        match self {
            Self::Codex(backend) => backend.apps_installed(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose the Codex installed app snapshot".to_owned(),
            )),
        }
    }

    pub async fn read_apps(
        &self,
        params: crate::AppsReadParams,
    ) -> Result<crate::AppsReadResponse> {
        match self {
            Self::Codex(backend) => backend.apps_read(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose the Codex app metadata catalog".to_owned(),
            )),
        }
    }

    pub async fn write_skill_config(
        &self,
        params: crate::SkillsConfigWriteParams,
    ) -> Result<crate::SkillsConfigWriteResponse> {
        match self {
            Self::Codex(backend) => backend.skills_config_write(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex skill configuration writes".to_owned(),
            )),
        }
    }

    pub async fn remote_control_status(&self) -> Result<RemoteControlStatusReadResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_status_read().await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control".to_owned(),
            )),
        }
    }

    pub async fn list_permission_profiles(
        &self,
        params: PermissionProfileListParams,
    ) -> Result<PermissionProfileListResponse> {
        match self {
            Self::Codex(backend) => backend.permission_profile_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP uses supervised/autonomous modes, not Codex permission profiles"
                    .to_owned(),
            )),
        }
    }

    pub async fn read_config_requirements(&self) -> Result<ConfigRequirementsReadResponse> {
        match self {
            Self::Codex(backend) => backend.config_requirements_read().await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex managed configuration requirements".to_owned(),
            )),
        }
    }

    pub async fn read_model_provider_capabilities(
        &self,
        params: ModelProviderCapabilitiesReadParams,
    ) -> Result<ModelProviderCapabilitiesReadResponse> {
        match self {
            Self::Codex(backend) => backend.model_provider_capabilities_read(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes capabilities through its own model catalog".to_owned(),
            )),
        }
    }

    pub async fn list_experimental_features(
        &self,
        params: ExperimentalFeatureListParams,
    ) -> Result<ExperimentalFeatureListResponse> {
        match self {
            Self::Codex(backend) => backend.experimental_feature_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose the Codex experimental-feature catalog".to_owned(),
            )),
        }
    }

    pub async fn set_experimental_feature_enablement(
        &self,
        params: ExperimentalFeatureEnablementSetParams,
    ) -> Result<ExperimentalFeatureEnablementSetResponse> {
        match self {
            Self::Codex(backend) => backend.experimental_feature_enablement_set(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex runtime feature enablement".to_owned(),
            )),
        }
    }

    pub async fn write_config_batch(
        &self,
        params: ConfigBatchWriteParams,
    ) -> Result<ConfigWriteResponse> {
        match self {
            Self::Codex(backend) => backend.config_batch_write(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex configuration writes".to_owned(),
            )),
        }
    }

    pub async fn set_thread_memory_mode(
        &self,
        params: crate::ThreadMemoryModeSetParams,
    ) -> Result<crate::ThreadMemoryModeSetResponse> {
        match self {
            Self::Codex(backend) => backend.thread_memory_mode_set(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose per-thread Codex memory mode".to_owned(),
            )),
        }
    }

    pub async fn reset_memories(&self) -> Result<crate::MemoryResetResponse> {
        match self {
            Self::Codex(backend) => backend.memory_reset().await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose the Codex local-memory store".to_owned(),
            )),
        }
    }

    pub async fn update_thread_settings(
        &self,
        session: &BackendSessionId,
        mut params: crate::ThreadSettingsUpdateParams,
    ) -> Result<crate::ThreadSettingsUpdateResponse> {
        self.ensure_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.thread_settings_update(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex per-thread settings".to_owned(),
            )),
        }
    }

    pub async fn update_thread_metadata(
        &self,
        session: &BackendSessionId,
        mut params: crate::ThreadMetadataUpdateParams,
    ) -> Result<crate::ThreadMetadataUpdateResponse> {
        self.ensure_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.thread_metadata_update(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex per-thread metadata".to_owned(),
            )),
        }
    }

    pub async fn list_thread_items(
        &self,
        session: &BackendSessionId,
        mut params: crate::ThreadItemsListParams,
    ) -> Result<crate::ThreadItemsListResponse> {
        self.ensure_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.thread_items_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex item pagination".to_owned(),
            )),
        }
    }

    pub async fn read_account_workspace_messages(&self) -> Result<GetWorkspaceMessagesResponse> {
        match self {
            Self::Codex(backend) => backend.account_workspace_messages_read().await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex workspace account messages".to_owned(),
            )),
        }
    }

    pub async fn consume_account_rate_limit_reset_credit(
        &self,
        params: ConsumeAccountRateLimitResetCreditParams,
    ) -> Result<ConsumeAccountRateLimitResetCreditResponse> {
        match self {
            Self::Codex(backend) => {
                backend
                    .account_rate_limit_reset_credit_consume(params)
                    .await
            }
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex rate-limit reset credits".to_owned(),
            )),
        }
    }

    pub async fn send_account_add_credits_nudge_email(
        &self,
        params: SendAddCreditsNudgeEmailParams,
    ) -> Result<SendAddCreditsNudgeEmailResponse> {
        match self {
            Self::Codex(backend) => backend.account_send_add_credits_nudge_email(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex workspace credit email actions".to_owned(),
            )),
        }
    }

    pub async fn detect_external_agent_config(
        &self,
        params: ExternalAgentConfigDetectParams,
    ) -> Result<ExternalAgentConfigDetectResponse> {
        match self {
            Self::Codex(backend) => backend.external_agent_config_detect(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex external-agent imports".to_owned(),
            )),
        }
    }

    pub async fn import_external_agent_config(
        &self,
        params: ExternalAgentConfigImportParams,
    ) -> Result<ExternalAgentConfigImportResponse> {
        match self {
            Self::Codex(backend) => backend.external_agent_config_import(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex external-agent imports".to_owned(),
            )),
        }
    }

    pub async fn read_external_agent_import_histories(
        &self,
    ) -> Result<ExternalAgentConfigImportHistoriesReadResponse> {
        match self {
            Self::Codex(backend) => backend.external_agent_config_import_read_histories().await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex external-agent import history".to_owned(),
            )),
        }
    }

    pub async fn record_external_agent_import_history(
        &self,
        params: ExternalAgentConfigImportHistoryRecordParams,
    ) -> Result<ExternalAgentConfigImportHistoryRecordResponse> {
        match self {
            Self::Codex(backend) => {
                backend
                    .external_agent_config_import_record_history(params)
                    .await
            }
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex external-agent import history".to_owned(),
            )),
        }
    }

    pub async fn enable_remote_control(
        &self,
        params: RemoteControlEnableParams,
    ) -> Result<RemoteControlEnableResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_enable(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control".to_owned(),
            )),
        }
    }

    pub async fn disable_remote_control(
        &self,
        params: RemoteControlDisableParams,
    ) -> Result<RemoteControlDisableResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_disable(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control".to_owned(),
            )),
        }
    }

    pub async fn start_remote_control_pairing(
        &self,
        params: RemoteControlPairingStartParams,
    ) -> Result<RemoteControlPairingStartResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_pairing_start(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control pairing".to_owned(),
            )),
        }
    }

    pub async fn remote_control_pairing_status(
        &self,
        params: RemoteControlPairingStatusParams,
    ) -> Result<RemoteControlPairingStatusResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_pairing_status(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control pairing".to_owned(),
            )),
        }
    }

    pub async fn list_remote_control_clients(
        &self,
        params: RemoteControlClientsListParams,
    ) -> Result<RemoteControlClientsListResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_clients_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control clients".to_owned(),
            )),
        }
    }

    pub async fn revoke_remote_control_client(
        &self,
        params: RemoteControlClientsRevokeParams,
    ) -> Result<RemoteControlClientsRevokeResponse> {
        match self {
            Self::Codex(backend) => backend.remote_control_clients_revoke(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex Remote Control clients".to_owned(),
            )),
        }
    }

    pub async fn exec_command(&self, params: CommandExecParams) -> Result<CommandExecResponse> {
        match self {
            Self::Codex(backend) => backend.command_exec(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose the Codex standalone command contract".to_owned(),
            )),
        }
    }

    pub async fn write_command_stdin(
        &self,
        params: CommandExecWriteParams,
    ) -> Result<CommandExecWriteResponse> {
        match self {
            Self::Codex(backend) => backend.command_exec_write(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose standalone command stdin".to_owned(),
            )),
        }
    }

    pub async fn resize_command_pty(
        &self,
        params: CommandExecResizeParams,
    ) -> Result<CommandExecResizeResponse> {
        match self {
            Self::Codex(backend) => backend.command_exec_resize(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose standalone command PTY resizing".to_owned(),
            )),
        }
    }

    pub async fn terminate_command(
        &self,
        params: CommandExecTerminateParams,
    ) -> Result<CommandExecTerminateResponse> {
        match self {
            Self::Codex(backend) => backend.command_exec_terminate(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose standalone command termination".to_owned(),
            )),
        }
    }

    pub async fn list_thread_background_terminals(
        &self,
        session: &BackendSessionId,
        mut params: ThreadBackgroundTerminalsListParams,
    ) -> Result<ThreadBackgroundTerminalsListResponse> {
        self.ensure_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.thread_background_terminals_list(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes a global process registry, not Codex thread terminals"
                    .to_owned(),
            )),
        }
    }

    pub async fn clean_thread_background_terminals(
        &self,
        session: &BackendSessionId,
        mut params: ThreadBackgroundTerminalsCleanParams,
    ) -> Result<ThreadBackgroundTerminalsCleanResponse> {
        self.ensure_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.thread_background_terminals_clean(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex thread-terminal cleanup".to_owned(),
            )),
        }
    }

    pub async fn terminate_thread_background_terminal(
        &self,
        session: &BackendSessionId,
        mut params: ThreadBackgroundTerminalsTerminateParams,
    ) -> Result<ThreadBackgroundTerminalsTerminateResponse> {
        self.ensure_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.thread_background_terminals_terminate(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose Codex thread-terminal termination".to_owned(),
            )),
        }
    }

    pub async fn write_file(&self, params: FsWriteFileParams) -> Result<FsWriteFileResponse> {
        match self {
            Self::Codex(backend) => backend.fs_write_file(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes file reads but not filesystem writes".to_owned(),
            )),
        }
    }

    pub async fn create_directory(
        &self,
        params: FsCreateDirectoryParams,
    ) -> Result<FsCreateDirectoryResponse> {
        match self {
            Self::Codex(backend) => backend.fs_create_directory(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes directory reads but not directory creation".to_owned(),
            )),
        }
    }

    pub async fn remove_path(&self, params: FsRemoveParams) -> Result<FsRemoveResponse> {
        match self {
            Self::Codex(backend) => backend.fs_remove(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes file reads but not filesystem removal".to_owned(),
            )),
        }
    }

    pub async fn copy_path(&self, params: FsCopyParams) -> Result<FsCopyResponse> {
        match self {
            Self::Codex(backend) => backend.fs_copy(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes file reads but not filesystem copy".to_owned(),
            )),
        }
    }

    pub async fn watch_path(&self, params: FsWatchParams) -> Result<FsWatchResponse> {
        match self {
            Self::Codex(backend) => backend.fs_watch(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose filesystem watch subscriptions".to_owned(),
            )),
        }
    }

    pub async fn unwatch_path(&self, params: FsUnwatchParams) -> Result<FsUnwatchResponse> {
        match self {
            Self::Codex(backend) => backend.fs_unwatch(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose filesystem watch subscriptions".to_owned(),
            )),
        }
    }

    pub async fn uninstall_plugin(
        &self,
        params: PluginUninstallParams,
    ) -> Result<PluginUninstallResponse> {
        match self {
            Self::Codex(backend) => backend.plugin_uninstall(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP exposes extension inventory but not plugin removal".to_owned(),
            )),
        }
    }

    pub async fn realtime_start(
        &self,
        session: &BackendSessionId,
        mut params: ThreadRealtimeStartParams,
    ) -> Result<ThreadRealtimeStartResponse> {
        self.ensure_realtime_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.realtime_start(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose realtime voice sessions".to_owned(),
            )),
        }
    }

    pub async fn realtime_append_audio(
        &self,
        session: &BackendSessionId,
        mut params: ThreadRealtimeAppendAudioParams,
    ) -> Result<ThreadRealtimeAppendAudioResponse> {
        self.ensure_realtime_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.realtime_append_audio(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose realtime voice sessions".to_owned(),
            )),
        }
    }

    pub async fn realtime_append_text(
        &self,
        session: &BackendSessionId,
        mut params: ThreadRealtimeAppendTextParams,
    ) -> Result<ThreadRealtimeAppendTextResponse> {
        self.ensure_realtime_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.realtime_append_text(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose realtime voice sessions".to_owned(),
            )),
        }
    }

    pub async fn realtime_append_speech(
        &self,
        session: &BackendSessionId,
        mut params: ThreadRealtimeAppendSpeechParams,
    ) -> Result<ThreadRealtimeAppendSpeechResponse> {
        self.ensure_realtime_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.realtime_append_speech(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose realtime voice sessions".to_owned(),
            )),
        }
    }

    pub async fn realtime_stop(
        &self,
        session: &BackendSessionId,
        mut params: ThreadRealtimeStopParams,
    ) -> Result<ThreadRealtimeStopResponse> {
        self.ensure_realtime_session_origin(session)?;
        params.thread_id.clone_from(&session.raw);
        match self {
            Self::Codex(backend) => backend.realtime_stop(params).await,
            Self::Mitsuro(_) => Err(AgentError::NotImplemented(
                "Mitsuro HTTP does not expose realtime voice sessions".to_owned(),
            )),
        }
    }

    pub fn run_turn_with_bridge_blocking(
        &self,
        params: TurnStartParams,
        event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
        bridge: Arc<LiveApprovalBridge>,
        timeout: Duration,
    ) -> Result<LiveTurnOutcome> {
        match self {
            Self::Codex(backend) => {
                let runtime = Arc::clone(backend);
                let runner = Arc::clone(backend);
                let thread_id = params.thread_id;
                let text = params
                    .input
                    .iter()
                    .filter_map(|value| value.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                let model = params.model;
                runtime.block_on(async move {
                    crate::run_live_turn_with_bridge_and_model(
                        runner.as_ref(),
                        thread_id,
                        text,
                        model,
                        |event| {
                            let _ = event_tx.send(event);
                        },
                        bridge,
                        timeout,
                    )
                    .await
                })
            }
            Self::Mitsuro(backend) => {
                let backend = Arc::clone(backend);
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|error| AgentError::Other(format!("tokio runtime: {error}")))?
                    .block_on(async move {
                        backend
                            .run_turn_streaming(params, event_tx, bridge, timeout)
                            .await
                    })
            }
        }
    }
}

impl Deref for DesktopBackend {
    type Target = dyn AgentBackend;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Codex(backend) => backend.as_ref(),
            Self::Mitsuro(backend) => backend.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_backend_namespaced() {
        assert_eq!(
            BackendSessionId::new(BackendKind::MitsuroHttp, "abc").qualified(),
            "mitsuro-http:abc"
        );
        assert_eq!(
            BackendSessionId::new(BackendKind::CodexStdio, "abc").qualified(),
            "codex-stdio:abc"
        );
        assert_eq!(
            BackendSessionId::parse_qualified("mitsuro-http:abc").expect("qualified id"),
            BackendSessionId::new(BackendKind::MitsuroHttp, "abc")
        );
        let persisted = serde_json::to_string(&BackendSessionId::new(
            BackendKind::CodexStdio,
            "session-42",
        ))
        .expect("serialize session identity");
        let restored: BackendSessionId =
            serde_json::from_str(&persisted).expect("deserialize session identity");
        assert_eq!(restored.qualified(), "codex-stdio:session-42");
    }

    #[test]
    fn capabilities_do_not_claim_unsupported_cross_backend_features() {
        assert!(!BackendCapabilities::mitsuro().archive);
        assert!(!BackendCapabilities::codex().hive);
        assert!(BackendCapabilities::mitsuro().hive_mutations);
        assert!(!BackendCapabilities::codex().hive_mutations);
        assert!(BackendCapabilities::mitsuro().schedules);
        assert!(BackendCapabilities::mitsuro().schedule_mutations);
        assert!(!BackendCapabilities::codex().schedule_mutations);
        assert!(!BackendCapabilities::mitsuro().processes);
        assert!(BackendCapabilities::codex().command_exec);
        assert!(!BackendCapabilities::mitsuro().command_exec);
        assert!(BackendCapabilities::codex().thread_shell_commands);
        assert!(!BackendCapabilities::mitsuro().thread_shell_commands);
        assert!(BackendCapabilities::mitsuro().streaming_chat);
        assert!(BackendCapabilities::codex().streaming_chat);
        assert!(BackendCapabilities::mitsuro().image_attachments);
        assert!(BackendCapabilities::codex().image_attachments);
        assert!(BackendCapabilities::mitsuro().steering);
        assert!(BackendCapabilities::codex().steering);
        assert!(!BackendCapabilities::mitsuro().manual_compaction);
        assert!(BackendCapabilities::codex().manual_compaction);
        assert!(!BackendCapabilities::mitsuro().review);
        assert!(BackendCapabilities::codex().review);
        assert!(BackendCapabilities::codex().realtime_voice);
        assert!(!BackendCapabilities::mitsuro().realtime_voice);
        assert!(BackendCapabilities::codex().plugin_mutations);
        assert!(!BackendCapabilities::mitsuro().plugin_mutations);
        assert!(BackendCapabilities::codex().environment_add);
        assert!(!BackendCapabilities::mitsuro().environment_add);
        assert!(BackendCapabilities::codex().mcp_oauth);
        assert!(!BackendCapabilities::mitsuro().mcp_oauth);
        assert!(BackendCapabilities::codex().mcp_config_write);
        assert!(!BackendCapabilities::mitsuro().mcp_config_write);
        assert!(BackendCapabilities::codex().hooks);
        assert!(!BackendCapabilities::mitsuro().hooks);
        assert!(BackendCapabilities::codex().apps);
        assert!(!BackendCapabilities::mitsuro().apps);
        assert!(BackendCapabilities::codex().skill_config_write);
        assert!(!BackendCapabilities::mitsuro().skill_config_write);
        assert!(BackendCapabilities::codex().remote_control);
        assert!(!BackendCapabilities::mitsuro().remote_control);
        assert!(BackendCapabilities::codex().permission_profiles);
        assert!(!BackendCapabilities::mitsuro().permission_profiles);
        assert!(BackendCapabilities::codex().config_requirements);
        assert!(!BackendCapabilities::mitsuro().config_requirements);
        assert!(BackendCapabilities::codex().model_provider_capabilities);
        assert!(!BackendCapabilities::mitsuro().model_provider_capabilities);
        assert!(BackendCapabilities::codex().external_agent_import);
        assert!(!BackendCapabilities::mitsuro().external_agent_import);
        assert!(BackendCapabilities::codex().experimental_features);
        assert!(!BackendCapabilities::mitsuro().experimental_features);
        assert!(BackendCapabilities::codex().memory_settings);
        assert!(!BackendCapabilities::mitsuro().memory_settings);
        assert!(BackendCapabilities::codex().thread_settings);
        assert!(!BackendCapabilities::mitsuro().thread_settings);
        assert!(BackendCapabilities::codex().thread_metadata);
        assert!(!BackendCapabilities::mitsuro().thread_metadata);
        assert!(BackendCapabilities::codex().item_pagination);
        assert!(!BackendCapabilities::mitsuro().item_pagination);
        assert!(BackendCapabilities::codex().account_workspace_messages);
        assert!(!BackendCapabilities::mitsuro().account_workspace_messages);
        assert!(BackendCapabilities::codex().account_reset_credits);
        assert!(!BackendCapabilities::mitsuro().account_reset_credits);
        assert!(BackendCapabilities::codex().account_credit_nudge);
        assert!(!BackendCapabilities::mitsuro().account_credit_nudge);
        assert!(BackendCapabilities::codex().file_mutations);
        assert!(!BackendCapabilities::mitsuro().file_mutations);
        assert!(BackendCapabilities::codex().file_watches);
        assert!(!BackendCapabilities::mitsuro().file_watches);
        assert!(BackendCapabilities::codex().background_terminals);
        assert!(!BackendCapabilities::mitsuro().background_terminals);
        assert!(!BackendCapabilities::codex().tracked_process_kill);
        assert!(BackendCapabilities::mitsuro().tracked_process_kill);
        assert!(BackendCapabilities::codex().conversation_search);
        assert!(BackendCapabilities::mitsuro().conversation_search);
        assert!(BackendCapabilities::codex().paged_history);
        assert!(BackendCapabilities::mitsuro().paged_history);
        assert!(BackendCapabilities::codex().edit_latest_message);
        assert!(!BackendCapabilities::mitsuro().edit_latest_message);
        assert!(BackendCapabilities::codex().side_conversations);
        assert!(!BackendCapabilities::mitsuro().side_conversations);
    }

    #[tokio::test]
    async fn realtime_rejects_mitsuro_and_cross_backend_sessions_before_io() {
        let mitsuro = DesktopBackend::Mitsuro(Arc::new(MitsuroServerBackend::new()));
        let own_session = BackendSessionId::new(BackendKind::MitsuroHttp, "mitsuro-thread");
        let error = mitsuro
            .realtime_start(
                &own_session,
                ThreadRealtimeStartParams::websocket(
                    "ignored",
                    crate::RealtimeOutputModality::Audio,
                ),
            )
            .await
            .expect_err("Mitsuro realtime must be rejected");
        assert!(matches!(error, AgentError::NotImplemented(_)));

        let codex = DesktopBackend::Codex(Arc::new(CodexAppServerBackend::with_defaults()));
        let error = codex
            .realtime_stop(
                &own_session,
                ThreadRealtimeStopParams {
                    thread_id: "ignored".to_owned(),
                },
            )
            .await
            .expect_err("cross-backend session must be rejected before transport");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }

    #[tokio::test]
    async fn background_terminals_reject_cross_backend_sessions_before_io() {
        let codex = DesktopBackend::Codex(Arc::new(CodexAppServerBackend::with_defaults()));
        let mitsuro_session = BackendSessionId::new(BackendKind::MitsuroHttp, "mitsuro-thread");
        let error = codex
            .list_thread_background_terminals(
                &mitsuro_session,
                ThreadBackgroundTerminalsListParams::new("ignored"),
            )
            .await
            .expect_err("cross-backend session must be rejected before transport");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }

    #[tokio::test]
    async fn thread_shell_commands_reject_mitsuro_and_cross_backend_sessions_before_io() {
        let mitsuro = DesktopBackend::Mitsuro(Arc::new(MitsuroServerBackend::new()));
        let mitsuro_session = BackendSessionId::new(BackendKind::MitsuroHttp, "mitsuro-thread");
        let error = mitsuro
            .start_thread_shell_command(&mitsuro_session, "pwd")
            .await
            .expect_err("Mitsuro must reject Codex host-local shell commands");
        assert!(matches!(error, AgentError::NotImplemented(_)));

        let codex = DesktopBackend::Codex(Arc::new(CodexAppServerBackend::with_defaults()));
        let error = codex
            .start_thread_shell_command(&mitsuro_session, "pwd")
            .await
            .expect_err("cross-backend session must be rejected before transport");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }

    #[tokio::test]
    async fn thread_item_injection_rejects_mitsuro_and_cross_backend_sessions_before_io() {
        let mitsuro = DesktopBackend::Mitsuro(Arc::new(MitsuroServerBackend::new()));
        let mitsuro_session = BackendSessionId::new(BackendKind::MitsuroHttp, "mitsuro-thread");
        let items =
            ThreadInjectItemsParams::input_text_boundary("ignored", "Side conversation boundary.")
                .items;
        let error = mitsuro
            .inject_thread_items(&mitsuro_session, items.clone())
            .await
            .expect_err("Mitsuro must reject Codex model-history injection");
        assert!(matches!(error, AgentError::NotImplemented(_)));

        let codex = DesktopBackend::Codex(Arc::new(CodexAppServerBackend::with_defaults()));
        let error = codex
            .inject_thread_items(&mitsuro_session, items)
            .await
            .expect_err("cross-backend session must be rejected before transport");
        assert!(error.to_string().contains("belongs to mitsuro-http"));
    }
}
