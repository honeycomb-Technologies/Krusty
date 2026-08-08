//! Desktop-facing backend selection and capability boundary.

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    AgentBackend, AgentError, ApprovalChoice, CodexAppServerBackend, LiveApprovalBridge,
    LiveTurnOutcome, MitsuroServerBackend, PendingApproval, Result, TurnStartParams,
    TurnStreamEvent,
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
    pub approvals: bool,
    pub models: bool,
    pub files: bool,
    pub processes: bool,
    pub extensions: bool,
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
            approvals: true,
            models: true,
            files: true,
            processes: true,
            extensions: true,
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
            approvals: true,
            models: true,
            files: true,
            // The HTTP API can inspect/kill tracked background processes, but it
            // does not expose the interactive spawn/stdin/PTY contract used by
            // the native terminal panel.
            processes: false,
            extensions: true,
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
    }
}
