//! Pluggable agent backend trait.

use async_trait::async_trait;
use serde_json::Value;

use crate::account::{
    CancelLoginAccountParams, CancelLoginAccountResponse, GetAccountParams,
    GetAccountRateLimitsResponse, GetAccountResponse, GetAccountTokenUsageResponse,
    LoginAccountParams, LoginAccountResponse, LogoutAccountResponse,
};
use crate::environment::{
    CollaborationModeListParams, CollaborationModeListResponse, EnvironmentAddParams,
    EnvironmentAddResponse, EnvironmentInfoParams, EnvironmentInfoResponse,
    EnvironmentStatusParams, EnvironmentStatusResponse, EnvironmentSummary,
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
use crate::methods::is_known_client_method;
use crate::process::{
    ProcessKillParams, ProcessKillResponse, ProcessResizePtyParams, ProcessResizePtyResponse,
    ProcessSpawnParams, ProcessSpawnResponse, ProcessWriteStdinParams, ProcessWriteStdinResponse,
};
use crate::protocol::{
    ConfigReadParams, ConfigReadResponse, InitializeResponse, ModelListParams, ModelListResponse,
    SkillsListParams, SkillsListResponse, ThreadArchiveParams, ThreadArchiveResponse,
    ThreadDeleteParams, ThreadDeleteResponse, ThreadForkParams, ThreadForkResponse,
    ThreadGoalClearParams, ThreadGoalClearResponse, ThreadGoalGetParams, ThreadGoalGetResponse,
    ThreadGoalSetParams, ThreadGoalSetResponse, ThreadListParams, ThreadListResponse,
    ThreadReadParams, ThreadReadResponse, ThreadResumeParams, ThreadResumeResponse,
    ThreadSearchParams, ThreadSearchResponse, ThreadSetNameParams, ThreadSetNameResponse,
    ThreadStartParams, ThreadStartResponse, ThreadUnarchiveParams, ThreadUnarchiveResponse,
    TurnInterruptParams, TurnInterruptResponse, TurnStartParams, TurnStartResponse,
    TurnSteerParams, TurnSteerResponse,
};
use crate::types::{ConnectionStatus, Result};

/// Abstraction over Codex app-server, offline fixtures, and a future Mitsuro server.
///
/// UI code should depend only on this trait so backends stay swappable.
///
/// Prefer typed helpers for well-known methods; use [`Self::call_raw`] for the full
/// app-server surface (method names inventoried in `fixtures/client-methods.txt`).
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Human-readable backend name (e.g. `"codex-app-server"`, `"fixture"`, `"mitsuro"`).
    fn name(&self) -> &'static str;

    fn status(&self) -> ConnectionStatus;

    /// Whether this backend claims support for `method` (registry-known and/or implemented).
    ///
    /// Default: known client methods from the protocol bar.
    fn supports_method(&self, method: &str) -> bool {
        is_known_client_method(method)
    }

    /// Universal JSON-RPC: invoke any client method with raw JSON params.
    ///
    /// - **Codex**: forwards over the live stdio request path for *any* method string.
    /// - **Fixture**: executes only its typed offline implementations and returns
    ///   `NotImplemented` for known methods without a truthful fixture behavior.
    /// - **Mitsuro stub**: [`crate::types::AgentError::NotImplemented`].
    async fn call_raw(&self, method: &str, params: Value) -> Result<Value>;

    /// Establish connection / handshake (for Codex: spawn + `initialize`).
    async fn connect(&self) -> Result<InitializeResponse>;

    async fn thread_list(&self, params: ThreadListParams) -> Result<ThreadListResponse>;

    async fn thread_start(&self, params: ThreadStartParams) -> Result<ThreadStartResponse>;

    /// Load a thread (optionally with turns/items). May be [`crate::types::AgentError::NotImplemented`].
    async fn thread_read(&self, params: ThreadReadParams) -> Result<ThreadReadResponse>;

    /// Catalog models via `model/list` (fixture returns demo models offline).
    async fn model_list(&self, params: ModelListParams) -> Result<ModelListResponse>;

    /// Effective config via `config/read` (fixture returns demo config offline).
    async fn config_read(&self, params: ConfigReadParams) -> Result<ConfigReadResponse>;

    /// Full-text / substring thread search via `thread/search`.
    /// UI may also filter locally; this is the server method when available.
    async fn thread_search(&self, params: ThreadSearchParams) -> Result<ThreadSearchResponse>;

    /// Set a user-facing thread title via `thread/name/set`.
    async fn thread_name_set(&self, params: ThreadSetNameParams) -> Result<ThreadSetNameResponse>;

    /// Archive a thread via `thread/archive`.
    async fn thread_archive(&self, params: ThreadArchiveParams) -> Result<ThreadArchiveResponse>;

    /// Unarchive a thread via `thread/unarchive`.
    async fn thread_unarchive(
        &self,
        params: ThreadUnarchiveParams,
    ) -> Result<ThreadUnarchiveResponse>;

    /// Permanently delete a thread via `thread/delete`.
    async fn thread_delete(&self, params: ThreadDeleteParams) -> Result<ThreadDeleteResponse>;

    /// Fork a thread into a new thread via `thread/fork`.
    async fn thread_fork(&self, params: ThreadForkParams) -> Result<ThreadForkResponse>;

    /// Resume / rejoin a thread via `thread/resume` (distinct from `thread/start`).
    async fn thread_resume(&self, params: ThreadResumeParams) -> Result<ThreadResumeResponse>;

    /// Read the long-running goal attached to a thread via `thread/goal/get`.
    async fn thread_goal_get(&self, params: ThreadGoalGetParams) -> Result<ThreadGoalGetResponse>;

    /// Create or update a thread goal via `thread/goal/set`.
    async fn thread_goal_set(&self, params: ThreadGoalSetParams) -> Result<ThreadGoalSetResponse>;

    /// Clear a thread goal via `thread/goal/clear`.
    async fn thread_goal_clear(
        &self,
        params: ThreadGoalClearParams,
    ) -> Result<ThreadGoalClearResponse>;

    /// List skills via `skills/list` (best-effort; fixture returns demo skills).
    async fn skills_list(&self, params: SkillsListParams) -> Result<SkillsListResponse>;

    /// Start a model turn. Live backends may incur paid usage — prefer fixtures offline.
    /// Callers should pass selected model in [`TurnStartParams::model`] when known.
    async fn turn_start(&self, params: TurnStartParams) -> Result<TurnStartResponse>;

    /// Inject user input into the active turn without starting a second turn.
    async fn turn_steer(&self, _params: TurnSteerParams) -> Result<TurnSteerResponse> {
        Err(crate::types::AgentError::NotImplemented(
            "turn/steer is not implemented by this backend".to_owned(),
        ))
    }

    /// Interrupt an in-progress turn via `turn/interrupt`.
    async fn turn_interrupt(&self, params: TurnInterruptParams) -> Result<TurnInterruptResponse>;

    /// Spawn a standalone host process via `process/spawn`.
    /// Output/exit arrive as `process/outputDelta` / `process/exited` notifications.
    async fn process_spawn(&self, params: ProcessSpawnParams) -> Result<ProcessSpawnResponse>;

    /// Write stdin (base64) and/or close stdin for a running process.
    async fn process_write_stdin(
        &self,
        params: ProcessWriteStdinParams,
    ) -> Result<ProcessWriteStdinResponse>;

    /// Resize a PTY-backed process.
    async fn process_resize_pty(
        &self,
        params: ProcessResizePtyParams,
    ) -> Result<ProcessResizePtyResponse>;

    /// Terminate a running process by client-supplied handle.
    async fn process_kill(&self, params: ProcessKillParams) -> Result<ProcessKillResponse>;

    /// List direct children of a directory via `fs/readDirectory`.
    async fn fs_read_directory(
        &self,
        params: FsReadDirectoryParams,
    ) -> Result<FsReadDirectoryResponse>;

    /// Read file contents (base64) via `fs/readFile`.
    async fn fs_read_file(&self, params: FsReadFileParams) -> Result<FsReadFileResponse>;

    /// Path metadata via `fs/getMetadata`.
    async fn fs_get_metadata(&self, params: FsGetMetadataParams) -> Result<FsGetMetadataResponse>;

    /// One-shot fuzzy file search via `fuzzyFileSearch`.
    async fn fuzzy_file_search(
        &self,
        params: FuzzyFileSearchParams,
    ) -> Result<FuzzyFileSearchResponse>;

    /// Start a fuzzy search session via `fuzzyFileSearch/sessionStart`.
    async fn fuzzy_file_search_session_start(
        &self,
        params: FuzzyFileSearchSessionStartParams,
    ) -> Result<FuzzyFileSearchSessionStartResponse>;

    /// Update query on a fuzzy search session via `fuzzyFileSearch/sessionUpdate`.
    async fn fuzzy_file_search_session_update(
        &self,
        params: FuzzyFileSearchSessionUpdateParams,
    ) -> Result<FuzzyFileSearchSessionUpdateResponse>;

    /// Stop a fuzzy search session via `fuzzyFileSearch/sessionStop`.
    async fn fuzzy_file_search_session_stop(
        &self,
        params: FuzzyFileSearchSessionStopParams,
    ) -> Result<FuzzyFileSearchSessionStopResponse>;

    /// List MCP server statuses via `mcpServerStatus/list`.
    async fn mcp_server_status_list(
        &self,
        params: ListMcpServerStatusParams,
    ) -> Result<ListMcpServerStatusResponse>;

    /// Invoke an MCP tool via `mcpServer/tool/call` (fixture is offline-safe only).
    async fn mcp_server_tool_call(
        &self,
        params: McpServerToolCallParams,
    ) -> Result<McpServerToolCallResponse>;

    /// List plugins / marketplaces via `plugin/list`.
    async fn plugin_list(&self, params: PluginListParams) -> Result<PluginListResponse>;

    /// Read plugin detail via `plugin/read`.
    async fn plugin_read(&self, params: PluginReadParams) -> Result<PluginReadResponse>;

    /// List installed plugins via `plugin/installed`.
    async fn plugin_installed(
        &self,
        params: PluginInstalledParams,
    ) -> Result<PluginInstalledResponse>;

    /// Read environment shell/cwd via `environment/info`.
    async fn environment_info(
        &self,
        params: EnvironmentInfoParams,
    ) -> Result<EnvironmentInfoResponse>;

    /// Probe environment connection status via `environment/status`.
    async fn environment_status(
        &self,
        params: EnvironmentStatusParams,
    ) -> Result<EnvironmentStatusResponse>;

    /// Register a remote environment via `environment/add`.
    async fn environment_add(&self, params: EnvironmentAddParams)
        -> Result<EnvironmentAddResponse>;

    /// List collaboration mode presets via `collaborationMode/list`.
    async fn collaboration_mode_list(
        &self,
        params: CollaborationModeListParams,
    ) -> Result<CollaborationModeListResponse>;

    /// Offline/UI catalog of environments (no protocol `environment/list`).
    ///
    /// Default: empty. Fixture returns demo local + remote stub rows.
    fn environment_catalog(&self) -> Vec<EnvironmentSummary> {
        Vec::new()
    }

    /// Read account profile via `account/read` (no paid model call).
    async fn account_read(&self, params: GetAccountParams) -> Result<GetAccountResponse>;

    /// Start login via `account/login/start` (fixture returns device URL + code; no network).
    async fn account_login_start(&self, params: LoginAccountParams)
        -> Result<LoginAccountResponse>;

    /// Cancel an in-progress login via `account/login/cancel`.
    async fn account_login_cancel(
        &self,
        params: CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse>;

    /// Log out via `account/logout`.
    async fn account_logout(&self) -> Result<LogoutAccountResponse>;

    /// Token usage summary via `account/usage/read` (fixture demo numbers offline).
    async fn account_usage_read(&self) -> Result<GetAccountTokenUsageResponse>;

    /// Rate-limit windows via `account/rateLimits/read` (fixture demo % offline).
    async fn account_rate_limits_read(&self) -> Result<GetAccountRateLimitsResponse>;

    /// Best-effort shutdown of the child process / connection.
    async fn disconnect(&self) -> Result<()>;
}
