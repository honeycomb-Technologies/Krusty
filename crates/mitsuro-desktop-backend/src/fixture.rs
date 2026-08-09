//! Offline fixture backend: replays canned JSONL turn streams (no paid models).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::account::{
    fixture_demo_account_response, fixture_demo_rate_limits, fixture_demo_usage,
    fixture_login_chatgpt_response, fixture_login_device_code_response,
    fixture_signed_out_account_response, CancelLoginAccountParams, CancelLoginAccountResponse,
    CancelLoginAccountStatus, GetAccountParams, GetAccountRateLimitsResponse, GetAccountResponse,
    GetAccountTokenUsageResponse, LoginAccountParams, LoginAccountResponse, LogoutAccountResponse,
    FIXTURE_LOGIN_ID,
};
use crate::backend::AgentBackend;
use crate::environment::{
    fixture_added_environment_summary, fixture_demo_collaboration_modes, fixture_demo_environments,
    fixture_environment_info, fixture_environment_status, CollaborationModeListParams,
    CollaborationModeListResponse, EnvironmentAddParams, EnvironmentAddResponse,
    EnvironmentInfoParams, EnvironmentInfoResponse, EnvironmentStatusParams,
    EnvironmentStatusResponse, EnvironmentSummary,
};
use crate::extensions::{
    fixture_demo_mcp_servers, fixture_demo_plugin_read, fixture_demo_plugins,
    fixture_demo_plugins_installed, fixture_mcp_tool_call, ListMcpServerStatusParams,
    ListMcpServerStatusResponse, McpServerToolCallParams, McpServerToolCallResponse,
    PluginInstalledParams, PluginInstalledResponse, PluginListParams, PluginListResponse,
    PluginReadParams, PluginReadResponse,
};
use crate::fs::{
    fixture_fuzzy_search, fixture_get_metadata, fixture_project_tree, fixture_read_directory,
    fixture_read_file, FixtureFsNode, FsGetMetadataParams, FsGetMetadataResponse,
    FsReadDirectoryParams, FsReadDirectoryResponse, FsReadFileParams, FsReadFileResponse,
    FuzzyFileSearchParams, FuzzyFileSearchResponse, FuzzyFileSearchSessionStartParams,
    FuzzyFileSearchSessionStartResponse, FuzzyFileSearchSessionStopParams,
    FuzzyFileSearchSessionStopResponse, FuzzyFileSearchSessionUpdateParams,
    FuzzyFileSearchSessionUpdateResponse, FIXTURE_PROJECT_ROOT,
};
use crate::methods::is_known_client_method;
use crate::process::{
    decode_base64, encode_base64, ProcessKillParams, ProcessKillResponse, ProcessOutputStream,
    ProcessResizePtyParams, ProcessResizePtyResponse, ProcessSpawnParams, ProcessSpawnResponse,
    ProcessTerminalSize, ProcessWriteStdinParams, ProcessWriteStdinResponse,
};
use crate::protocol::{
    fixture_demo_config, fixture_demo_models, fixture_demo_skills, parse_fixture_jsonl,
    user_input_text_value, ConfigReadParams, ConfigReadResponse, InitializeResponse,
    ModelListParams, ModelListResponse, SkillsListParams, SkillsListResponse, ThreadArchiveParams,
    ThreadArchiveResponse, ThreadDeleteParams, ThreadDeleteResponse, ThreadForkParams,
    ThreadForkResponse, ThreadGoal, ThreadGoalClearParams, ThreadGoalClearResponse,
    ThreadGoalGetParams, ThreadGoalGetResponse, ThreadGoalSetParams, ThreadGoalSetResponse,
    ThreadListParams, ThreadListResponse, ThreadReadParams, ThreadReadResponse, ThreadResumeParams,
    ThreadResumeResponse, ThreadSearchParams, ThreadSearchResponse, ThreadSearchResult,
    ThreadSetNameParams, ThreadSetNameResponse, ThreadStartParams, ThreadStartResponse,
    ThreadSummary, ThreadUnarchiveParams, ThreadUnarchiveResponse, TurnInterruptParams,
    TurnInterruptResponse, TurnStartParams, TurnStartResponse,
};
use crate::types::{AgentError, ConnectionStatus, Result, TurnStreamEvent};

/// Active fuzzy search session (roots + last query results for UI drain).
#[derive(Debug, Clone)]
struct FixtureFuzzySession {
    roots: Vec<String>,
    last_query: String,
    last_results: FuzzyFileSearchResponse,
}

/// In-memory process session for the fixture `process/*` mock.
#[derive(Debug, Clone)]
struct FixtureProcess {
    #[allow(dead_code)]
    handle: String,
    #[allow(dead_code)]
    command: Vec<String>,
    stdin: Vec<u8>,
    running: bool,
    stream_stdout: bool,
    #[allow(dead_code)]
    size: Option<ProcessTerminalSize>,
    #[allow(dead_code)]
    exit_code: Option<i32>,
}

/// Embedded sample turn stream (also on disk under `fixtures/sample-turn.jsonl`).
pub const SAMPLE_TURN_JSONL: &str = include_str!("../fixtures/sample-turn.jsonl");

fn is_fixture_typed_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "thread/list"
            | "thread/start"
            | "thread/read"
            | "thread/search"
            | "thread/name/set"
            | "thread/archive"
            | "thread/unarchive"
            | "thread/delete"
            | "thread/fork"
            | "thread/resume"
            | "thread/goal/get"
            | "thread/goal/set"
            | "thread/goal/clear"
            | "model/list"
            | "config/read"
            | "skills/list"
            | "turn/start"
            | "turn/interrupt"
            | "process/spawn"
            | "process/writeStdin"
            | "process/resizePty"
            | "process/kill"
            | "account/read"
            | "account/login/start"
            | "account/login/cancel"
            | "account/logout"
            | "account/usage/read"
            | "account/rateLimits/read"
            | "fs/readDirectory"
            | "fs/readFile"
            | "fs/getMetadata"
            | "fuzzyFileSearch"
            | "fuzzyFileSearch/sessionStart"
            | "fuzzyFileSearch/sessionUpdate"
            | "fuzzyFileSearch/sessionStop"
            | "mcpServerStatus/list"
            | "mcpServer/tool/call"
            | "plugin/list"
            | "plugin/read"
            | "plugin/installed"
            | "environment/info"
            | "environment/status"
            | "environment/add"
            | "collaborationMode/list"
    )
}

/// Default path relative to the crate root for external tooling / overrides.
pub fn default_sample_turn_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sample-turn.jsonl")
}

/// Parse the embedded sample fixture into typed events.
pub fn load_sample_turn_events() -> Result<Vec<TurnStreamEvent>> {
    parse_fixture_jsonl(SAMPLE_TURN_JSONL).map_err(crate::types::AgentError::Protocol)
}

/// Load events from an arbitrary JSONL path.
pub fn load_turn_events_from_path(path: &Path) -> Result<Vec<TurnStreamEvent>> {
    let content = std::fs::read_to_string(path)?;
    parse_fixture_jsonl(&content).map_err(crate::types::AgentError::Protocol)
}

/// Replay events on a channel with optional inter-event delay (for UI streaming).
pub async fn replay_events(
    events: Vec<TurnStreamEvent>,
    delay: Duration,
) -> mpsc::UnboundedReceiver<TurnStreamEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for ev in events {
            if tx.send(ev).is_err() {
                break;
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
        }
    });
    rx
}

/// Replay the embedded sample turn with a small delay between deltas.
pub async fn replay_sample_turn(
    delay: Duration,
) -> Result<mpsc::UnboundedReceiver<TurnStreamEvent>> {
    let events = load_sample_turn_events()?;
    Ok(replay_events(events, delay).await)
}

/// Offline agent backend that never talks to models or app-server.
///
/// - `connect` → Ready/Fixture with canned initialize payload
/// - `thread_list` / `thread_start` / `thread_read` → local memory
/// - `turn_start` → returns a synthetic Turn and stores last params (stream via
///   [`FixtureBackend::stream_turn`] separately so UI can control timing)
/// - [`AgentBackend::call_raw`] → every known client method returns success JSON
#[derive(Debug)]
pub struct FixtureBackend {
    status: RwLock<ConnectionStatus>,
    init: RwLock<Option<InitializeResponse>>,
    threads: RwLock<Vec<Value>>,
    next_thread: AtomicU64,
    next_turn: AtomicU64,
    /// Optional override path for turn JSONL; default is embedded sample.
    fixture_path: Option<PathBuf>,
    stream_delay: Duration,
    /// Active / recently exited processes keyed by client `processHandle`.
    processes: RwLock<HashMap<String, FixtureProcess>>,
    /// Queued process notifications produced by spawn/write/kill (drain via [`Self::take_process_events`]).
    process_events: RwLock<Vec<TurnStreamEvent>>,
    /// Methods invoked via [`AgentBackend::call_raw`] (for coverage tests).
    fixture_calls: Mutex<Vec<String>>,
    /// Virtual filesystem tree (rooted at `/` with child `fixture-project`).
    fs_tree: FixtureFsNode,
    /// Fuzzy search sessions keyed by `sessionId`.
    fuzzy_sessions: RwLock<HashMap<String, FixtureFuzzySession>>,
    /// Last session-update results (UI convenience; protocol would use notifications).
    last_fuzzy_session_results: RwLock<Option<(String, FuzzyFileSearchResponse)>>,
    /// Environments registered via `environment/add` (beyond the static demo catalog).
    added_environments: RwLock<Vec<EnvironmentSummary>>,
    /// Thread-attached goals keyed by `threadId` (`thread/goal/*`).
    goals: RwLock<HashMap<String, ThreadGoal>>,
    /// Fixture account signed-in flag (default true → demo Pro profile).
    account_signed_in: RwLock<bool>,
    /// Pending login id from `account/login/start` (device-code / OAuth stub).
    pending_login_id: RwLock<Option<String>>,
}

impl Default for FixtureBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FixtureBackend {
    pub fn new() -> Self {
        Self {
            status: RwLock::new(ConnectionStatus::Disconnected),
            init: RwLock::new(None),
            threads: RwLock::new(vec![fixture_thread_value(
                "fixture-thread",
                "Fixture sample turn",
                "Offline stream · sample-turn.jsonl",
            )]),
            next_thread: AtomicU64::new(2),
            next_turn: AtomicU64::new(1),
            fixture_path: None,
            stream_delay: Duration::from_millis(40),
            processes: RwLock::new(HashMap::new()),
            process_events: RwLock::new(Vec::new()),
            fixture_calls: Mutex::new(Vec::new()),
            fs_tree: fixture_project_tree(),
            fuzzy_sessions: RwLock::new(HashMap::new()),
            last_fuzzy_session_results: RwLock::new(None),
            added_environments: RwLock::new(Vec::new()),
            goals: RwLock::new(HashMap::new()),
            account_signed_in: RwLock::new(true),
            pending_login_id: RwLock::new(None),
        }
    }

    /// Snapshot of stored thread goals (tests / Work UI).
    pub fn fixture_goals(&self) -> HashMap<String, ThreadGoal> {
        self.goals.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn extras_environments(&self) -> Vec<EnvironmentSummary> {
        self.added_environments
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    fn is_account_signed_in(&self) -> bool {
        self.account_signed_in.read().map(|g| *g).unwrap_or(true)
    }

    fn set_account_signed_in(&self, signed_in: bool) {
        if let Ok(mut g) = self.account_signed_in.write() {
            *g = signed_in;
        }
    }

    fn set_pending_login(&self, login_id: Option<String>) {
        if let Ok(mut g) = self.pending_login_id.write() {
            *g = login_id;
        }
    }

    fn pending_login(&self) -> Option<String> {
        self.pending_login_id.read().ok().and_then(|g| g.clone())
    }

    /// Absolute root of the virtual fixture project (`/fixture-project`).
    pub fn fixture_project_root() -> &'static str {
        FIXTURE_PROJECT_ROOT
    }

    /// Take last fuzzy session update results (sessionId, response), if any.
    pub fn take_fuzzy_session_results(&self) -> Option<(String, FuzzyFileSearchResponse)> {
        self.last_fuzzy_session_results
            .write()
            .ok()
            .and_then(|mut g| g.take())
    }

    /// Snapshot of methods recorded by [`AgentBackend::call_raw`].
    pub fn fixture_calls(&self) -> Vec<String> {
        self.fixture_calls
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Clear the call-raw audit log.
    pub fn clear_fixture_calls(&self) {
        if let Ok(mut g) = self.fixture_calls.lock() {
            g.clear();
        }
    }

    fn record_fixture_call(&self, method: &str) {
        if let Ok(mut g) = self.fixture_calls.lock() {
            g.push(method.to_string());
        }
    }

    /// Route `call_raw` for methods that already have typed offline implementations.
    ///
    /// Returns `None` when the method should use the generic fixture success payload
    /// (missing/invalid params for strict typed paths).
    async fn call_raw_typed(&self, method: &str, params: Value) -> Result<Option<Value>> {
        match method {
            "initialize" => {
                let init = if let Some(cached) = self.init.read().ok().and_then(|g| g.clone()) {
                    cached
                } else {
                    self.connect().await?
                };
                Ok(Some(serde_json::to_value(init)?))
            }
            "thread/list" => {
                let p: ThreadListParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.thread_list(p).await?)?))
            }
            "thread/start" => {
                let p: ThreadStartParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.thread_start(p).await?)?))
            }
            "thread/read" => {
                let Ok(p) = serde_json::from_value::<ThreadReadParams>(params) else {
                    return Ok(None);
                };
                match self.thread_read(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/search" => {
                let p: ThreadSearchParams =
                    serde_json::from_value(params).unwrap_or_else(|_| ThreadSearchParams::new(""));
                Ok(Some(serde_json::to_value(self.thread_search(p).await?)?))
            }
            "thread/name/set" => {
                let Ok(p) = serde_json::from_value::<ThreadSetNameParams>(params) else {
                    return Ok(None);
                };
                match self.thread_name_set(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/archive" => {
                let Ok(p) = serde_json::from_value::<ThreadArchiveParams>(params) else {
                    return Ok(None);
                };
                match self.thread_archive(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/unarchive" => {
                let Ok(p) = serde_json::from_value::<ThreadUnarchiveParams>(params) else {
                    return Ok(None);
                };
                match self.thread_unarchive(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/delete" => {
                let Ok(p) = serde_json::from_value::<ThreadDeleteParams>(params) else {
                    return Ok(None);
                };
                match self.thread_delete(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/fork" => {
                let Ok(p) = serde_json::from_value::<ThreadForkParams>(params) else {
                    return Ok(None);
                };
                match self.thread_fork(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/resume" => {
                let Ok(p) = serde_json::from_value::<ThreadResumeParams>(params) else {
                    return Ok(None);
                };
                match self.thread_resume(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/goal/get" => {
                let Ok(p) = serde_json::from_value::<ThreadGoalGetParams>(params) else {
                    return Ok(None);
                };
                match self.thread_goal_get(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/goal/set" => {
                let Ok(p) = serde_json::from_value::<ThreadGoalSetParams>(params) else {
                    return Ok(None);
                };
                match self.thread_goal_set(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "thread/goal/clear" => {
                let Ok(p) = serde_json::from_value::<ThreadGoalClearParams>(params) else {
                    return Ok(None);
                };
                match self.thread_goal_clear(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "model/list" => {
                let p: ModelListParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.model_list(p).await?)?))
            }
            "config/read" => {
                let p: ConfigReadParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.config_read(p).await?)?))
            }
            "skills/list" => {
                let p: SkillsListParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.skills_list(p).await?)?))
            }
            "turn/start" => {
                let Ok(p) = serde_json::from_value::<TurnStartParams>(params) else {
                    return Ok(None);
                };
                match self.turn_start(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "turn/interrupt" => {
                let p: TurnInterruptParams =
                    serde_json::from_value(params).unwrap_or_else(|_| TurnInterruptParams {
                        thread_id: "fixture-thread".into(),
                        turn_id: "turn-fixture-0".into(),
                    });
                Ok(Some(serde_json::to_value(self.turn_interrupt(p).await?)?))
            }
            "process/spawn" => {
                let Ok(p) = serde_json::from_value::<ProcessSpawnParams>(params) else {
                    return Ok(None);
                };
                if p.command.is_empty() || p.process_handle.trim().is_empty() {
                    return Ok(None);
                }
                match self.process_spawn(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "process/writeStdin" => {
                let Ok(p) = serde_json::from_value::<ProcessWriteStdinParams>(params) else {
                    return Ok(None);
                };
                match self.process_write_stdin(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "process/resizePty" => {
                let Ok(p) = serde_json::from_value::<ProcessResizePtyParams>(params) else {
                    return Ok(None);
                };
                match self.process_resize_pty(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "process/kill" => {
                let Ok(p) = serde_json::from_value::<ProcessKillParams>(params) else {
                    return Ok(None);
                };
                match self.process_kill(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "account/read" => {
                let p: GetAccountParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.account_read(p).await?)?))
            }
            "account/login/start" => {
                let p: LoginAccountParams =
                    serde_json::from_value(params).unwrap_or(LoginAccountParams::device_code());
                Ok(Some(serde_json::to_value(
                    self.account_login_start(p).await?,
                )?))
            }
            "account/login/cancel" => {
                let Ok(p) = serde_json::from_value::<CancelLoginAccountParams>(params) else {
                    return Ok(None);
                };
                Ok(Some(serde_json::to_value(
                    self.account_login_cancel(p).await?,
                )?))
            }
            "account/logout" => Ok(Some(serde_json::to_value(self.account_logout().await?)?)),
            "account/usage/read" => Ok(Some(serde_json::to_value(
                self.account_usage_read().await?,
            )?)),
            "account/rateLimits/read" => Ok(Some(serde_json::to_value(
                self.account_rate_limits_read().await?,
            )?)),
            "fs/readDirectory" => {
                let Ok(p) = serde_json::from_value::<FsReadDirectoryParams>(params) else {
                    return Ok(None);
                };
                match self.fs_read_directory(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "fs/readFile" => {
                let Ok(p) = serde_json::from_value::<FsReadFileParams>(params) else {
                    return Ok(None);
                };
                match self.fs_read_file(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "fs/getMetadata" => {
                let Ok(p) = serde_json::from_value::<FsGetMetadataParams>(params) else {
                    return Ok(None);
                };
                match self.fs_get_metadata(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "fuzzyFileSearch" => {
                let Ok(p) = serde_json::from_value::<FuzzyFileSearchParams>(params) else {
                    return Ok(None);
                };
                match self.fuzzy_file_search(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "fuzzyFileSearch/sessionStart" => {
                let Ok(p) = serde_json::from_value::<FuzzyFileSearchSessionStartParams>(params)
                else {
                    return Ok(None);
                };
                match self.fuzzy_file_search_session_start(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "fuzzyFileSearch/sessionUpdate" => {
                let Ok(p) = serde_json::from_value::<FuzzyFileSearchSessionUpdateParams>(params)
                else {
                    return Ok(None);
                };
                match self.fuzzy_file_search_session_update(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "fuzzyFileSearch/sessionStop" => {
                let Ok(p) = serde_json::from_value::<FuzzyFileSearchSessionStopParams>(params)
                else {
                    return Ok(None);
                };
                match self.fuzzy_file_search_session_stop(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "mcpServerStatus/list" => {
                let p: ListMcpServerStatusParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(
                    self.mcp_server_status_list(p).await?,
                )?))
            }
            "mcpServer/tool/call" => {
                let Ok(p) = serde_json::from_value::<McpServerToolCallParams>(params) else {
                    return Ok(None);
                };
                Ok(Some(serde_json::to_value(
                    self.mcp_server_tool_call(p).await?,
                )?))
            }
            "plugin/list" => {
                let p: PluginListParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.plugin_list(p).await?)?))
            }
            "plugin/read" => {
                let Ok(p) = serde_json::from_value::<PluginReadParams>(params) else {
                    return Ok(None);
                };
                match self.plugin_read(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "plugin/installed" => {
                let p: PluginInstalledParams = serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(self.plugin_installed(p).await?)?))
            }
            "environment/info" => {
                let Ok(p) = serde_json::from_value::<EnvironmentInfoParams>(params) else {
                    return Ok(None);
                };
                match self.environment_info(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "environment/status" => {
                let Ok(p) = serde_json::from_value::<EnvironmentStatusParams>(params) else {
                    return Ok(None);
                };
                match self.environment_status(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "environment/add" => {
                let Ok(p) = serde_json::from_value::<EnvironmentAddParams>(params) else {
                    return Ok(None);
                };
                match self.environment_add(p).await {
                    Ok(r) => Ok(Some(serde_json::to_value(r)?)),
                    Err(_) => Ok(None),
                }
            }
            "collaborationMode/list" => {
                let p: CollaborationModeListParams =
                    serde_json::from_value(params).unwrap_or_default();
                Ok(Some(serde_json::to_value(
                    self.collaboration_mode_list(p).await?,
                )?))
            }
            _ => Ok(None),
        }
    }

    pub fn with_fixture_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.fixture_path = Some(path.into());
        self
    }

    pub fn with_stream_delay(mut self, delay: Duration) -> Self {
        self.stream_delay = delay;
        self
    }

    pub fn stream_delay(&self) -> Duration {
        self.stream_delay
    }

    /// Load the canned (or path-overridden) turn events.
    pub fn load_events(&self) -> Result<Vec<TurnStreamEvent>> {
        if let Some(path) = &self.fixture_path {
            return load_turn_events_from_path(path);
        }
        load_sample_turn_events()
    }

    /// Spawn a streaming replay of the sample turn; returns the receiver.
    pub async fn stream_turn(&self) -> Result<mpsc::UnboundedReceiver<TurnStreamEvent>> {
        let events = self.load_events()?;
        Ok(replay_events(events, self.stream_delay).await)
    }

    fn set_status(&self, status: ConnectionStatus) {
        if let Ok(mut g) = self.status.write() {
            *g = status;
        }
    }

    /// Drain process notifications produced since the last call (`process/outputDelta`, `process/exited`).
    pub fn take_process_events(&self) -> Vec<TurnStreamEvent> {
        self.process_events
            .write()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }

    /// Snapshot whether a handle is currently running.
    pub fn process_is_running(&self, handle: &str) -> bool {
        self.processes
            .read()
            .ok()
            .and_then(|g| g.get(handle).map(|p| p.running))
            .unwrap_or(false)
    }

    /// Stdin bytes accumulated via `process/writeStdin` (fixture only).
    pub fn process_stdin(&self, handle: &str) -> Option<Vec<u8>> {
        self.processes
            .read()
            .ok()
            .and_then(|g| g.get(handle).map(|p| p.stdin.clone()))
    }

    fn push_process_event(&self, ev: TurnStreamEvent) {
        if let Ok(mut g) = self.process_events.write() {
            g.push(ev);
        }
    }

    fn push_output_delta(&self, handle: &str, stream: ProcessOutputStream, text: &str) {
        let delta_base64 = encode_base64(text.as_bytes());
        self.push_process_event(TurnStreamEvent::ProcessOutputDelta {
            process_handle: handle.to_string(),
            stream,
            delta: text.to_string(),
            delta_base64,
            cap_reached: false,
        });
    }

    fn push_exited(&self, handle: &str, exit_code: i32) {
        self.push_process_event(TurnStreamEvent::ProcessExited {
            process_handle: handle.to_string(),
            exit_code,
            stdout: String::new(),
            stdout_cap_reached: false,
            stderr: String::new(),
            stderr_cap_reached: false,
        });
    }

    fn require_connected(&self) -> Result<()> {
        if !self.status().is_usable() {
            Err(AgentError::NotConnected)
        } else {
            Ok(())
        }
    }
}

fn fixture_thread_value(id: &str, name: &str, preview: &str) -> Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "preview": preview,
        "cwd": "/tmp/mitsuro-fixture",
        "createdAt": 1_722_700_000,
        "updatedAt": 1_722_701_200,
        "modelProvider": "fixture",
        "ephemeral": true,
        "isPinned": false,
        "archived": false,
        "cliVersion": "fixture",
        "sessionId": "fixture-session",
        "source": "appServer",
        "status": "idle",
        "turns": [],
    })
}

fn thread_is_archived(thread: &Value) -> bool {
    thread
        .get("archived")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn find_thread_mut<'a>(threads: &'a mut [Value], thread_id: &str) -> Option<&'a mut Value> {
    threads
        .iter_mut()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(thread_id))
}

#[async_trait]
impl AgentBackend for FixtureBackend {
    fn name(&self) -> &'static str {
        "fixture"
    }

    fn status(&self) -> ConnectionStatus {
        self.status
            .read()
            .map(|s| s.clone())
            .unwrap_or(ConnectionStatus::Disconnected)
    }

    fn supports_method(&self, method: &str) -> bool {
        is_fixture_typed_method(method)
    }

    async fn call_raw(&self, method: &str, params: Value) -> Result<Value> {
        self.record_fixture_call(method);
        // Allow call_raw before connect for initialize; all others need fixture status.
        if method != "initialize" {
            self.require_connected()?;
        } else if !self.status().is_usable() {
            // initialize may connect implicitly via call_raw_typed
        }

        if let Some(value) = self.call_raw_typed(method, params).await? {
            return Ok(value);
        }

        let scope = if is_known_client_method(method) {
            "known but not implemented by the fixture"
        } else {
            "unknown method"
        };
        Err(AgentError::NotImplemented(format!(
            "FixtureBackend::call_raw({method}) — {scope}"
        )))
    }

    async fn connect(&self) -> Result<InitializeResponse> {
        let init = InitializeResponse {
            codex_home: "/tmp/mitsuro-fixture-home".into(),
            platform_family: "unix".into(),
            platform_os: std::env::consts::OS.into(),
            user_agent: format!("mitsuro-fixture/{}", env!("CARGO_PKG_VERSION")),
        };
        if let Ok(mut g) = self.init.write() {
            *g = Some(init.clone());
        }
        self.set_status(ConnectionStatus::Fixture);
        Ok(init)
    }

    async fn thread_list(&self, params: ThreadListParams) -> Result<ThreadListResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let want_archived = params.archived.unwrap_or(false);
        let data = self
            .threads
            .read()
            .map(|t| t.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|t| thread_is_archived(t) == want_archived)
            .collect();
        Ok(ThreadListResponse {
            data,
            next_cursor: None,
            backwards_cursor: None,
        })
    }

    async fn thread_start(&self, params: ThreadStartParams) -> Result<ThreadStartResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let n = self.next_thread.fetch_add(1, Ordering::Relaxed);
        let id = format!("fixture-thread-{n}");
        let cwd = params.cwd.unwrap_or_else(|| "/tmp/mitsuro-fixture".into());
        let thread = fixture_thread_value(&id, "New fixture thread", "Empty · fixture backend");
        let mut thread = thread;
        if let Some(obj) = thread.as_object_mut() {
            obj.insert("cwd".into(), Value::String(cwd.clone()));
            obj.insert(
                "ephemeral".into(),
                Value::Bool(params.ephemeral.unwrap_or(true)),
            );
        }
        if let Ok(mut g) = self.threads.write() {
            g.insert(0, thread.clone());
        }
        Ok(ThreadStartResponse {
            thread,
            model: params.model,
            model_provider: Some("fixture".into()),
            cwd: Some(cwd),
        })
    }

    async fn thread_read(&self, params: ThreadReadParams) -> Result<ThreadReadResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let threads = self.threads.read().map(|t| t.clone()).unwrap_or_default();
        let thread = threads
            .into_iter()
            .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(params.thread_id.as_str()))
            .ok_or_else(|| {
                crate::types::AgentError::Protocol(format!(
                    "fixture thread not found: {}",
                    params.thread_id
                ))
            })?;
        Ok(ThreadReadResponse { thread })
    }

    async fn model_list(&self, _params: ModelListParams) -> Result<ModelListResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        Ok(ModelListResponse {
            data: fixture_demo_models(),
            next_cursor: None,
        })
    }

    async fn config_read(&self, _params: ConfigReadParams) -> Result<ConfigReadResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        Ok(fixture_demo_config())
    }

    async fn thread_search(&self, params: ThreadSearchParams) -> Result<ThreadSearchResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let term = params.search_term.to_lowercase();
        let threads = self.threads.read().map(|t| t.clone()).unwrap_or_default();
        let mut data = Vec::new();
        for thread in threads {
            let summary = ThreadSummary::from_value(&thread);
            let hay = format!(
                "{} {} {}",
                summary.display_title(),
                summary.preview.as_deref().unwrap_or(""),
                summary.cwd.as_deref().unwrap_or("")
            )
            .to_lowercase();
            if term.is_empty() || hay.contains(&term) {
                let snippet = summary
                    .preview
                    .clone()
                    .or(summary.name.clone())
                    .unwrap_or_else(|| summary.id.clone());
                data.push(ThreadSearchResult { snippet, thread });
            }
        }
        if let Some(limit) = params.limit {
            data.truncate(limit as usize);
        }
        Ok(ThreadSearchResponse {
            data,
            next_cursor: None,
            backwards_cursor: None,
        })
    }

    async fn thread_name_set(&self, params: ThreadSetNameParams) -> Result<ThreadSetNameResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let mut found = false;
        if let Ok(mut g) = self.threads.write() {
            if let Some(t) = find_thread_mut(&mut g, &params.thread_id) {
                if let Some(obj) = t.as_object_mut() {
                    obj.insert("name".into(), Value::String(params.name.clone()));
                }
                found = true;
            }
        }
        if !found {
            return Err(crate::types::AgentError::Protocol(format!(
                "fixture thread not found: {}",
                params.thread_id
            )));
        }
        Ok(ThreadSetNameResponse {})
    }

    async fn thread_archive(&self, params: ThreadArchiveParams) -> Result<ThreadArchiveResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let mut found = false;
        if let Ok(mut g) = self.threads.write() {
            if let Some(t) = find_thread_mut(&mut g, &params.thread_id) {
                if let Some(obj) = t.as_object_mut() {
                    obj.insert("archived".into(), Value::Bool(true));
                }
                found = true;
            }
        }
        if !found {
            return Err(crate::types::AgentError::Protocol(format!(
                "fixture thread not found: {}",
                params.thread_id
            )));
        }
        Ok(ThreadArchiveResponse {})
    }

    async fn thread_unarchive(
        &self,
        params: ThreadUnarchiveParams,
    ) -> Result<ThreadUnarchiveResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let mut thread = None;
        if let Ok(mut g) = self.threads.write() {
            if let Some(t) = find_thread_mut(&mut g, &params.thread_id) {
                if let Some(obj) = t.as_object_mut() {
                    obj.insert("archived".into(), Value::Bool(false));
                }
                thread = Some(t.clone());
            }
        }
        let thread = thread.ok_or_else(|| {
            crate::types::AgentError::Protocol(format!(
                "fixture thread not found: {}",
                params.thread_id
            ))
        })?;
        Ok(ThreadUnarchiveResponse { thread })
    }

    async fn thread_delete(&self, params: ThreadDeleteParams) -> Result<ThreadDeleteResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let mut removed = false;
        if let Ok(mut g) = self.threads.write() {
            let before = g.len();
            g.retain(|t| t.get("id").and_then(|v| v.as_str()) != Some(params.thread_id.as_str()));
            removed = g.len() < before;
        }
        if !removed {
            return Err(crate::types::AgentError::Protocol(format!(
                "fixture thread not found: {}",
                params.thread_id
            )));
        }
        Ok(ThreadDeleteResponse {})
    }

    async fn thread_fork(&self, params: ThreadForkParams) -> Result<ThreadForkResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let source = {
            let threads = self.threads.read().map(|t| t.clone()).unwrap_or_default();
            threads
                .into_iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(params.thread_id.as_str()))
                .ok_or_else(|| {
                    crate::types::AgentError::Protocol(format!(
                        "fixture thread not found: {}",
                        params.thread_id
                    ))
                })?
        };
        let n = self.next_thread.fetch_add(1, Ordering::Relaxed);
        let new_id = format!("fixture-thread-{n}");
        let mut forked = source;
        if let Some(obj) = forked.as_object_mut() {
            obj.insert("id".into(), Value::String(new_id));
            obj.insert("archived".into(), Value::Bool(false));
            let base_name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("Thread");
            obj.insert("name".into(), Value::String(format!("{base_name} (fork)")));
            if let Some(cwd) = &params.cwd {
                obj.insert("cwd".into(), Value::String(cwd.clone()));
            }
            if let Some(ephemeral) = params.ephemeral {
                obj.insert("ephemeral".into(), Value::Bool(ephemeral));
            }
            if params.exclude_turns == Some(true) {
                obj.insert("turns".into(), Value::Array(vec![]));
            }
            // Fork is a new thread; clear session identity so it is distinct.
            obj.insert(
                "sessionId".into(),
                Value::String(format!("fixture-session-fork-{n}")),
            );
        }
        if let Ok(mut g) = self.threads.write() {
            g.insert(0, forked.clone());
        }
        let cwd = forked
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(params.cwd);
        Ok(ThreadForkResponse {
            thread: forked,
            model: params.model,
            model_provider: params.model_provider.or_else(|| Some("fixture".into())),
            cwd,
        })
    }

    async fn thread_resume(&self, params: ThreadResumeParams) -> Result<ThreadResumeResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let mut thread = {
            let threads = self.threads.read().map(|t| t.clone()).unwrap_or_default();
            threads
                .into_iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(params.thread_id.as_str()))
                .ok_or_else(|| {
                    crate::types::AgentError::Protocol(format!(
                        "fixture thread not found: {}",
                        params.thread_id
                    ))
                })?
        };
        if params.exclude_turns == Some(true) {
            if let Some(obj) = thread.as_object_mut() {
                obj.insert("turns".into(), Value::Array(vec![]));
            }
        }
        if let Some(cwd) = &params.cwd {
            if let Some(obj) = thread.as_object_mut() {
                obj.insert("cwd".into(), Value::String(cwd.clone()));
            }
        }
        let cwd = thread
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(params.cwd);
        Ok(ThreadResumeResponse {
            thread,
            model: params.model,
            model_provider: params.model_provider.or_else(|| Some("fixture".into())),
            cwd,
        })
    }

    async fn thread_goal_get(&self, params: ThreadGoalGetParams) -> Result<ThreadGoalGetResponse> {
        self.require_connected()?;
        let goal = self
            .goals
            .read()
            .ok()
            .and_then(|g| g.get(&params.thread_id).cloned());
        Ok(ThreadGoalGetResponse { goal })
    }

    async fn thread_goal_set(&self, params: ThreadGoalSetParams) -> Result<ThreadGoalSetResponse> {
        self.require_connected()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let mut goals = self
            .goals
            .write()
            .map_err(|_| AgentError::Protocol("goals lock poisoned".into()))?;
        let goal = if let Some(existing) = goals.get_mut(&params.thread_id) {
            if let Some(obj) = params.objective {
                existing.objective = obj;
            }
            if let Some(status) = params.status {
                existing.status = status;
            }
            if params.token_budget.is_some() {
                existing.token_budget = params.token_budget;
            }
            existing.updated_at = now;
            existing.clone()
        } else {
            let mut g = ThreadGoal::new_active(
                &params.thread_id,
                params.objective.unwrap_or_else(|| "Untitled goal".into()),
            );
            if let Some(status) = params.status {
                g.status = status;
            }
            g.token_budget = params.token_budget;
            g.created_at = now;
            g.updated_at = now;
            goals.insert(params.thread_id.clone(), g.clone());
            g
        };
        Ok(ThreadGoalSetResponse { goal })
    }

    async fn thread_goal_clear(
        &self,
        params: ThreadGoalClearParams,
    ) -> Result<ThreadGoalClearResponse> {
        self.require_connected()?;
        let mut goals = self
            .goals
            .write()
            .map_err(|_| AgentError::Protocol("goals lock poisoned".into()))?;
        let cleared = goals.remove(&params.thread_id).is_some();
        Ok(ThreadGoalClearResponse { cleared })
    }

    async fn skills_list(&self, _params: SkillsListParams) -> Result<SkillsListResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        Ok(fixture_demo_skills())
    }

    async fn turn_start(&self, params: TurnStartParams) -> Result<TurnStartResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        let n = self.next_turn.fetch_add(1, Ordering::Relaxed);
        let turn_id = format!("turn-fixture-{n}");
        // Echo user text into a userMessage item so thread_read-style consumers work.
        let user_text = params
            .input
            .iter()
            .find_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    p.get("text").and_then(|t| t.as_str()).map(str::to_string)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        // Record selected model when provided (parity with live TurnStartParams.model).
        let model = params.model.clone().unwrap_or_else(|| "fixture".into());
        let turn = serde_json::json!({
            "id": turn_id,
            "model": model,
            "items": [{
                "type": "userMessage",
                "id": format!("user-{n}"),
                "clientId": null,
                "content": [user_input_text_value(user_text)],
            }],
            "itemsView": "full",
            "status": "inProgress",
            "error": null,
            "startedAt": 1_722_701_200i64,
            "completedAt": null,
            "durationMs": null,
        });
        // Preview update on matching thread
        if let Ok(mut g) = self.threads.write() {
            for t in g.iter_mut() {
                if t.get("id").and_then(|v| v.as_str()) == Some(params.thread_id.as_str()) {
                    if let Some(obj) = t.as_object_mut() {
                        let preview: String = params
                            .input
                            .iter()
                            .find_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .unwrap_or("fixture turn")
                            .chars()
                            .take(64)
                            .collect();
                        obj.insert("preview".into(), Value::String(preview));
                    }
                }
            }
        }
        Ok(TurnStartResponse { turn })
    }

    async fn turn_interrupt(&self, _params: TurnInterruptParams) -> Result<TurnInterruptResponse> {
        if !self.status().is_usable() {
            return Err(crate::types::AgentError::NotConnected);
        }
        // Fixture: no live stream to cancel at the backend layer; success no-op.
        // UI clears its local replay via a cancel flag after this returns.
        Ok(TurnInterruptResponse {})
    }

    async fn process_spawn(&self, params: ProcessSpawnParams) -> Result<ProcessSpawnResponse> {
        self.require_connected()?;
        if params.command.is_empty() {
            return Err(AgentError::Protocol(
                "process/spawn: command must be non-empty".into(),
            ));
        }
        if params.process_handle.trim().is_empty() {
            return Err(AgentError::Protocol(
                "process/spawn: processHandle required".into(),
            ));
        }
        let handle = params.process_handle;
        let stream_stdout =
            params.stream_stdout_stderr.unwrap_or(true) || params.tty.unwrap_or(false);
        {
            let mut map = self
                .processes
                .write()
                .map_err(|_| AgentError::Other("process lock poisoned".into()))?;
            if let Some(existing) = map.get(&handle) {
                if existing.running {
                    return Err(AgentError::Protocol(format!(
                        "process/spawn: handle already active: {handle}"
                    )));
                }
            }
            map.insert(
                handle.clone(),
                FixtureProcess {
                    handle: handle.clone(),
                    command: params.command.clone(),
                    stdin: Vec::new(),
                    running: true,
                    stream_stdout,
                    size: params.size,
                    exit_code: None,
                },
            );
        }

        if stream_stdout {
            let cmd_display = params.command.join(" ");
            self.push_output_delta(
                &handle,
                ProcessOutputStream::Stdout,
                &format!("[fixture] spawn {cmd_display}\n"),
            );
            if let Some(text) = fixture_auto_output(&params.command) {
                self.push_output_delta(&handle, ProcessOutputStream::Stdout, &text);
                if let Ok(mut map) = self.processes.write() {
                    if let Some(p) = map.get_mut(&handle) {
                        p.running = false;
                        p.exit_code = Some(0);
                    }
                }
                self.push_exited(&handle, 0);
            }
        }

        Ok(ProcessSpawnResponse {
            process_handle: Some(handle),
        })
    }

    async fn process_write_stdin(
        &self,
        params: ProcessWriteStdinParams,
    ) -> Result<ProcessWriteStdinResponse> {
        self.require_connected()?;
        let handle = params.process_handle.clone();
        let bytes = if let Some(b64) = params.delta_base64.as_deref() {
            decode_base64(b64).map_err(AgentError::Protocol)?
        } else {
            Vec::new()
        };
        let stream = {
            let mut map = self
                .processes
                .write()
                .map_err(|_| AgentError::Other("process lock poisoned".into()))?;
            let proc = map.get_mut(&handle).ok_or_else(|| {
                AgentError::Protocol(format!("process/writeStdin: unknown handle {handle}"))
            })?;
            if !proc.running {
                return Err(AgentError::Protocol(format!(
                    "process/writeStdin: process not running: {handle}"
                )));
            }
            proc.stdin.extend_from_slice(&bytes);
            proc.stream_stdout
        };
        if stream && !bytes.is_empty() {
            let text = String::from_utf8_lossy(&bytes);
            self.push_output_delta(&handle, ProcessOutputStream::Stdout, &text);
        }
        Ok(ProcessWriteStdinResponse {})
    }

    async fn process_resize_pty(
        &self,
        params: ProcessResizePtyParams,
    ) -> Result<ProcessResizePtyResponse> {
        self.require_connected()?;
        let handle = params.process_handle.clone();
        let mut map = self
            .processes
            .write()
            .map_err(|_| AgentError::Other("process lock poisoned".into()))?;
        let proc = map.get_mut(&handle).ok_or_else(|| {
            AgentError::Protocol(format!("process/resizePty: unknown handle {handle}"))
        })?;
        if !proc.running {
            return Err(AgentError::Protocol(format!(
                "process/resizePty: process not running: {handle}"
            )));
        }
        proc.size = Some(params.size);
        Ok(ProcessResizePtyResponse {})
    }

    async fn process_kill(&self, params: ProcessKillParams) -> Result<ProcessKillResponse> {
        self.require_connected()?;
        let handle = params.process_handle;
        let was_running = {
            let mut map = self
                .processes
                .write()
                .map_err(|_| AgentError::Other("process lock poisoned".into()))?;
            let proc = map.get_mut(&handle).ok_or_else(|| {
                AgentError::Protocol(format!("process/kill: unknown handle {handle}"))
            })?;
            let running = proc.running;
            proc.running = false;
            proc.exit_code = Some(137);
            running
        };
        if was_running {
            self.push_exited(&handle, 137);
        }
        Ok(ProcessKillResponse {})
    }

    async fn fs_read_directory(
        &self,
        params: FsReadDirectoryParams,
    ) -> Result<FsReadDirectoryResponse> {
        self.require_connected()?;
        fixture_read_directory(&self.fs_tree, &params.path).map_err(AgentError::Protocol)
    }

    async fn fs_read_file(&self, params: FsReadFileParams) -> Result<FsReadFileResponse> {
        self.require_connected()?;
        fixture_read_file(&self.fs_tree, &params.path).map_err(AgentError::Protocol)
    }

    async fn fs_get_metadata(&self, params: FsGetMetadataParams) -> Result<FsGetMetadataResponse> {
        self.require_connected()?;
        fixture_get_metadata(&self.fs_tree, &params.path).map_err(AgentError::Protocol)
    }

    async fn fuzzy_file_search(
        &self,
        params: FuzzyFileSearchParams,
    ) -> Result<FuzzyFileSearchResponse> {
        self.require_connected()?;
        let roots = if params.roots.is_empty() {
            vec![FIXTURE_PROJECT_ROOT.to_string()]
        } else {
            params.roots
        };
        Ok(fixture_fuzzy_search(&self.fs_tree, &params.query, &roots))
    }

    async fn fuzzy_file_search_session_start(
        &self,
        params: FuzzyFileSearchSessionStartParams,
    ) -> Result<FuzzyFileSearchSessionStartResponse> {
        self.require_connected()?;
        if params.session_id.trim().is_empty() {
            return Err(AgentError::Protocol(
                "fuzzyFileSearch/sessionStart: sessionId required".into(),
            ));
        }
        let roots = if params.roots.is_empty() {
            vec![FIXTURE_PROJECT_ROOT.to_string()]
        } else {
            params.roots
        };
        let mut map = self
            .fuzzy_sessions
            .write()
            .map_err(|_| AgentError::Other("fuzzy session lock poisoned".into()))?;
        map.insert(
            params.session_id,
            FixtureFuzzySession {
                roots,
                last_query: String::new(),
                last_results: FuzzyFileSearchResponse { files: vec![] },
            },
        );
        Ok(FuzzyFileSearchSessionStartResponse {})
    }

    async fn fuzzy_file_search_session_update(
        &self,
        params: FuzzyFileSearchSessionUpdateParams,
    ) -> Result<FuzzyFileSearchSessionUpdateResponse> {
        self.require_connected()?;
        let results = {
            let mut map = self
                .fuzzy_sessions
                .write()
                .map_err(|_| AgentError::Other("fuzzy session lock poisoned".into()))?;
            let session = map.get_mut(&params.session_id).ok_or_else(|| {
                AgentError::Protocol(format!(
                    "fuzzyFileSearch/sessionUpdate: unknown session {}",
                    params.session_id
                ))
            })?;
            let results = fixture_fuzzy_search(&self.fs_tree, &params.query, &session.roots);
            session.last_query = params.query.clone();
            session.last_results = results.clone();
            results
        };
        if let Ok(mut g) = self.last_fuzzy_session_results.write() {
            *g = Some((params.session_id, results));
        }
        Ok(FuzzyFileSearchSessionUpdateResponse {})
    }

    async fn fuzzy_file_search_session_stop(
        &self,
        params: FuzzyFileSearchSessionStopParams,
    ) -> Result<FuzzyFileSearchSessionStopResponse> {
        self.require_connected()?;
        let mut map = self
            .fuzzy_sessions
            .write()
            .map_err(|_| AgentError::Other("fuzzy session lock poisoned".into()))?;
        map.remove(&params.session_id);
        Ok(FuzzyFileSearchSessionStopResponse {})
    }

    async fn mcp_server_status_list(
        &self,
        _params: ListMcpServerStatusParams,
    ) -> Result<ListMcpServerStatusResponse> {
        self.require_connected()?;
        Ok(fixture_demo_mcp_servers())
    }

    async fn mcp_server_tool_call(
        &self,
        params: McpServerToolCallParams,
    ) -> Result<McpServerToolCallResponse> {
        self.require_connected()?;
        Ok(fixture_mcp_tool_call(&params))
    }

    async fn plugin_list(&self, _params: PluginListParams) -> Result<PluginListResponse> {
        self.require_connected()?;
        Ok(fixture_demo_plugins())
    }

    async fn plugin_read(&self, params: PluginReadParams) -> Result<PluginReadResponse> {
        self.require_connected()?;
        fixture_demo_plugin_read(&params.plugin_name).ok_or_else(|| {
            AgentError::Protocol(format!("fixture plugin not found: {}", params.plugin_name))
        })
    }

    async fn plugin_installed(
        &self,
        _params: PluginInstalledParams,
    ) -> Result<PluginInstalledResponse> {
        self.require_connected()?;
        Ok(fixture_demo_plugins_installed())
    }

    async fn environment_info(
        &self,
        params: EnvironmentInfoParams,
    ) -> Result<EnvironmentInfoResponse> {
        self.require_connected()?;
        let extras = self.extras_environments();
        fixture_environment_info(&params.environment_id, &extras).ok_or_else(|| {
            AgentError::Protocol(format!(
                "environment/info: unknown environment {}",
                params.environment_id
            ))
        })
    }

    async fn environment_status(
        &self,
        params: EnvironmentStatusParams,
    ) -> Result<EnvironmentStatusResponse> {
        self.require_connected()?;
        let extras = self.extras_environments();
        Ok(fixture_environment_status(&params.environment_id, &extras))
    }

    async fn environment_add(
        &self,
        params: EnvironmentAddParams,
    ) -> Result<EnvironmentAddResponse> {
        self.require_connected()?;
        if params.environment_id.trim().is_empty() || params.exec_server_url.trim().is_empty() {
            return Err(AgentError::Protocol(
                "environment/add requires environmentId and execServerUrl".into(),
            ));
        }
        // Built-in demo ids are already present — treat re-add as success no-op.
        if fixture_demo_environments()
            .iter()
            .any(|e| e.id == params.environment_id)
        {
            return Ok(EnvironmentAddResponse {});
        }
        let summary = fixture_added_environment_summary(&params);
        if let Ok(mut g) = self.added_environments.write() {
            if let Some(existing) = g.iter_mut().find(|e| e.id == params.environment_id) {
                *existing = summary;
            } else {
                g.push(summary);
            }
        }
        Ok(EnvironmentAddResponse {})
    }

    async fn collaboration_mode_list(
        &self,
        _params: CollaborationModeListParams,
    ) -> Result<CollaborationModeListResponse> {
        self.require_connected()?;
        Ok(fixture_demo_collaboration_modes())
    }

    fn environment_catalog(&self) -> Vec<EnvironmentSummary> {
        let mut list = fixture_demo_environments();
        list.extend(self.extras_environments());
        list
    }

    async fn account_read(&self, _params: GetAccountParams) -> Result<GetAccountResponse> {
        self.require_connected()?;
        if self.is_account_signed_in() {
            Ok(fixture_demo_account_response())
        } else {
            Ok(fixture_signed_out_account_response())
        }
    }

    async fn account_login_start(
        &self,
        params: LoginAccountParams,
    ) -> Result<LoginAccountResponse> {
        self.require_connected()?;
        let response = match params {
            LoginAccountParams::ApiKey { .. } => {
                // Offline: accept any API key as signed-in.
                self.set_account_signed_in(true);
                self.set_pending_login(None);
                LoginAccountResponse::ApiKey
            }
            LoginAccountParams::Chatgpt { .. } => {
                let r = fixture_login_chatgpt_response();
                self.set_pending_login(r.login_id().map(str::to_string));
                // Fixture auto-completes login (no network / browser).
                self.set_account_signed_in(true);
                r
            }
            LoginAccountParams::ChatgptDeviceCode => {
                let r = fixture_login_device_code_response();
                self.set_pending_login(r.login_id().map(str::to_string));
                // Fixture auto-completes device login (no network).
                self.set_account_signed_in(true);
                r
            }
        };
        Ok(response)
    }

    async fn account_login_cancel(
        &self,
        params: CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse> {
        self.require_connected()?;
        let pending = self.pending_login();
        let status = if pending.as_deref() == Some(params.login_id.as_str())
            || params.login_id == FIXTURE_LOGIN_ID
            || pending.is_some()
        {
            self.set_pending_login(None);
            CancelLoginAccountStatus::Canceled
        } else {
            CancelLoginAccountStatus::NotFound
        };
        Ok(CancelLoginAccountResponse { status })
    }

    async fn account_logout(&self) -> Result<LogoutAccountResponse> {
        self.require_connected()?;
        self.set_account_signed_in(false);
        self.set_pending_login(None);
        Ok(LogoutAccountResponse {})
    }

    async fn account_usage_read(&self) -> Result<GetAccountTokenUsageResponse> {
        self.require_connected()?;
        Ok(fixture_demo_usage())
    }

    async fn account_rate_limits_read(&self) -> Result<GetAccountRateLimitsResponse> {
        self.require_connected()?;
        Ok(fixture_demo_rate_limits())
    }

    async fn disconnect(&self) -> Result<()> {
        self.set_status(ConnectionStatus::Disconnected);
        Ok(())
    }
}

/// Synthetic stdout for offline echo-style commands (`None` → leave process running).
fn fixture_auto_output(command: &[String]) -> Option<String> {
    if command.is_empty() {
        return None;
    }
    if command[0] == "echo" {
        let rest = command[1..].join(" ");
        return Some(format!("{rest}\n"));
    }
    if command.len() >= 3 && command[0] == "bash" && (command[1] == "-lc" || command[1] == "-c") {
        let script = &command[2];
        if let Some(rest) = script.strip_prefix("echo ") {
            let text = rest.trim().trim_matches('"').trim_matches('\'');
            return Some(format!("{text}\n"));
        }
        if script.contains("mitsuro") || script.starts_with("printf ") {
            return Some(format!("[fixture] {script}\n"));
        }
        if script == "bash" || script.contains("sleep") || script == "cat" {
            return None;
        }
    }
    None
}

/// Helper for UI: summary list from fixture backend without full connect dance.
pub fn fixture_thread_summaries() -> Vec<ThreadSummary> {
    vec![ThreadSummary {
        id: "fixture-thread".into(),
        name: Some("Fixture sample turn".into()),
        preview: Some("Offline stream · sample-turn.jsonl".into()),
        cwd: Some("/tmp/mitsuro-fixture".into()),
        created_at: Some(1_722_700_000),
        updated_at: Some(1_722_701_200),
        model_provider: Some("fixture".into()),
        ephemeral: Some(true),
        is_pinned: Some(false),
        archived: Some(false),
        raw: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ItemKind, TurnStreamEvent};

    #[test]
    fn embedded_sample_parses_with_expected_kinds() {
        let events = load_sample_turn_events().expect("parse sample");
        assert!(
            events.len() >= 8,
            "expected a multi-step sample stream, got {}",
            events.len()
        );
        assert!(matches!(
            events.first(),
            Some(TurnStreamEvent::TurnStarted { .. })
        ));
        assert!(matches!(
            events.last(),
            Some(TurnStreamEvent::TurnCompleted {
                status: Some(s),
                ..
            }) if s == "completed"
        ));
        let deltas: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, TurnStreamEvent::AgentMessageDelta { .. }))
            .collect();
        assert!(
            deltas.len() >= 3,
            "expected agent message deltas, got {}",
            deltas.len()
        );
        let mut body = String::new();
        for e in &events {
            if let TurnStreamEvent::AgentMessageDelta { delta, .. } = e {
                body.push_str(delta);
            }
        }
        assert!(body.contains("fixture"), "assembled body: {body}");
        assert!(events.iter().any(|e| matches!(
            e,
            TurnStreamEvent::ItemStarted {
                kind: ItemKind::AgentMessage,
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            TurnStreamEvent::PlanDelta { .. } | TurnStreamEvent::ReasoningSummaryDelta { .. }
        )));
        // Mid-stream approval server request
        assert!(
            events.iter().any(|e| matches!(
                e,
                TurnStreamEvent::ApprovalRequested(p)
                    if p.summary.contains("ls -la")
            )),
            "sample fixture must inject an approval request mid-stream"
        );
        // P7: command output deltas + fileChange patch
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TurnStreamEvent::CommandExecutionOutputDelta { .. })),
            "sample fixture must include commandExecution/outputDelta"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                TurnStreamEvent::ItemStarted {
                    kind: ItemKind::CommandExecution,
                    ..
                }
            )),
            "sample fixture must start a commandExecution item"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                TurnStreamEvent::FileChangePatchUpdated { .. }
                    | TurnStreamEvent::ItemStarted {
                        kind: ItemKind::FileChange,
                        ..
                    }
            )),
            "sample fixture must include a fileChange item / patchUpdated"
        );
    }

    #[test]
    fn sample_file_on_disk_matches_embedded() {
        let path = default_sample_turn_path();
        assert!(path.is_file(), "missing {}", path.display());
        let from_disk = load_turn_events_from_path(&path).unwrap();
        let embedded = load_sample_turn_events().unwrap();
        assert_eq!(from_disk.len(), embedded.len());
    }

    #[tokio::test]
    async fn fixture_backend_connect_list_and_turn() {
        let backend = FixtureBackend::new().with_stream_delay(Duration::ZERO);
        let init = backend.connect().await.unwrap();
        assert!(init.user_agent.contains("fixture"));
        assert!(matches!(backend.status(), ConnectionStatus::Fixture));

        let list = backend
            .thread_list(ThreadListParams::default())
            .await
            .unwrap();
        assert!(!list.threads().is_empty());

        let started = backend
            .thread_start(ThreadStartParams {
                cwd: Some("/tmp".into()),
                ephemeral: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(!started.summary().id.is_empty());

        let turn = backend
            .turn_start(TurnStartParams::text_with_model(
                started.summary().id.clone(),
                "hello fixture",
                Some("gpt-5".into()),
            ))
            .await
            .unwrap();
        assert!(turn.turn_id().is_some());
        assert_eq!(
            turn.turn.get("model").and_then(|v| v.as_str()),
            Some("gpt-5"),
            "fixture turn_start should echo selected model"
        );

        let models = backend
            .model_list(ModelListParams::default())
            .await
            .unwrap();
        assert!(
            models.data.len() >= 3,
            "fixture must expose demo models, got {}",
            models.data.len()
        );
        assert!(
            models
                .data
                .iter()
                .any(|m| m.id == "gpt-5-demo" || m.model.contains("gpt-5")),
            "expected gpt-5 demo model"
        );
        assert!(models.default_model().is_some());

        // config/read, skills/list, thread/search, thread/name/set offline stubs
        let cfg = backend
            .config_read(ConfigReadParams {
                include_layers: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(cfg.model(), Some("gpt-5"));
        assert!(cfg.settings_snippet().contains("model: gpt-5"));

        let skills = backend
            .skills_list(SkillsListParams::default())
            .await
            .unwrap();
        assert!(skills.skill_count() >= 2, "demo skills expected");

        let search = backend
            .thread_search(ThreadSearchParams::new("fixture"))
            .await
            .unwrap();
        assert!(
            !search.data.is_empty(),
            "search for 'fixture' should hit sample thread"
        );

        let tid = started.summary().id.clone();
        backend
            .thread_name_set(ThreadSetNameParams::new(tid.clone(), "Renamed fixture"))
            .await
            .unwrap();
        let read = backend
            .thread_read(ThreadReadParams {
                thread_id: tid,
                include_turns: Some(false),
            })
            .await
            .unwrap();
        assert_eq!(read.summary().name.as_deref(), Some("Renamed fixture"));

        let mut rx = backend.stream_turn().await.unwrap();
        let mut count = 0;
        while rx.recv().await.is_some() {
            count += 1;
        }
        assert!(count >= 8, "streamed {count} events");
    }

    #[test]
    fn fixture_demo_models_include_gpt5() {
        let models = fixture_demo_models();
        assert!(models.iter().any(|m| m.id == "gpt-5-demo"));
        assert_eq!(
            models.iter().filter(|m| m.is_default).count(),
            1,
            "exactly one default demo model"
        );
    }

    #[tokio::test]
    async fn fixture_archive_fork_delete_interrupt() {
        let backend = FixtureBackend::new().with_stream_delay(Duration::ZERO);
        backend.connect().await.unwrap();

        // Start a dedicated thread for lifecycle ops.
        let started = backend
            .thread_start(ThreadStartParams {
                cwd: Some("/tmp/life".into()),
                ephemeral: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        let tid = started.summary().id.clone();

        // archive → disappears from default list, appears with archived=true
        backend
            .thread_archive(ThreadArchiveParams::new(tid.clone()))
            .await
            .unwrap();
        let active = backend
            .thread_list(ThreadListParams {
                archived: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            active.threads().iter().all(|t| t.id != tid),
            "archived thread must leave active list"
        );
        let archived = backend
            .thread_list(ThreadListParams {
                archived: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        let archived_sum = archived
            .threads()
            .into_iter()
            .find(|t| t.id == tid)
            .expect("in archived list");
        assert_eq!(archived_sum.archived, Some(true));

        // unarchive
        let un = backend
            .thread_unarchive(ThreadUnarchiveParams::new(tid.clone()))
            .await
            .unwrap();
        assert_eq!(un.summary().id, tid);
        assert_eq!(un.summary().archived, Some(false));

        // fork clones
        let forked = backend
            .thread_fork(ThreadForkParams::new(tid.clone()))
            .await
            .unwrap();
        let fork_id = forked.summary().id.clone();
        assert_ne!(fork_id, tid);
        assert!(
            forked
                .summary()
                .name
                .as_deref()
                .unwrap_or("")
                .contains("fork"),
            "fork name should indicate fork"
        );
        let list = backend
            .thread_list(ThreadListParams::default())
            .await
            .unwrap();
        assert!(list.threads().iter().any(|t| t.id == fork_id));
        assert!(list.threads().iter().any(|t| t.id == tid));

        // resume returns the same thread
        let resumed = backend
            .thread_resume(ThreadResumeParams::new(tid.clone()))
            .await
            .unwrap();
        assert_eq!(resumed.summary().id, tid);

        // turn + interrupt (no-op success)
        let turn = backend
            .turn_start(TurnStartParams::text(tid.clone(), "hi"))
            .await
            .unwrap();
        let turn_id = turn.turn_id().expect("turn id").to_string();
        backend
            .turn_interrupt(TurnInterruptParams::new(tid.clone(), turn_id))
            .await
            .unwrap();

        // delete removes
        backend
            .thread_delete(ThreadDeleteParams::new(tid.clone()))
            .await
            .unwrap();
        let after = backend
            .thread_list(ThreadListParams {
                archived: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(after.threads().iter().all(|t| t.id != tid));
        // forked still present
        assert!(after.threads().iter().any(|t| t.id == fork_id));

        // delete missing → error
        let err = backend
            .thread_delete(ThreadDeleteParams::new(tid))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "expected not found, got {err}"
        );
    }

    #[test]
    fn lifecycle_params_serialize_camel_case() {
        // Mirror protocol shapes used by fixture calls.
        let a = serde_json::to_value(ThreadArchiveParams::new("t1")).unwrap();
        assert_eq!(a["threadId"], "t1");
        let f = serde_json::to_value(ThreadForkParams::new("t2")).unwrap();
        assert_eq!(f["threadId"], "t2");
        assert!(f.get("lastTurnId").is_none());
        let i = serde_json::to_value(TurnInterruptParams::new("t3", "turn-1")).unwrap();
        assert_eq!(i["threadId"], "t3");
        assert_eq!(i["turnId"], "turn-1");
        let d = serde_json::to_value(ThreadDeleteParams::new("t4")).unwrap();
        assert_eq!(d["threadId"], "t4");
        let r = serde_json::to_value(ThreadResumeParams::new("t5")).unwrap();
        assert_eq!(r["threadId"], "t5");
        let u = serde_json::to_value(ThreadUnarchiveParams::new("t6")).unwrap();
        assert_eq!(u["threadId"], "t6");
    }

    #[tokio::test]
    async fn fixture_process_spawn_write_kill() {
        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let spawn = backend
            .process_spawn(ProcessSpawnParams::streaming(
                vec!["bash".into(), "-lc".into(), "cat".into()],
                "ph-term-1",
                "/tmp/mitsuro-fixture",
            ))
            .await
            .unwrap();
        assert_eq!(spawn.process_handle.as_deref(), Some("ph-term-1"));
        assert!(backend.process_is_running("ph-term-1"));

        let events = backend.take_process_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                TurnStreamEvent::ProcessOutputDelta {
                    process_handle,
                    ..
                } if process_handle == "ph-term-1"
            )),
            "spawn should emit outputDelta banner, got {events:?}"
        );

        backend
            .process_write_stdin(ProcessWriteStdinParams::text("ph-term-1", "hello stdin\n"))
            .await
            .unwrap();
        let stdin = backend.process_stdin("ph-term-1").unwrap();
        assert_eq!(String::from_utf8_lossy(&stdin), "hello stdin\n");
        let events = backend.take_process_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                TurnStreamEvent::ProcessOutputDelta { delta, .. } if delta.contains("hello stdin")
            )),
            "writeStdin should echo as outputDelta: {events:?}"
        );

        backend
            .process_resize_pty(ProcessResizePtyParams::new("ph-term-1", 40, 120))
            .await
            .unwrap();

        backend
            .process_kill(ProcessKillParams::new("ph-term-1"))
            .await
            .unwrap();
        assert!(!backend.process_is_running("ph-term-1"));
        let events = backend.take_process_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                TurnStreamEvent::ProcessExited {
                    process_handle,
                    exit_code: 137,
                    ..
                } if process_handle == "ph-term-1"
            )),
            "kill should emit process/exited: {events:?}"
        );

        let _ = backend
            .process_spawn(ProcessSpawnParams::bash_lc(
                "echo hello from mitsuro",
                "ph-echo",
                "/tmp",
            ))
            .await
            .unwrap();
        assert!(!backend.process_is_running("ph-echo"));
        let events = backend.take_process_events();
        assert!(events.iter().any(|e| matches!(
            e,
            TurnStreamEvent::ProcessOutputDelta { delta, .. } if delta.contains("hello from mitsuro")
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            TurnStreamEvent::ProcessExited {
                exit_code: 0,
                process_handle,
                ..
            } if process_handle == "ph-echo"
        )));
    }

    #[tokio::test]
    async fn fixture_fs_read_directory_file_metadata() {
        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let dir = backend
            .fs_read_directory(FsReadDirectoryParams::new(FIXTURE_PROJECT_ROOT))
            .await
            .unwrap();
        assert!(
            dir.entries
                .iter()
                .any(|e| e.file_name == "src" && e.is_directory),
            "expected src dir: {:?}",
            dir.entries
        );
        assert!(
            dir.entries
                .iter()
                .any(|e| e.file_name == "README.md" && e.is_file),
            "expected README.md: {:?}",
            dir.entries
        );

        let main_path = format!("{FIXTURE_PROJECT_ROOT}/src/main.rs");
        let file = backend
            .fs_read_file(FsReadFileParams::new(&main_path))
            .await
            .unwrap();
        assert!(file.text_lossy().contains("fixture-project"));

        let meta = backend
            .fs_get_metadata(FsGetMetadataParams::new(&main_path))
            .await
            .unwrap();
        assert!(meta.is_file);
        assert!(!meta.is_directory);

        let missing = backend
            .fs_read_file(FsReadFileParams::new("/nope/missing.txt"))
            .await;
        assert!(missing.is_err());
    }

    #[tokio::test]
    async fn fixture_fuzzy_file_search_filters_names() {
        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let resp = backend
            .fuzzy_file_search(FuzzyFileSearchParams::new(
                "main",
                vec![FIXTURE_PROJECT_ROOT.into()],
            ))
            .await
            .unwrap();
        assert!(
            resp.files.iter().any(|f| f.file_name == "main.rs"),
            "main should match main.rs: {:?}",
            resp.files
        );
        assert!(
            !resp.files.iter().any(|f| f.file_name == "guide.md"),
            "guide.md should not match 'main'"
        );

        // Session path
        backend
            .fuzzy_file_search_session_start(FuzzyFileSearchSessionStartParams::new(
                "sess-1",
                vec![FIXTURE_PROJECT_ROOT.into()],
            ))
            .await
            .unwrap();
        backend
            .fuzzy_file_search_session_update(FuzzyFileSearchSessionUpdateParams::new(
                "sess-1", "lib",
            ))
            .await
            .unwrap();
        let (sid, results) = backend
            .take_fuzzy_session_results()
            .expect("session results");
        assert_eq!(sid, "sess-1");
        assert!(
            results.files.iter().any(|f| f.file_name == "lib.rs"),
            "lib should match lib.rs: {:?}",
            results.files
        );
        backend
            .fuzzy_file_search_session_stop(FuzzyFileSearchSessionStopParams::new("sess-1"))
            .await
            .unwrap();
        let stop_update = backend
            .fuzzy_file_search_session_update(FuzzyFileSearchSessionUpdateParams::new(
                "sess-1", "x",
            ))
            .await;
        assert!(stop_update.is_err(), "stopped session should reject update");
    }

    /// Fixture support is explicit: implemented typed methods work and all
    /// other methods fail honestly instead of returning synthetic success.
    #[tokio::test]
    async fn fixture_call_raw_is_honest_about_coverage() {
        let backend = FixtureBackend::new().with_stream_delay(Duration::ZERO);
        backend.connect().await.unwrap();
        backend.clear_fixture_calls();

        assert!(backend.supports_method("thread/list"));
        let list = backend
            .call_raw("thread/list", serde_json::json!({}))
            .await
            .expect("typed fixture thread/list");
        assert!(list.get("data").and_then(Value::as_array).is_some());

        assert!(!backend.supports_method("app/list"));
        assert!(matches!(
            backend.call_raw("app/list", serde_json::json!({})).await,
            Err(AgentError::NotImplemented(_))
        ));
        assert!(matches!(
            backend
                .call_raw("not/a/real/method", serde_json::json!({}))
                .await,
            Err(AgentError::NotImplemented(_))
        ));

        let models = backend
            .call_raw("model/list", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            models
                .get("data")
                .and_then(|d| d.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "model/list via call_raw: {models}"
        );
        assert_eq!(
            backend.fixture_calls(),
            vec![
                "thread/list".to_owned(),
                "app/list".to_owned(),
                "not/a/real/method".to_owned(),
                "model/list".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn mitsuro_call_raw_not_implemented() {
        use crate::mitsuro::MitsuroServerBackend;
        use crate::types::AgentError;

        let backend = MitsuroServerBackend::new();
        assert!(backend.supports_method("thread/list"));
        let err = backend
            .call_raw("thread/list", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentError::NotImplemented(_)),
            "expected NotImplemented, got {err}"
        );
    }

    #[tokio::test]
    async fn fixture_mcp_and_plugin_lists() {
        use crate::extensions::{
            ListMcpServerStatusParams, McpServerToolCallParams, PluginInstalledParams,
            PluginListParams, PluginReadParams,
        };

        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let mcp = backend
            .mcp_server_status_list(ListMcpServerStatusParams::default())
            .await
            .unwrap();
        assert!(
            (2..=3).contains(&mcp.data.len()),
            "expected 2–3 MCP servers, got {}",
            mcp.data.len()
        );
        assert!(mcp.data.iter().any(|s| s.name == "fixture-filesystem"));
        assert!(mcp.data.iter().all(|s| !s.name.is_empty()));

        let plugins = backend
            .plugin_list(PluginListParams::default())
            .await
            .unwrap();
        let n = plugins.plugin_count();
        assert!(
            (40..=90).contains(&n),
            "expected 40–90 marketplace plugins, got {n}"
        );
        assert!(plugins.installed_count() >= 10);

        let installed = backend
            .plugin_installed(PluginInstalledParams::default())
            .await
            .unwrap();
        assert_eq!(installed.plugin_count(), plugins.installed_count());
        assert!(installed.all_plugins().iter().all(|p| p.installed));

        let detail = backend
            .plugin_read(PluginReadParams::new("fixture-review"))
            .await
            .unwrap();
        assert_eq!(detail.plugin.summary.name, "fixture-review");

        let call = backend
            .mcp_server_tool_call(McpServerToolCallParams::new(
                "fixture-thread",
                "fixture-filesystem",
                "read_file",
            ))
            .await
            .unwrap();
        assert_eq!(call.is_error, Some(false));
        assert!(!call.content.is_empty());

        // call_raw typed routes return real demo shapes (not generic ok payload).
        let raw_mcp = backend
            .call_raw("mcpServerStatus/list", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            raw_mcp
                .get("data")
                .and_then(|d| d.as_array())
                .map(|a| a.len() >= 2)
                .unwrap_or(false),
            "call_raw mcpServerStatus/list: {raw_mcp}"
        );
        let raw_plugins = backend
            .call_raw("plugin/list", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            raw_plugins
                .get("marketplaces")
                .and_then(|m| m.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "call_raw plugin/list: {raw_plugins}"
        );
    }

    #[tokio::test]
    async fn fixture_environment_and_collaboration_modes() {
        use crate::environment::{
            CollaborationModeListParams, EnvironmentAddParams, EnvironmentInfoParams,
            EnvironmentKind, EnvironmentStatusKind, EnvironmentStatusParams,
        };

        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let catalog = backend.environment_catalog();
        assert_eq!(catalog.len(), 2);
        assert!(catalog
            .iter()
            .any(|e| e.id == "local" && e.kind == EnvironmentKind::Local));
        assert!(catalog
            .iter()
            .any(|e| e.id == "remote-stub" && e.kind == EnvironmentKind::Remote));

        let info = backend
            .environment_info(EnvironmentInfoParams::new("local"))
            .await
            .unwrap();
        assert_eq!(info.shell.name, "bash");
        assert!(info.cwd.is_some());

        let st = backend
            .environment_status(EnvironmentStatusParams::new("local"))
            .await
            .unwrap();
        assert_eq!(st.status, EnvironmentStatusKind::Ready);

        let st_remote = backend
            .environment_status(EnvironmentStatusParams::new("remote-stub"))
            .await
            .unwrap();
        assert_eq!(st_remote.status, EnvironmentStatusKind::Disconnected);

        let unknown = backend
            .environment_status(EnvironmentStatusParams::new("missing"))
            .await
            .unwrap();
        assert_eq!(unknown.status, EnvironmentStatusKind::Unknown);

        backend
            .environment_add(EnvironmentAddParams::new(
                "env-added",
                "wss://fixture.example/new-exec",
            ))
            .await
            .unwrap();
        let catalog2 = backend.environment_catalog();
        assert_eq!(catalog2.len(), 3);
        assert!(catalog2.iter().any(|e| e.id == "env-added"));
        let st_added = backend
            .environment_status(EnvironmentStatusParams::new("env-added"))
            .await
            .unwrap();
        assert_eq!(st_added.status, EnvironmentStatusKind::Pending);

        let modes = backend
            .collaboration_mode_list(CollaborationModeListParams::default())
            .await
            .unwrap();
        assert!(
            (2..=4).contains(&modes.data.len()),
            "modes: {}",
            modes.data.len()
        );

        // call_raw typed routes return real shapes (not generic ok).
        let raw_status = backend
            .call_raw(
                "environment/status",
                serde_json::json!({ "environmentId": "local" }),
            )
            .await
            .unwrap();
        assert_eq!(
            raw_status.get("status").and_then(|s| s.as_str()),
            Some("ready")
        );

        let raw_info = backend
            .call_raw(
                "environment/info",
                serde_json::json!({ "environmentId": "local" }),
            )
            .await
            .unwrap();
        assert!(raw_info.get("shell").is_some());

        let raw_modes = backend
            .call_raw("collaborationMode/list", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            raw_modes
                .get("data")
                .and_then(|d| d.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "call_raw collaborationMode/list: {raw_modes}"
        );
    }

    #[tokio::test]
    async fn fixture_thread_goal_set_get_clear() {
        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let get_empty = backend
            .thread_goal_get(ThreadGoalGetParams::new("th-work-1"))
            .await
            .unwrap();
        assert!(get_empty.goal.is_none());

        let set = backend
            .thread_goal_set(
                ThreadGoalSetParams::new("th-work-1")
                    .with_objective("Ship Chat and Work modes")
                    .with_status(crate::protocol::ThreadGoalStatus::Active),
            )
            .await
            .unwrap();
        assert_eq!(set.goal.thread_id, "th-work-1");
        assert_eq!(set.goal.objective, "Ship Chat and Work modes");
        assert_eq!(set.goal.status, crate::protocol::ThreadGoalStatus::Active);
        assert_eq!(set.goal.tokens_used, 0);

        let get = backend
            .thread_goal_get(ThreadGoalGetParams::new("th-work-1"))
            .await
            .unwrap();
        assert_eq!(
            get.goal.as_ref().map(|g| g.objective.as_str()),
            Some("Ship Chat and Work modes")
        );

        // Update objective via set
        let set2 = backend
            .thread_goal_set(
                ThreadGoalSetParams::new("th-work-1")
                    .with_objective("Ship multi-mode shell")
                    .with_status(crate::protocol::ThreadGoalStatus::Paused),
            )
            .await
            .unwrap();
        assert_eq!(set2.goal.objective, "Ship multi-mode shell");
        assert_eq!(set2.goal.status, crate::protocol::ThreadGoalStatus::Paused);

        let raw = backend
            .call_raw(
                "thread/goal/get",
                serde_json::json!({ "threadId": "th-work-1" }),
            )
            .await
            .unwrap();
        assert_eq!(raw["goal"]["threadId"], "th-work-1");
        assert_eq!(raw["goal"]["objective"], "Ship multi-mode shell");
        assert_eq!(raw["goal"]["status"], "paused");

        let cleared = backend
            .thread_goal_clear(ThreadGoalClearParams::new("th-work-1"))
            .await
            .unwrap();
        assert!(cleared.cleared);

        let get_after = backend
            .thread_goal_get(ThreadGoalGetParams::new("th-work-1"))
            .await
            .unwrap();
        assert!(get_after.goal.is_none());

        let clear_again = backend
            .thread_goal_clear(ThreadGoalClearParams::new("th-work-1"))
            .await
            .unwrap();
        assert!(!clear_again.cleared);

        assert!(backend.fixture_goals().is_empty());
    }

    #[tokio::test]
    async fn fixture_account_read_usage_rate_limits() {
        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        let account = backend
            .account_read(GetAccountParams::default())
            .await
            .unwrap();
        assert!(account.has_account(), "fixture defaults to signed-in demo");
        assert!(account.requires_openai_auth);
        let acc = account.account.as_ref().unwrap();
        assert_eq!(acc.plan_type(), Some(crate::account::PlanType::Pro));
        assert_eq!(
            acc.email_display().as_deref(),
            Some(crate::account::FIXTURE_DEMO_EMAIL_MASKED)
        );

        let usage = backend.account_usage_read().await.unwrap();
        assert!(usage.summary.lifetime_tokens.unwrap_or(0) > 0);
        assert_eq!(usage.daily_usage_buckets.as_ref().map(|b| b.len()), Some(4));

        let limits = backend.account_rate_limits_read().await.unwrap();
        assert_eq!(
            limits.rate_limits.primary.as_ref().map(|w| w.used_percent),
            Some(42)
        );
        assert_eq!(
            limits
                .rate_limits
                .secondary
                .as_ref()
                .map(|w| w.used_percent),
            Some(18)
        );
        assert_eq!(
            limits.rate_limits.plan_type,
            Some(crate::account::PlanType::Pro)
        );

        // call_raw returns typed demo shapes
        let raw = backend
            .call_raw("account/read", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(raw["account"]["type"], "chatgpt");
        assert_eq!(raw["account"]["planType"], "pro");
        assert!(raw
            .get("requiresOpenaiAuth")
            .and_then(|v| v.as_bool())
            .unwrap());

        let raw_usage = backend
            .call_raw("account/usage/read", serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            raw_usage["summary"]["lifetimeTokens"].as_i64().unwrap_or(0) > 0,
            "call_raw usage: {raw_usage}"
        );

        let raw_limits = backend
            .call_raw("account/rateLimits/read", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(raw_limits["rateLimits"]["primary"]["usedPercent"], 42);
    }

    #[tokio::test]
    async fn fixture_account_login_logout_cancel() {
        let backend = FixtureBackend::new();
        backend.connect().await.unwrap();

        // Sign out
        backend.account_logout().await.unwrap();
        let after_logout = backend
            .account_read(GetAccountParams::default())
            .await
            .unwrap();
        assert!(!after_logout.has_account());
        assert!(after_logout.account.is_none());

        // Device-code login stub (no network) → returns URL + code and signs in
        let login = backend
            .account_login_start(LoginAccountParams::device_code())
            .await
            .unwrap();
        assert_eq!(
            login.device_url(),
            Some(crate::account::FIXTURE_LOGIN_VERIFICATION_URL)
        );
        assert_eq!(
            login.user_code(),
            Some(crate::account::FIXTURE_LOGIN_USER_CODE)
        );
        assert_eq!(login.login_id(), Some(crate::account::FIXTURE_LOGIN_ID));

        let after_login = backend
            .account_read(GetAccountParams::default())
            .await
            .unwrap();
        assert!(after_login.has_account());

        // Cancel pending login (fixture keeps signed-in; cancel only clears pending id)
        let cancel = backend
            .account_login_cancel(CancelLoginAccountParams::new(
                crate::account::FIXTURE_LOGIN_ID,
            ))
            .await
            .unwrap();
        assert_eq!(cancel.status, CancelLoginAccountStatus::Canceled);

        let cancel_missing = backend
            .account_login_cancel(CancelLoginAccountParams::new("no-such-login"))
            .await
            .unwrap();
        assert_eq!(cancel_missing.status, CancelLoginAccountStatus::NotFound);

        // call_raw login start
        let raw_login = backend
            .call_raw(
                "account/login/start",
                serde_json::json!({ "type": "chatgptDeviceCode" }),
            )
            .await
            .unwrap();
        assert_eq!(raw_login["type"], "chatgptDeviceCode");
        assert_eq!(
            raw_login["userCode"],
            crate::account::FIXTURE_LOGIN_USER_CODE
        );
        assert!(raw_login.get("verificationUrl").is_some());
    }
}
