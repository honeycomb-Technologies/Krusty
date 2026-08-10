//! Desktop-facing backend selection and capability boundary.

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    AgentBackend, AgentError, ApprovalChoice, CodexAppServerBackend, FsCopyParams, FsCopyResponse,
    FsCreateDirectoryParams, FsCreateDirectoryResponse, FsRemoveParams, FsRemoveResponse,
    FsUnwatchParams, FsUnwatchResponse, FsWatchParams, FsWatchResponse, FsWriteFileParams,
    FsWriteFileResponse, LifecycleNotification, LiveApprovalBridge, LiveTurnOutcome,
    McpServerConfigAddParams, McpServerOauthLoginParams, McpServerOauthLoginResponse,
    MitsuroServerBackend, PendingApproval, PluginInstallParams, PluginInstallResponse,
    PluginUninstallParams, PluginUninstallResponse, Result, ThreadRealtimeAppendAudioParams,
    ThreadRealtimeAppendAudioResponse, ThreadRealtimeAppendSpeechParams,
    ThreadRealtimeAppendSpeechResponse, ThreadRealtimeAppendTextParams,
    ThreadRealtimeAppendTextResponse, ThreadRealtimeListVoicesParams,
    ThreadRealtimeListVoicesResponse, ThreadRealtimeStartParams, ThreadRealtimeStartResponse,
    ThreadRealtimeStopParams, ThreadRealtimeStopResponse, TurnStartParams, TurnStreamEvent,
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
    pub extensions: bool,
    pub plugin_mutations: bool,
    pub environment_add: bool,
    pub mcp_oauth: bool,
    pub mcp_config_write: bool,
    pub hooks: bool,
    pub apps: bool,
    pub skill_config_write: bool,
    pub hive: bool,
    pub schedules: bool,
    pub sites: bool,
    pub archive: bool,
    pub fork: bool,
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
            extensions: true,
            plugin_mutations: true,
            environment_add: true,
            mcp_oauth: true,
            mcp_config_write: true,
            hooks: true,
            apps: true,
            skill_config_write: true,
            hive: false,
            schedules: false,
            sites: false,
            archive: true,
            fork: true,
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
            extensions: true,
            plugin_mutations: false,
            environment_add: false,
            mcp_oauth: false,
            mcp_config_write: false,
            hooks: false,
            apps: false,
            skill_config_write: false,
            hive: true,
            schedules: true,
            sites: false,
            archive: false,
            fork: false,
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
        if session.backend == self.kind() {
            return Ok(());
        }
        Err(AgentError::Other(format!(
            "session {} belongs to {}, but the active backend is {}",
            session.qualified(),
            session.backend.id(),
            self.kind().id()
        )))
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
        assert!(!BackendCapabilities::mitsuro().processes);
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
        assert!(BackendCapabilities::codex().file_mutations);
        assert!(!BackendCapabilities::mitsuro().file_mutations);
        assert!(BackendCapabilities::codex().file_watches);
        assert!(!BackendCapabilities::mitsuro().file_watches);
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
}
