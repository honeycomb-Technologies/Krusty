//! Codex app-server backend: spawn + JSONL JSON-RPC over stdio.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tracing::{debug, warn};

use crate::account::{
    CancelLoginAccountParams, CancelLoginAccountResponse, GetAccountParams,
    GetAccountRateLimitsResponse, GetAccountResponse, GetAccountTokenUsageResponse,
    LoginAccountParams, LoginAccountResponse, LogoutAccountResponse,
};
use crate::approvals::{ApprovalChoice, PendingApproval};
use crate::backend::AgentBackend;
use crate::environment::{
    CollaborationModeListParams, CollaborationModeListResponse, EnvironmentAddParams,
    EnvironmentAddResponse, EnvironmentInfoParams, EnvironmentInfoResponse,
    EnvironmentStatusParams, EnvironmentStatusResponse,
};
use crate::extensions::{
    ListMcpServerStatusParams, ListMcpServerStatusResponse, McpServerToolCallParams,
    McpServerToolCallResponse, PluginInstalledParams, PluginInstalledResponse, PluginListParams,
    PluginListResponse, PluginReadParams, PluginReadResponse,
};
use crate::fs::{
    FsGetMetadataParams, FsGetMetadataResponse, FsReadDirectoryParams, FsReadDirectoryResponse,
    FsReadFileParams, FsReadFileResponse, FuzzyFileSearchParams, FuzzyFileSearchResponse,
    FuzzyFileSearchSessionStartParams, FuzzyFileSearchSessionStartResponse,
    FuzzyFileSearchSessionStopParams, FuzzyFileSearchSessionStopResponse,
    FuzzyFileSearchSessionUpdateParams, FuzzyFileSearchSessionUpdateResponse,
};
use crate::process::{
    ProcessKillParams, ProcessKillResponse, ProcessResizePtyParams, ProcessResizePtyResponse,
    ProcessSpawnParams, ProcessSpawnResponse, ProcessWriteStdinParams, ProcessWriteStdinResponse,
};
use crate::protocol::{
    map_notification_to_event, ClientInfo, ConfigReadParams, ConfigReadResponse,
    InitializeCapabilities, InitializeParams, InitializeResponse, JsonRpcId, JsonRpcMessage,
    JsonRpcRequest, ModelListParams, ModelListResponse, Notification, SkillsListParams,
    SkillsListResponse, ThreadArchiveParams, ThreadArchiveResponse, ThreadDeleteParams,
    ThreadDeleteResponse, ThreadForkParams, ThreadForkResponse, ThreadGoalClearParams,
    ThreadGoalClearResponse, ThreadGoalGetParams, ThreadGoalGetResponse, ThreadGoalSetParams,
    ThreadGoalSetResponse, ThreadListParams, ThreadListResponse, ThreadReadParams,
    ThreadReadResponse, ThreadResumeParams, ThreadResumeResponse, ThreadSearchParams,
    ThreadSearchResponse, ThreadSetNameParams, ThreadSetNameResponse, ThreadStartParams,
    ThreadStartResponse, ThreadUnarchiveParams, ThreadUnarchiveResponse, TurnInterruptParams,
    TurnInterruptResponse, TurnStartParams, TurnStartResponse,
};
use crate::realtime::{
    ThreadRealtimeAppendAudioParams, ThreadRealtimeAppendAudioResponse,
    ThreadRealtimeAppendSpeechParams, ThreadRealtimeAppendSpeechResponse,
    ThreadRealtimeAppendTextParams, ThreadRealtimeAppendTextResponse,
    ThreadRealtimeListVoicesParams, ThreadRealtimeListVoicesResponse, ThreadRealtimeStartParams,
    ThreadRealtimeStartResponse, ThreadRealtimeStopParams, ThreadRealtimeStopResponse,
};
use crate::server_requests::{automatic_server_response, AutomaticServerResponse};
use crate::types::{AgentError, ConnectionStatus, Result, TurnStreamEvent};

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_NAME: &str = "mitsuro";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CLIENT_TITLE: &str = "Mitsuro";

fn shared_pump_runtime() -> Arc<tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Arc<tokio::runtime::Runtime>> = OnceLock::new();
    Arc::clone(RUNTIME.get_or_init(|| {
        Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("mitsuro-appserver-pump")
                .build()
                .expect("mitsuro app-server pump runtime"),
        )
    }))
}

/// Configuration for spawning / talking to `codex app-server`.
#[derive(Debug, Clone)]
pub struct CodexAppServerConfig {
    /// Path to the `codex` binary. Resolved via [`resolve_codex_bin`] when `None`.
    pub codex_bin: Option<PathBuf>,
    /// Extra args after `app-server` (default includes `--stdio`).
    pub extra_args: Vec<String>,
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Client identity sent in `initialize`.
    pub client_info: ClientInfo,
    /// Capabilities negotiated at initialize.
    pub capabilities: Option<InitializeCapabilities>,
}

impl Default for CodexAppServerConfig {
    fn default() -> Self {
        Self {
            codex_bin: None,
            extra_args: vec!["--stdio".into()],
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            client_info: ClientInfo {
                name: CLIENT_NAME.into(),
                version: CLIENT_VERSION.into(),
                title: Some(CLIENT_TITLE.into()),
            },
            capabilities: Some(InitializeCapabilities {
                // The desktop renders process, environment, realtime, and
                // background-terminal workflows that Codex currently exposes
                // through the experimental contract. Negotiate that contract
                // explicitly instead of advertising controls that the default
                // handshake then rejects.
                experimental_api: Some(true),
                // We do not currently register client-owned extensions or
                // OpenAI form elicitations, and never request attestation.
                extensions: None,
                mcp_server_openai_form_elicitation: Some(false),
                request_attestation: Some(false),
                ..Default::default()
            }),
        }
    }
}

/// Resolve `CODEX_BIN`, then `~/.local/bin/codex`, then `codex` on PATH.
pub fn resolve_codex_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CODEX_BIN") {
        if !bin.is_empty() {
            return PathBuf::from(bin);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = Path::new(&home).join(".local/bin/codex");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("codex")
}

/// Returns `true` if a usable codex binary appears present.
pub fn codex_bin_available() -> bool {
    let bin = resolve_codex_bin();
    if bin.is_absolute() {
        return bin.is_file();
    }
    // PATH lookup
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let p = dir.join(&bin);
                p.is_file()
            })
        })
        .unwrap_or(false)
}

type PendingMap = HashMap<String, oneshot::Sender<std::result::Result<Value, AgentError>>>;

struct Inner {
    status: RwLock<ConnectionStatus>,
    next_id: AtomicU64,
    pending: Mutex<PendingMap>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    /// Application-lifetime notification hub. Every active turn gets its own
    /// receiver, so completing one turn cannot consume streaming for the next.
    notify_tx: broadcast::Sender<Notification>,
    init_response: RwLock<Option<InitializeResponse>>,
    /// When set, use this instead of spawning a process (unit tests).
    test_io: Mutex<Option<TestIo>>,
}

struct TestIo {
    writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
}

/// Live connection to a `codex app-server` child (or mock).
///
/// Owns a multi-thread **pump** runtime so stdout/stderr reader tasks outlive
/// any short-lived `current_thread` runtimes used by the desktop UI for
/// `block_on` RPC calls. Without this, bootstrap's temporary runtime drops
/// and kills the reader — subsequent `thread/read` / turns hang forever.
pub struct CodexAppServerBackend {
    config: CodexAppServerConfig,
    inner: Arc<Inner>,
    /// Long-lived runtime for I/O pumps + notification fan-out.
    pump_rt: Arc<tokio::runtime::Runtime>,
}

impl CodexAppServerBackend {
    pub fn new(config: CodexAppServerConfig) -> Self {
        let (notify_tx, _) = broadcast::channel(1_024);
        let pump_rt = shared_pump_runtime();
        Self {
            config,
            inner: Arc::new(Inner {
                status: RwLock::new(ConnectionStatus::Disconnected),
                next_id: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
                stdin: Mutex::new(None),
                child: Mutex::new(None),
                notify_tx,
                init_response: RwLock::new(None),
                test_io: Mutex::new(None),
            }),
            pump_rt,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(CodexAppServerConfig::default())
    }

    /// Drive a future on the long-lived pump runtime.
    ///
    /// Tokio process I/O (child stdin/stdout) is bound to the runtime that
    /// spawned the process. All live app-server RPC must run here — never on a
    /// short-lived `current_thread` runtime that drops after bootstrap.
    ///
    /// Safe to call from another Tokio runtime (uses spawn + channel).
    pub fn block_on<F>(&self, fut: F) -> F::Output
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            self.pump_rt.spawn(async move {
                let _ = tx.send(fut.await);
            });
            rx.recv().expect("mitsuro app-server pump task")
        } else {
            self.pump_rt.block_on(fut)
        }
    }

    /// Subscribe to the application-lifetime notification hub.
    ///
    /// Each call returns an independent receiver. Receivers observe events sent
    /// after subscription; a slow receiver may lag without stalling app-server I/O.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<Notification> {
        self.inner.notify_tx.subscribe()
    }

    /// Subscribe to raw notifications and map them into typed [`TurnStreamEvent`]s.
    ///
    /// Every caller receives an independent stream. The `Option` return is kept
    /// for source compatibility with the original prototype and is always `Some`.
    /// Server requests (including approvals) are forwarded as pseudo-notifications
    /// `serverRequest/<method>` and mapped to [`TurnStreamEvent::ApprovalRequested`] when applicable.
    pub async fn subscribe_turn_events(&self) -> Option<mpsc::UnboundedReceiver<TurnStreamEvent>> {
        let mut raw = self.subscribe_notifications();
        let (tx, rx) = mpsc::unbounded_channel();
        // Fan-out must live on the pump runtime (not a temporary UI block_on rt).
        self.pump_rt.spawn(async move {
            loop {
                match raw.recv().await {
                    Ok(n) => {
                        let ev = map_notification_to_event(&n.method, n.params.as_ref());
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "app-server notification subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Some(rx)
    }

    /// Subscribe only to normalized non-turn lifecycle notifications.
    ///
    /// Desktop shells keep this receiver alive for the full backend connection,
    /// so account, extension, and thread-list changes are observed even while no
    /// turn is running. Turn/item events remain on [`Self::subscribe_turn_events`]
    /// and are deliberately excluded to avoid replaying transcript deltas twice.
    pub fn subscribe_lifecycle_events(
        &self,
    ) -> mpsc::UnboundedReceiver<crate::notifications::LifecycleNotification> {
        let mut raw = self.subscribe_notifications();
        let (tx, rx) = mpsc::unbounded_channel();
        self.pump_rt.spawn(async move {
            loop {
                match raw.recv().await {
                    Ok(notification) => {
                        let event = map_notification_to_event(
                            &notification.method,
                            notification.params.as_ref(),
                        );
                        if let TurnStreamEvent::Lifecycle(event) = event {
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "app-server lifecycle subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        rx
    }

    /// Answer a server-originated JSON-RPC request with a raw `result` value.
    pub async fn respond_to_server_request(&self, id: JsonRpcId, result: Value) -> Result<()> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        let line = serde_json::to_string(&body)? + "\n";
        self.write_line(&line).await
    }

    /// Answer a server-originated JSON-RPC request with an explicit error.
    pub async fn respond_to_server_request_error(
        &self,
        id: JsonRpcId,
        error: crate::protocol::JsonRpcErrorBody,
    ) -> Result<()> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": error,
        });
        let line = serde_json::to_string(&body)? + "\n";
        self.write_line(&line).await
    }

    /// Approve or deny a pending approval (exec / patch) by writing the protocol response.
    pub async fn respond_approval(
        &self,
        pending: &PendingApproval,
        choice: ApprovalChoice,
    ) -> Result<()> {
        let result = crate::approvals::build_pending_approval_result(pending, choice);
        self.respond_to_server_request(pending.request_id.clone(), result)
            .await
    }

    /// Convenience: approve a pending approval request.
    pub async fn approve(&self, pending: &PendingApproval) -> Result<()> {
        self.respond_approval(pending, ApprovalChoice::Approve)
            .await
    }

    /// Convenience: reject a pending approval (agent may continue).
    pub async fn deny(&self, pending: &PendingApproval) -> Result<()> {
        self.respond_approval(pending, ApprovalChoice::Reject).await
    }

    /// Probe `account/read` for a usable auth account (no paid model call).
    ///
    /// Returns `true` when the server reports an account object. Failures / missing
    /// method → `false` (caller should use fixtures).
    pub async fn has_usable_auth(&self) -> bool {
        match self.account_read(GetAccountParams::default()).await {
            Ok(resp) => resp.has_account(),
            Err(_) => false,
        }
    }

    /// Typed `account/read` (no paid model call).
    pub async fn account_read(&self, params: GetAccountParams) -> Result<GetAccountResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("account/read", Some(value)).await
    }

    /// Typed `account/login/start`.
    pub async fn account_login_start(
        &self,
        params: LoginAccountParams,
    ) -> Result<LoginAccountResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("account/login/start", Some(value)).await
    }

    /// Typed `account/login/cancel`.
    pub async fn account_login_cancel(
        &self,
        params: CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("account/login/cancel", Some(value))
            .await
    }

    /// Typed `account/logout`.
    pub async fn account_logout(&self) -> Result<LogoutAccountResponse> {
        self.request_typed("account/logout", Some(serde_json::json!({})))
            .await
    }

    /// Typed `account/usage/read`.
    pub async fn account_usage_read(&self) -> Result<GetAccountTokenUsageResponse> {
        self.request_typed("account/usage/read", Some(serde_json::json!({})))
            .await
    }

    /// Typed `account/rateLimits/read`.
    pub async fn account_rate_limits_read(&self) -> Result<GetAccountRateLimitsResponse> {
        self.request_typed("account/rateLimits/read", Some(serde_json::json!({})))
            .await
    }

    /// List models via `model/list` (no paid turn).
    pub async fn list_models(&self, params: ModelListParams) -> Result<ModelListResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("model/list", Some(value)).await
    }

    /// Effective config via `config/read`.
    pub async fn read_config(&self, params: ConfigReadParams) -> Result<ConfigReadResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("config/read", Some(value)).await
    }

    /// Full-text / substring thread search via `thread/search`.
    pub async fn search_threads(&self, params: ThreadSearchParams) -> Result<ThreadSearchResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/search", Some(value)).await
    }

    /// Set user-facing thread title via `thread/name/set`.
    pub async fn set_thread_name(
        &self,
        params: ThreadSetNameParams,
    ) -> Result<ThreadSetNameResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/name/set", Some(value)).await
    }

    /// List skills via `skills/list` (best-effort; may fail if method unavailable).
    pub async fn list_skills(&self, params: SkillsListParams) -> Result<SkillsListResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("skills/list", Some(value)).await
    }

    /// List the voices available to the experimental thread realtime service.
    pub async fn realtime_list_voices(
        &self,
        params: ThreadRealtimeListVoicesParams,
    ) -> Result<ThreadRealtimeListVoicesResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/realtime/listVoices", Some(value))
            .await
    }

    /// Start an experimental thread-scoped realtime session.
    pub async fn realtime_start(
        &self,
        params: ThreadRealtimeStartParams,
    ) -> Result<ThreadRealtimeStartResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/realtime/start", Some(value))
            .await
    }

    /// Append PCM audio to an active realtime session.
    pub async fn realtime_append_audio(
        &self,
        params: ThreadRealtimeAppendAudioParams,
    ) -> Result<ThreadRealtimeAppendAudioResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/realtime/appendAudio", Some(value))
            .await
    }

    /// Append role-bearing text to an active realtime session.
    pub async fn realtime_append_text(
        &self,
        params: ThreadRealtimeAppendTextParams,
    ) -> Result<ThreadRealtimeAppendTextResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/realtime/appendText", Some(value))
            .await
    }

    /// Append text that should be spoken by an active realtime session.
    pub async fn realtime_append_speech(
        &self,
        params: ThreadRealtimeAppendSpeechParams,
    ) -> Result<ThreadRealtimeAppendSpeechResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/realtime/appendSpeech", Some(value))
            .await
    }

    /// Stop a thread-scoped realtime session.
    pub async fn realtime_stop(
        &self,
        params: ThreadRealtimeStopParams,
    ) -> Result<ThreadRealtimeStopResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/realtime/stop", Some(value))
            .await
    }

    /// Archive a thread via `thread/archive`.
    pub async fn archive_thread(
        &self,
        params: ThreadArchiveParams,
    ) -> Result<ThreadArchiveResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/archive", Some(value)).await
    }

    /// Unarchive a thread via `thread/unarchive`.
    pub async fn unarchive_thread(
        &self,
        params: ThreadUnarchiveParams,
    ) -> Result<ThreadUnarchiveResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/unarchive", Some(value)).await
    }

    /// Delete a thread via `thread/delete`.
    pub async fn delete_thread(&self, params: ThreadDeleteParams) -> Result<ThreadDeleteResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/delete", Some(value)).await
    }

    /// Fork a thread via `thread/fork`.
    pub async fn fork_thread(&self, params: ThreadForkParams) -> Result<ThreadForkResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/fork", Some(value)).await
    }

    /// Resume a thread via `thread/resume`.
    pub async fn resume_thread(&self, params: ThreadResumeParams) -> Result<ThreadResumeResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/resume", Some(value)).await
    }

    /// Interrupt an in-progress turn via `turn/interrupt`.
    pub async fn interrupt_turn(
        &self,
        params: TurnInterruptParams,
    ) -> Result<TurnInterruptResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("turn/interrupt", Some(value)).await
    }

    pub fn initialize_response(&self) -> Option<InitializeResponse> {
        self.inner.init_response.read().ok().and_then(|g| g.clone())
    }

    fn set_status(&self, status: ConnectionStatus) {
        if let Ok(mut g) = self.inner.status.write() {
            *g = status;
        }
    }

    fn id_key(id: &JsonRpcId) -> String {
        match id {
            JsonRpcId::Number(n) => format!("n:{n}"),
            JsonRpcId::String(s) => format!("s:{s}"),
        }
    }

    /// Generic JSON-RPC request with typed result deserialization.
    pub async fn request_typed<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T> {
        let value = self.request(method, params).await?;
        serde_json::from_value(value).map_err(AgentError::from)
    }

    /// Generic JSON-RPC request returning raw JSON result.
    ///
    /// Child stdin/stdout I/O always runs on [`Self::pump_rt`] so callers may
    /// await from any runtime (desktop `current_thread`, gpui workers, etc.).
    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let status = self.status();
        match &status {
            ConnectionStatus::Ready | ConnectionStatus::Connecting | ConnectionStatus::Fixture => {}
            ConnectionStatus::Disconnected => {
                return Err(AgentError::NotConnected);
            }
            ConnectionStatus::Error(e) => {
                return Err(AgentError::NotReady(e.clone()));
            }
        }

        let method = method.to_string();
        let timeout = self.config.request_timeout;
        let inner = Arc::clone(&self.inner);
        let pump = Arc::clone(&self.pump_rt);

        // Bridge: any-runtime await ← pump_rt does real I/O.
        let (tx, rx) = oneshot::channel();
        pump.spawn(async move {
            let outcome = Self::request_on_pump(inner, method, params, timeout).await;
            let _ = tx.send(outcome);
        });
        match rx.await {
            Ok(v) => v,
            Err(_) => Err(AgentError::ChannelClosed),
        }
    }

    /// Perform one JSON-RPC round-trip on the pump runtime (owns child I/O).
    async fn request_on_pump(
        inner: Arc<Inner>,
        method: String,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let id = {
            let n = inner.next_id.fetch_add(1, Ordering::Relaxed);
            JsonRpcId::Number(n as i64)
        };
        let req = JsonRpcRequest::new(id.clone(), method, params);
        let line = serde_json::to_string(&req)? + "\n";

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = inner.pending.lock().await;
            pending.insert(Self::id_key(&id), tx);
        }

        if let Err(e) = Self::write_line_inner(&inner, &line).await {
            let mut pending = inner.pending.lock().await;
            pending.remove(&Self::id_key(&id));
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err(AgentError::ChannelClosed),
            Err(_) => {
                let mut pending = inner.pending.lock().await;
                pending.remove(&Self::id_key(&id));
                Err(AgentError::Timeout(timeout))
            }
        }
    }

    async fn write_line(&self, line: &str) -> Result<()> {
        // Child stdin is owned by pump_rt; hop so callers need not share that runtime.
        let line = line.to_string();
        let inner = Arc::clone(&self.inner);
        let (tx, rx) = oneshot::channel();
        self.pump_rt.spawn(async move {
            let r = Self::write_line_inner(&inner, &line).await;
            let _ = tx.send(r);
        });
        match rx.await {
            Ok(v) => v,
            Err(_) => Err(AgentError::ChannelClosed),
        }
    }

    async fn write_line_inner(inner: &Inner, line: &str) -> Result<()> {
        // Prefer real child stdin
        {
            let mut stdin_guard = inner.stdin.lock().await;
            if let Some(stdin) = stdin_guard.as_mut() {
                stdin.write_all(line.as_bytes()).await?;
                stdin.flush().await?;
                return Ok(());
            }
        }
        // Test IO
        {
            let mut test = inner.test_io.lock().await;
            if let Some(io) = test.as_mut() {
                io.writer.write_all(line.as_bytes()).await?;
                io.writer.flush().await?;
                return Ok(());
            }
        }
        Err(AgentError::NotConnected)
    }

    /// Inject a mock duplex for unit tests (reader task must be started separately).
    #[doc(hidden)]
    pub async fn connect_with_mock_writer(
        &self,
        writer: impl tokio::io::AsyncWrite + Send + Unpin + 'static,
    ) {
        *self.inner.test_io.lock().await = Some(TestIo {
            writer: Box::new(writer),
        });
        self.set_status(ConnectionStatus::Connecting);
    }

    /// Feed a single stdout line into the dispatcher (unit tests).
    #[doc(hidden)]
    pub async fn inject_stdout_line(&self, line: &str) {
        Self::dispatch_line(&self.inner, line).await;
    }

    async fn dispatch_line(inner: &Inner, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let msg = match JsonRpcMessage::parse_line(line) {
            Ok(m) => m,
            Err(e) => {
                warn!("failed to parse app-server line: {e}; line={line}");
                return;
            }
        };
        match msg {
            JsonRpcMessage::Response(resp) => {
                let key = Self::id_key(&resp.id);
                let sender = {
                    let mut pending = inner.pending.lock().await;
                    pending.remove(&key)
                };
                if let Some(tx) = sender {
                    let outcome = if let Some(err) = resp.error {
                        Err(AgentError::Rpc {
                            code: err.code,
                            message: err.message,
                        })
                    } else {
                        Ok(resp.result.unwrap_or(Value::Null))
                    };
                    let _ = tx.send(outcome);
                } else {
                    debug!("unmatched response id={key}");
                }
            }
            JsonRpcMessage::Notification(n) => {
                let _ = inner.notify_tx.send(n);
            }
            JsonRpcMessage::ServerRequest { id, method, params } => {
                // Non-interactive client requests are answered in the pump. A
                // turn must not depend on the UI event loop to make progress.
                let current_time_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if let Some(automatic) = automatic_server_response(&method, current_time_at) {
                    let response = match automatic {
                        AutomaticServerResponse::Result(result) => serde_json::json!({
                            "jsonrpc": "2.0", "id": id, "result": result,
                        }),
                        AutomaticServerResponse::Error(error) => serde_json::json!({
                            "jsonrpc": "2.0", "id": id, "error": error,
                        }),
                    };
                    match serde_json::to_string(&response) {
                        Ok(mut line) => {
                            line.push('\n');
                            if let Err(error) = Self::write_line_inner(inner, &line).await {
                                warn!(method, "failed to answer server request: {error}");
                            }
                        }
                        Err(error) => warn!(method, "failed to encode server response: {error}"),
                    }
                    return;
                }
                // Surface as pseudo-notification so `subscribe_notifications` and
                // `subscribe_turn_events` both see approvals (mapped via
                // `map_notification_to_event` → ApprovalRequested).
                let _ = inner.notify_tx.send(Notification {
                    method: format!("serverRequest/{method}"),
                    params: Some(serde_json::json!({
                        "id": id,
                        "params": params,
                    })),
                    emitted_at_ms: None,
                });
            }
            JsonRpcMessage::Unknown(v) => {
                debug!("unknown app-server message: {v}");
            }
        }
    }

    async fn spawn_and_handshake(&self) -> Result<InitializeResponse> {
        self.set_status(ConnectionStatus::Connecting);

        let bin = self
            .config
            .codex_bin
            .clone()
            .unwrap_or_else(resolve_codex_bin);

        let mut args = vec!["app-server".to_string()];
        args.extend(self.config.extra_args.clone());

        let mut child = Command::new(&bin)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                AgentError::Spawn(format!(
                    "failed to spawn `{} app-server`: {e}",
                    bin.display()
                ))
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AgentError::Spawn("child stdout missing".into()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AgentError::Spawn("child stdin missing".into()))?;
        let stderr = child.stderr.take();

        *self.inner.stdin.lock().await = Some(stdin);
        *self.inner.child.lock().await = Some(child);

        // stdout reader — same runtime that owns the child (must be pump_rt via block_on).
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => Self::dispatch_line(&inner, &line).await,
                    Err(e) => {
                        warn!("app-server stdout read error: {e}");
                        break;
                    }
                }
            }
            // Fail pending requests
            let mut pending = inner.pending.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(AgentError::ChannelClosed));
            }
            if let Ok(mut st) = inner.status.write() {
                if matches!(*st, ConnectionStatus::Ready | ConnectionStatus::Connecting) {
                    *st = ConnectionStatus::Error("app-server stdout closed".into());
                }
            }
        });

        // stderr drain (avoid pipe fill).
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let t = line.trim();
                            if !t.is_empty() {
                                debug!("app-server stderr: {t}");
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // initialize handshake
        let params = InitializeParams {
            client_info: self.config.client_info.clone(),
            capabilities: self.config.capabilities.clone(),
        };
        let params_value = serde_json::to_value(params)?;
        let result = self
            .request("initialize", Some(params_value))
            .await
            .inspect_err(|e| {
                self.set_status(ConnectionStatus::Error(e.to_string()));
            })?;

        let init: InitializeResponse = serde_json::from_value(result)?;
        if let Ok(mut g) = self.inner.init_response.write() {
            *g = Some(init.clone());
        }
        self.set_status(ConnectionStatus::Ready);
        Ok(init)
    }

    /// Mark ready without full spawn (after mock inject + manual initialize).
    #[doc(hidden)]
    pub fn mark_ready_for_test(&self, init: InitializeResponse) {
        if let Ok(mut g) = self.inner.init_response.write() {
            *g = Some(init);
        }
        self.set_status(ConnectionStatus::Ready);
    }
}

#[async_trait]
impl AgentBackend for CodexAppServerBackend {
    fn name(&self) -> &'static str {
        "codex-app-server"
    }

    fn status(&self) -> ConnectionStatus {
        self.inner
            .status
            .read()
            .map(|s| s.clone())
            .unwrap_or(ConnectionStatus::Disconnected)
    }

    fn supports_method(&self, method: &str) -> bool {
        // Live transport accepts any method string; advertise the bar registry.
        crate::methods::is_known_client_method(method)
    }

    /// Forward any method string over the existing JSON-RPC request path.
    async fn call_raw(&self, method: &str, params: Value) -> Result<Value> {
        self.request(method, Some(params)).await
    }

    async fn connect(&self) -> Result<InitializeResponse> {
        // If already ready, return cached init
        if matches!(self.status(), ConnectionStatus::Ready) {
            if let Some(init) = self.initialize_response() {
                return Ok(init);
            }
        }
        self.spawn_and_handshake().await
    }

    async fn thread_list(&self, params: ThreadListParams) -> Result<ThreadListResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/list", Some(value)).await
    }

    async fn thread_start(&self, params: ThreadStartParams) -> Result<ThreadStartResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/start", Some(value)).await
    }

    async fn thread_read(&self, params: ThreadReadParams) -> Result<ThreadReadResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/read", Some(value)).await
    }

    async fn model_list(&self, params: ModelListParams) -> Result<ModelListResponse> {
        self.list_models(params).await
    }

    async fn config_read(&self, params: ConfigReadParams) -> Result<ConfigReadResponse> {
        self.read_config(params).await
    }

    async fn thread_search(&self, params: ThreadSearchParams) -> Result<ThreadSearchResponse> {
        self.search_threads(params).await
    }

    async fn thread_name_set(&self, params: ThreadSetNameParams) -> Result<ThreadSetNameResponse> {
        self.set_thread_name(params).await
    }

    async fn thread_archive(&self, params: ThreadArchiveParams) -> Result<ThreadArchiveResponse> {
        self.archive_thread(params).await
    }

    async fn thread_unarchive(
        &self,
        params: ThreadUnarchiveParams,
    ) -> Result<ThreadUnarchiveResponse> {
        self.unarchive_thread(params).await
    }

    async fn thread_delete(&self, params: ThreadDeleteParams) -> Result<ThreadDeleteResponse> {
        self.delete_thread(params).await
    }

    async fn thread_fork(&self, params: ThreadForkParams) -> Result<ThreadForkResponse> {
        self.fork_thread(params).await
    }

    async fn thread_resume(&self, params: ThreadResumeParams) -> Result<ThreadResumeResponse> {
        self.resume_thread(params).await
    }

    async fn thread_compact_start(
        &self,
        params: crate::protocol::ThreadCompactStartParams,
    ) -> Result<crate::protocol::ThreadCompactStartResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/compact/start", Some(value))
            .await
    }

    async fn review_start(
        &self,
        params: crate::protocol::ReviewStartParams,
    ) -> Result<crate::protocol::ReviewStartResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("review/start", Some(value)).await
    }

    async fn thread_goal_get(&self, params: ThreadGoalGetParams) -> Result<ThreadGoalGetResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/goal/get", Some(value)).await
    }

    async fn thread_goal_set(&self, params: ThreadGoalSetParams) -> Result<ThreadGoalSetResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/goal/set", Some(value)).await
    }

    async fn thread_goal_clear(
        &self,
        params: ThreadGoalClearParams,
    ) -> Result<ThreadGoalClearResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("thread/goal/clear", Some(value)).await
    }

    async fn skills_list(&self, params: SkillsListParams) -> Result<SkillsListResponse> {
        self.list_skills(params).await
    }

    async fn turn_start(&self, params: TurnStartParams) -> Result<TurnStartResponse> {
        // Live model turn — callers must opt in; default UI path uses fixtures.
        // `params.model` is serialized as camelCase `model` when set (UI selected model).
        let value = serde_json::to_value(params)?;
        self.request_typed("turn/start", Some(value)).await
    }

    async fn turn_steer(
        &self,
        params: crate::protocol::TurnSteerParams,
    ) -> Result<crate::protocol::TurnSteerResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("turn/steer", Some(value)).await
    }

    async fn turn_interrupt(&self, params: TurnInterruptParams) -> Result<TurnInterruptResponse> {
        self.interrupt_turn(params).await
    }

    async fn process_spawn(&self, params: ProcessSpawnParams) -> Result<ProcessSpawnResponse> {
        let handle = params.process_handle.clone();
        let value = serde_json::to_value(params)?;
        let mut resp: ProcessSpawnResponse =
            self.request_typed("process/spawn", Some(value)).await?;
        // Wire response is empty; echo the client handle for UI convenience.
        if resp.process_handle.is_none() {
            resp.process_handle = Some(handle);
        }
        Ok(resp)
    }

    async fn process_write_stdin(
        &self,
        params: ProcessWriteStdinParams,
    ) -> Result<ProcessWriteStdinResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("process/writeStdin", Some(value)).await
    }

    async fn process_resize_pty(
        &self,
        params: ProcessResizePtyParams,
    ) -> Result<ProcessResizePtyResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("process/resizePty", Some(value)).await
    }

    async fn process_kill(&self, params: ProcessKillParams) -> Result<ProcessKillResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("process/kill", Some(value)).await
    }

    async fn fs_read_directory(
        &self,
        params: FsReadDirectoryParams,
    ) -> Result<FsReadDirectoryResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fs/readDirectory", Some(value)).await
    }

    async fn fs_read_file(&self, params: FsReadFileParams) -> Result<FsReadFileResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fs/readFile", Some(value)).await
    }

    async fn fs_get_metadata(&self, params: FsGetMetadataParams) -> Result<FsGetMetadataResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fs/getMetadata", Some(value)).await
    }

    async fn fuzzy_file_search(
        &self,
        params: FuzzyFileSearchParams,
    ) -> Result<FuzzyFileSearchResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fuzzyFileSearch", Some(value)).await
    }

    async fn fuzzy_file_search_session_start(
        &self,
        params: FuzzyFileSearchSessionStartParams,
    ) -> Result<FuzzyFileSearchSessionStartResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fuzzyFileSearch/sessionStart", Some(value))
            .await
    }

    async fn fuzzy_file_search_session_update(
        &self,
        params: FuzzyFileSearchSessionUpdateParams,
    ) -> Result<FuzzyFileSearchSessionUpdateResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fuzzyFileSearch/sessionUpdate", Some(value))
            .await
    }

    async fn fuzzy_file_search_session_stop(
        &self,
        params: FuzzyFileSearchSessionStopParams,
    ) -> Result<FuzzyFileSearchSessionStopResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("fuzzyFileSearch/sessionStop", Some(value))
            .await
    }

    async fn mcp_server_status_list(
        &self,
        params: ListMcpServerStatusParams,
    ) -> Result<ListMcpServerStatusResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("mcpServerStatus/list", Some(value))
            .await
    }

    async fn mcp_server_tool_call(
        &self,
        params: McpServerToolCallParams,
    ) -> Result<McpServerToolCallResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("mcpServer/tool/call", Some(value)).await
    }

    async fn plugin_list(&self, params: PluginListParams) -> Result<PluginListResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("plugin/list", Some(value)).await
    }

    async fn plugin_read(&self, params: PluginReadParams) -> Result<PluginReadResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("plugin/read", Some(value)).await
    }

    async fn plugin_installed(
        &self,
        params: PluginInstalledParams,
    ) -> Result<PluginInstalledResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("plugin/installed", Some(value)).await
    }

    async fn environment_info(
        &self,
        params: EnvironmentInfoParams,
    ) -> Result<EnvironmentInfoResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("environment/info", Some(value)).await
    }

    async fn environment_status(
        &self,
        params: EnvironmentStatusParams,
    ) -> Result<EnvironmentStatusResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("environment/status", Some(value)).await
    }

    async fn environment_add(
        &self,
        params: EnvironmentAddParams,
    ) -> Result<EnvironmentAddResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("environment/add", Some(value)).await
    }

    async fn collaboration_mode_list(
        &self,
        params: CollaborationModeListParams,
    ) -> Result<CollaborationModeListResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("collaborationMode/list", Some(value))
            .await
    }

    async fn account_read(&self, params: GetAccountParams) -> Result<GetAccountResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("account/read", Some(value)).await
    }

    async fn account_login_start(
        &self,
        params: LoginAccountParams,
    ) -> Result<LoginAccountResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("account/login/start", Some(value)).await
    }

    async fn account_login_cancel(
        &self,
        params: CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse> {
        let value = serde_json::to_value(params)?;
        self.request_typed("account/login/cancel", Some(value))
            .await
    }

    async fn account_logout(&self) -> Result<LogoutAccountResponse> {
        self.request_typed("account/logout", Some(serde_json::json!({})))
            .await
    }

    async fn account_usage_read(&self) -> Result<GetAccountTokenUsageResponse> {
        self.request_typed("account/usage/read", Some(serde_json::json!({})))
            .await
    }

    async fn account_rate_limits_read(&self) -> Result<GetAccountRateLimitsResponse> {
        self.request_typed("account/rateLimits/read", Some(serde_json::json!({})))
            .await
    }

    async fn disconnect(&self) -> Result<()> {
        {
            let mut stdin = self.inner.stdin.lock().await;
            *stdin = None;
        }
        {
            let mut child_guard = self.inner.child.lock().await;
            if let Some(mut child) = child_guard.take() {
                let _ = child.kill().await;
            }
        }
        {
            let mut test = self.inner.test_io.lock().await;
            *test = None;
        }
        {
            let mut pending = self.inner.pending.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(AgentError::ChannelClosed));
            }
        }
        self.set_status(ConnectionStatus::Disconnected);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::ApprovalKind;
    use crate::protocol::JsonRpcMessage;
    use tokio::io::{duplex, AsyncReadExt};

    #[test]
    fn classifies_response_and_notification() {
        let resp = JsonRpcMessage::parse_line(
            r#"{"id":1,"result":{"userAgent":"x","codexHome":"/c","platformFamily":"unix","platformOs":"linux"}}"#,
        )
        .unwrap();
        assert!(matches!(resp, JsonRpcMessage::Response(_)));

        let note = JsonRpcMessage::parse_line(
            r#"{"method":"thread/started","params":{"thread":{}},"emittedAtMs":1}"#,
        )
        .unwrap();
        assert!(matches!(note, JsonRpcMessage::Notification(n) if n.method == "thread/started"));

        let err = JsonRpcMessage::parse_line(
            r#"{"id":2,"error":{"code":-32601,"message":"Method not found"}}"#,
        )
        .unwrap();
        assert!(matches!(err, JsonRpcMessage::Response(r) if r.error.is_some()));
    }

    #[test]
    fn classifies_exec_command_approval_server_request() {
        let msg = JsonRpcMessage::parse_line(
            r#"{"id":9,"method":"execCommandApproval","params":{"conversationId":"c","callId":"x","command":["true"],"cwd":"/","parsedCmd":[]}}"#,
        )
        .unwrap();
        let JsonRpcMessage::ServerRequest { id, method, params } = msg else {
            panic!("expected ServerRequest");
        };
        assert_eq!(method, "execCommandApproval");
        let pending =
            crate::approvals::parse_approval_request(id, &method, params.as_ref()).unwrap();
        assert_eq!(pending.kind, ApprovalKind::ExecCommand);
        assert_eq!(pending.summary, "true");
    }

    #[tokio::test]
    async fn respond_approval_writes_expected_shape() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = CodexAppServerBackend::with_defaults();
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        let pending = PendingApproval {
            request_id: JsonRpcId::Number(77),
            method: "item/commandExecution/requestApproval".into(),
            kind: ApprovalKind::CommandExecution,
            title: "Approve command".into(),
            summary: "echo hi".into(),
            detail: String::new(),
            thread_id: Some("t".into()),
            turn_id: Some("u".into()),
            raw_params: serde_json::json!({}),
        };

        let reader = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut acc = String::new();
            loop {
                let n = server_reader.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains('\n') {
                    break;
                }
            }
            acc
        });

        backend.approve(&pending).await.unwrap();
        let line = reader.await.unwrap();
        let line = line.lines().next().unwrap();
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["id"], 77);
        assert_eq!(v["result"]["decision"], "accept");
    }

    #[tokio::test]
    async fn turn_steer_uses_expected_turn_precondition() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        let responder = Arc::clone(&backend);
        let server = tokio::spawn(async move {
            let mut line = String::new();
            BufReader::new(&mut server_reader)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(request["method"], "turn/steer");
            assert_eq!(request["params"]["threadId"], "thread-1");
            assert_eq!(request["params"]["expectedTurnId"], "turn-1");
            assert_eq!(request["params"]["input"][0]["text"], "focus on tests");
            responder
                .inject_stdout_line(
                    &serde_json::json!({
                        "id": request["id"],
                        "result": {"turnId": "turn-1"}
                    })
                    .to_string(),
                )
                .await;
        });

        let response = backend
            .turn_steer(crate::protocol::TurnSteerParams::text(
                "thread-1",
                "turn-1",
                "focus on tests",
            ))
            .await
            .unwrap();
        assert_eq!(response.turn_id, "turn-1");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn realtime_methods_use_the_generated_experimental_contract() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        let responder = Arc::clone(&backend);
        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(&mut server_reader);
            for expected in [
                "thread/realtime/listVoices",
                "thread/realtime/start",
                "thread/realtime/appendAudio",
                "thread/realtime/stop",
            ] {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let request: Value = serde_json::from_str(line.trim()).unwrap();
                assert_eq!(request["method"], expected);
                match expected {
                    "thread/realtime/start" => {
                        assert_eq!(request["params"]["threadId"], "thread-live");
                        assert_eq!(request["params"]["outputModality"], "audio");
                        assert_eq!(request["params"]["transport"]["type"], "websocket");
                        assert_eq!(request["params"]["version"], "v3");
                        assert_eq!(request["params"]["voice"], "sol");
                    }
                    "thread/realtime/appendAudio" => {
                        assert_eq!(request["params"]["threadId"], "thread-live");
                        assert_eq!(request["params"]["audio"]["data"], "AQI=");
                        assert_eq!(request["params"]["audio"]["numChannels"], 1);
                        assert_eq!(request["params"]["audio"]["sampleRate"], 24_000);
                    }
                    _ => {}
                }
                let result = if expected == "thread/realtime/listVoices" {
                    serde_json::json!({
                        "voices": {
                            "defaultV1": "alloy",
                            "defaultV2": "sol",
                            "v1": ["alloy"],
                            "v2": ["sol", "marin"]
                        }
                    })
                } else {
                    serde_json::json!({})
                };
                responder
                    .inject_stdout_line(
                        &serde_json::json!({
                            "id": request["id"],
                            "result": result
                        })
                        .to_string(),
                    )
                    .await;
            }
        });

        let voices = backend
            .realtime_list_voices(ThreadRealtimeListVoicesParams::default())
            .await
            .unwrap();
        assert_eq!(voices.voices.default_v2, crate::RealtimeVoice::Sol);

        let mut start = ThreadRealtimeStartParams::websocket(
            "thread-live",
            crate::RealtimeOutputModality::Audio,
        );
        start.voice = Some(crate::RealtimeVoice::Sol);
        backend.realtime_start(start).await.unwrap();
        backend
            .realtime_append_audio(ThreadRealtimeAppendAudioParams {
                thread_id: "thread-live".to_owned(),
                audio: crate::ThreadRealtimeAudioChunk {
                    data: "AQI=".to_owned(),
                    num_channels: 1,
                    sample_rate: 24_000,
                    samples_per_channel: Some(1),
                    item_id: None,
                },
            })
            .await
            .unwrap();
        backend
            .realtime_stop(ThreadRealtimeStopParams {
                thread_id: "thread-live".to_owned(),
            })
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn browser_login_keeps_async_completion_on_lifecycle_stream() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });
        let mut lifecycle = backend.subscribe_lifecycle_events();

        let responder = Arc::clone(&backend);
        let server = tokio::spawn(async move {
            let mut line = String::new();
            BufReader::new(&mut server_reader)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(request["method"], "account/login/start");
            assert_eq!(request["params"]["type"], "chatgpt");
            assert_eq!(request["params"]["appBrand"], "codex");
            responder
                .inject_stdout_line(
                    &serde_json::json!({
                        "id": request["id"],
                        "result": {
                            "type": "chatgpt",
                            "loginId": "login-live",
                            "authUrl": "https://auth.openai.com/authorize"
                        }
                    })
                    .to_string(),
                )
                .await;
            responder
                .inject_stdout_line(
                    r#"{"method":"account/login/completed","params":{"loginId":"login-live","success":true,"error":null}}"#,
                )
                .await;
        });

        let login = backend
            .account_login_start(LoginAccountParams::chatgpt())
            .await
            .unwrap();
        assert_eq!(login.login_id(), Some("login-live"));
        let completed = tokio::time::timeout(Duration::from_secs(1), lifecycle.recv())
            .await
            .expect("login completion timeout")
            .expect("login completion event");
        assert_eq!(completed.method, "account/login/completed");
        assert_eq!(
            completed
                .params
                .as_ref()
                .and_then(|params| params.get("success"))
                .and_then(Value::as_bool),
            Some(true)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn current_time_server_request_is_answered_without_ui_round_trip() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = CodexAppServerBackend::with_defaults();
        backend.connect_with_mock_writer(client_writer).await;

        backend
            .inject_stdout_line(
                r#"{"id":"clock-1","method":"currentTime/read","params":{"threadId":"thread-1"}}"#,
            )
            .await;

        let mut line = String::new();
        let mut reader = BufReader::new(&mut server_reader);
        tokio::time::timeout(Duration::from_secs(1), reader.read_line(&mut line))
            .await
            .expect("clock response timeout")
            .expect("clock response readable");
        let value: Value = serde_json::from_str(line.trim()).expect("valid JSON-RPC response");
        assert_eq!(value["id"], "clock-1");
        assert!(value["result"]["currentTimeAt"].as_u64().is_some());
    }

    #[tokio::test]
    async fn unsupported_client_owned_requests_cannot_stall_the_turn() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = CodexAppServerBackend::with_defaults();
        backend.connect_with_mock_writer(client_writer).await;

        backend
            .inject_stdout_line(
                r#"{"id":"tool-1","method":"item/tool/call","params":{"tool":"missing"}}"#,
            )
            .await;
        backend
            .inject_stdout_line(
                r#"{"id":"auth-1","method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}}"#,
            )
            .await;

        let mut reader = BufReader::new(&mut server_reader);
        let mut tool_line = String::new();
        let mut auth_line = String::new();
        tokio::time::timeout(Duration::from_secs(1), reader.read_line(&mut tool_line))
            .await
            .expect("dynamic tool response timeout")
            .expect("dynamic tool response readable");
        tokio::time::timeout(Duration::from_secs(1), reader.read_line(&mut auth_line))
            .await
            .expect("auth response timeout")
            .expect("auth response readable");

        let tool: Value = serde_json::from_str(tool_line.trim()).unwrap();
        assert_eq!(tool["id"], "tool-1");
        assert_eq!(tool["result"]["success"], false);
        let auth: Value = serde_json::from_str(auth_line.trim()).unwrap();
        assert_eq!(auth["id"], "auth-1");
        assert_eq!(auth["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn mock_stdio_initialize_and_thread_list() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = CodexAppServerBackend::with_defaults();
        backend.connect_with_mock_writer(client_writer).await;

        // Server task: read requests, write canned responses
        let backend_reader = Arc::new(backend);
        let backend_writer = Arc::clone(&backend_reader);

        let server = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            let mut acc = String::new();
            // read first line (initialize)
            loop {
                let n = server_reader.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains('\n') {
                    break;
                }
            }
            let line = acc.lines().next().unwrap();
            let req: Value = serde_json::from_str(line).unwrap();
            assert_eq!(req["method"], "initialize");
            let id = req["id"].clone();
            let resp = serde_json::json!({
                "id": id,
                "result": {
                    "userAgent": "mitsuro-test/0.0.0",
                    "codexHome": "/tmp/codex-home",
                    "platformFamily": "unix",
                    "platformOs": "linux"
                }
            });
            // Deliver via inject (simulates stdout reader)
            backend_writer.inject_stdout_line(&resp.to_string()).await;
        });

        // Drive initialize manually for mock path
        let init_params = serde_json::to_value(InitializeParams {
            client_info: ClientInfo {
                name: "mitsuro".into(),
                version: "0.1.0".into(),
                title: Some("Mitsuro".into()),
            },
            capabilities: None,
        })
        .unwrap();

        // request() allows Connecting status
        let result = backend_reader
            .request("initialize", Some(init_params))
            .await
            .expect("initialize");
        let init: InitializeResponse = serde_json::from_value(result).unwrap();
        assert_eq!(init.platform_os, "linux");
        backend_reader.mark_ready_for_test(init);

        server.await.unwrap();

        // Second request: thread/list
        let (client_writer2, mut server_reader2) = duplex(64 * 1024);
        // Replace writer
        *backend_reader.inner.test_io.lock().await = Some(TestIo {
            writer: Box::new(client_writer2),
        });

        let backend_list = Arc::clone(&backend_reader);
        let server2 = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            let mut acc = String::new();
            loop {
                let n = server_reader2.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains('\n') {
                    break;
                }
            }
            let line = acc.lines().next().unwrap();
            let req: Value = serde_json::from_str(line).unwrap();
            assert_eq!(req["method"], "thread/list");
            let id = req["id"].clone();
            let resp = serde_json::json!({
                "id": id,
                "result": {
                    "data": [{
                        "id": "thread-1",
                        "name": "Hello",
                        "preview": "hi",
                        "cwd": "/tmp",
                        "createdAt": 1,
                        "updatedAt": 2,
                        "modelProvider": "openai",
                        "ephemeral": false,
                        "isPinned": false
                    }],
                    "nextCursor": null
                }
            });
            backend_list.inject_stdout_line(&resp.to_string()).await;
        });

        let list = backend_reader
            .thread_list(ThreadListParams {
                limit: Some(10),
                ..Default::default()
            })
            .await
            .expect("thread/list");
        let threads = list.threads();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "thread-1");
        assert_eq!(threads[0].display_title(), "Hello");

        server2.await.unwrap();
    }

    #[tokio::test]
    async fn notification_subscription_receives_events() {
        let backend = CodexAppServerBackend::with_defaults();
        let mut first = backend.subscribe_notifications();
        let mut second = backend.subscribe_notifications();
        backend
            .inject_stdout_line(
                r#"{"method":"remoteControl/status/changed","params":{"status":"disabled"},"emittedAtMs":42}"#,
            )
            .await;
        let note = tokio::time::timeout(Duration::from_secs(1), first.recv())
            .await
            .expect("timeout")
            .expect("note");
        assert_eq!(note.method, "remoteControl/status/changed");
        assert_eq!(note.emitted_at_ms, Some(42));

        let same_note = tokio::time::timeout(Duration::from_secs(1), second.recv())
            .await
            .expect("timeout")
            .expect("note");
        assert_eq!(same_note.method, note.method);
        assert_eq!(same_note.params, note.params);
        assert_eq!(same_note.emitted_at_ms, note.emitted_at_ms);

        drop(first);
        let mut later_turn = backend.subscribe_notifications();
        backend
            .inject_stdout_line(r#"{"method":"turn/started","params":{"threadId":"t2"}}"#)
            .await;
        let later_note = tokio::time::timeout(Duration::from_secs(1), later_turn.recv())
            .await
            .expect("timeout")
            .expect("later turn note");
        assert_eq!(later_note.method, "turn/started");
    }

    #[tokio::test]
    async fn lifecycle_subscription_filters_turn_events_and_survives_idle_time() {
        let backend = CodexAppServerBackend::with_defaults();
        let mut lifecycle = backend.subscribe_lifecycle_events();

        backend
            .inject_stdout_line(
                r#"{"method":"item/agentMessage/delta","params":{"threadId":"t1","delta":"hi"}}"#,
            )
            .await;
        backend
            .inject_stdout_line(r#"{"method":"account/updated","params":{"message":"signed in"}}"#)
            .await;

        let event = tokio::time::timeout(Duration::from_secs(1), lifecycle.recv())
            .await
            .expect("timeout")
            .expect("lifecycle event");
        assert_eq!(event.method, "account/updated");
        assert_eq!(event.detail, "signed in");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::backend::AgentBackend;

    fn should_run_integration() -> bool {
        if std::env::var_os("MITSURO_RUN_APP_SERVER_IT").is_some() {
            return codex_bin_available();
        }
        // Also run when binary is present and not CI-skipping
        codex_bin_available()
    }

    #[tokio::test]
    async fn real_app_server_initialize_and_thread_list() {
        if !should_run_integration() {
            eprintln!("skip: codex binary not available");
            return;
        }

        let backend = CodexAppServerBackend::with_defaults();
        let init = backend.connect().await.expect("connect/initialize");
        assert!(!init.codex_home.is_empty());
        assert_eq!(init.platform_os, "linux");
        assert!(matches!(backend.status(), ConnectionStatus::Ready));

        let list = backend
            .thread_list(ThreadListParams {
                limit: Some(3),
                use_state_db_only: Some(true),
                ..Default::default()
            })
            .await
            .expect("thread/list");
        // data may be empty but must deserialize
        let _ = list.threads();

        // ephemeral thread/start — no model turn
        let started = backend
            .thread_start(ThreadStartParams {
                cwd: Some("/tmp".into()),
                ephemeral: Some(true),
                ..Default::default()
            })
            .await
            .expect("thread/start");
        let summary = started.summary();
        assert!(!summary.id.is_empty());
        assert_eq!(summary.ephemeral, Some(true));

        // Optional thread/read (may fail if thread not fully persisted — ignore soft errors)
        let _ = backend
            .thread_read(crate::protocol::ThreadReadParams {
                thread_id: summary.id.clone(),
                include_turns: Some(true),
            })
            .await;

        backend.disconnect().await.expect("disconnect");
        assert!(matches!(backend.status(), ConnectionStatus::Disconnected));
    }

    /// Live `turn/start` hits paid models — only runs with explicit opt-in + auth.
    #[tokio::test]
    async fn real_app_server_turn_start_skipped_without_opt_in() {
        if std::env::var_os("MITSURO_ALLOW_LIVE_TURN").is_none() {
            eprintln!("skip: set MITSURO_ALLOW_LIVE_TURN=1 to exercise live turn/start");
            return;
        }
        if !should_run_integration() {
            eprintln!("skip: codex binary not available");
            return;
        }

        let backend = CodexAppServerBackend::with_defaults();
        backend.connect().await.expect("connect");
        if !backend.has_usable_auth().await {
            eprintln!("skip: no usable auth for live turn/start");
            let _ = backend.disconnect().await;
            return;
        }

        let started = backend
            .thread_start(ThreadStartParams {
                cwd: Some("/tmp".into()),
                ephemeral: Some(true),
                ..Default::default()
            })
            .await
            .expect("thread/start");
        let thread_id = started.summary().id;

        // This path may consume credits — gated above.
        let result = backend
            .turn_start(TurnStartParams::text(thread_id, "ping from mitsuro test"))
            .await;
        // Soft-assert: either works or returns a structured RPC error (rate limit / etc.)
        match result {
            Ok(resp) => {
                assert!(resp.turn_id().is_some());
            }
            Err(e) => {
                eprintln!("live turn/start error (acceptable under credits/auth): {e}");
            }
        }
        let _ = backend.disconnect().await;
    }

    /// Strict paid acceptance: a real Codex turn must stream text and complete.
    #[tokio::test]
    async fn real_app_server_streaming_turn() {
        if std::env::var_os("MITSURO_RUN_LIVE_ACCEPTANCE").is_none() {
            eprintln!(
                "skip: set MITSURO_RUN_LIVE_ACCEPTANCE=1 to require a completed live Codex turn"
            );
            return;
        }
        assert!(
            should_run_integration(),
            "MITSURO_RUN_LIVE_ACCEPTANCE requires an available Codex binary"
        );

        let backend = CodexAppServerBackend::with_defaults();
        backend.connect().await.expect("connect");
        assert!(
            backend.has_usable_auth().await,
            "live Codex acceptance requires usable authentication"
        );

        let started = backend
            .thread_start(ThreadStartParams {
                cwd: Some(
                    std::env::current_dir()
                        .expect("current directory")
                        .display()
                        .to_string(),
                ),
                ephemeral: Some(true),
                ..Default::default()
            })
            .await
            .expect("ephemeral thread/start");
        let thread_id = started.summary().id;
        let mut events = Vec::new();
        let outcome = crate::live_turn::run_live_turn_with_policy(
            &backend,
            thread_id,
            "Reply with exactly CODEX_DESKTOP_ACCEPTANCE_OK. Do not use tools.".to_owned(),
            |event| events.push(event),
            crate::live_turn::LiveApprovalPolicy::AutoReject,
            Duration::from_secs(120),
        )
        .await;
        backend.disconnect().await.expect("disconnect");

        let outcome = outcome.expect("completed Codex streaming turn");
        assert!(outcome.completed, "Codex turn did not emit turn/completed");
        assert!(
            events.iter().any(|event| matches!(
                event,
                TurnStreamEvent::AgentMessageDelta { delta, .. } if !delta.is_empty()
            )),
            "Codex turn emitted no assistant text delta"
        );
    }
}
