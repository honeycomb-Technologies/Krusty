//! Pluggable agent backend trait.

use async_trait::async_trait;
use serde_json::Value;

use crate::account::{
    CancelLoginAccountParams, CancelLoginAccountResponse, GetAccountParams,
    GetAccountRateLimitsResponse, GetAccountResponse, GetAccountTokenUsageResponse,
    LoginAccountParams, LoginAccountResponse, LogoutAccountResponse,
};
use crate::apps::{
    AppsInstalledParams, AppsInstalledResponse, AppsListParams, AppsListResponse, AppsReadParams,
    AppsReadResponse,
};
use crate::command::{
    CommandExecParams, CommandExecResizeParams, CommandExecResizeResponse, CommandExecResponse,
    CommandExecTerminateParams, CommandExecTerminateResponse, CommandExecWriteParams,
    CommandExecWriteResponse,
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
use crate::external_agent_config::{
    ExternalAgentConfigDetectParams, ExternalAgentConfigDetectResponse,
    ExternalAgentConfigImportHistoriesReadResponse, ExternalAgentConfigImportHistoryRecordParams,
    ExternalAgentConfigImportHistoryRecordResponse, ExternalAgentConfigImportParams,
    ExternalAgentConfigImportResponse,
};
use crate::fs::{
    FsCopyParams, FsCopyResponse, FsCreateDirectoryParams, FsCreateDirectoryResponse,
    FsGetMetadataParams, FsGetMetadataResponse, FsReadDirectoryParams, FsReadDirectoryResponse,
    FsReadFileParams, FsReadFileResponse, FsRemoveParams, FsRemoveResponse, FsUnwatchParams,
    FsUnwatchResponse, FsWatchParams, FsWatchResponse, FsWriteFileParams, FsWriteFileResponse,
    FuzzyFileSearchParams, FuzzyFileSearchResponse, FuzzyFileSearchSessionStartParams,
    FuzzyFileSearchSessionStartResponse, FuzzyFileSearchSessionStopParams,
    FuzzyFileSearchSessionStopResponse, FuzzyFileSearchSessionUpdateParams,
    FuzzyFileSearchSessionUpdateResponse,
};
use crate::mcp_auth::{McpServerOauthLoginParams, McpServerOauthLoginResponse};
use crate::mcp_config::{
    ConfigMcpServerReloadResponse, ConfigValueWriteParams, ConfigWriteResponse,
};
use crate::methods::is_known_client_method;
use crate::permissions::{
    ConfigRequirementsReadResponse, ModelProviderCapabilitiesReadParams,
    ModelProviderCapabilitiesReadResponse, PermissionProfileListParams,
    PermissionProfileListResponse,
};
use crate::plugin_mutations::{
    PluginInstallParams, PluginInstallResponse, PluginUninstallParams, PluginUninstallResponse,
};
use crate::process::{
    ProcessKillParams, ProcessKillResponse, ProcessResizePtyParams, ProcessResizePtyResponse,
    ProcessSpawnParams, ProcessSpawnResponse, ProcessWriteStdinParams, ProcessWriteStdinResponse,
    ThreadBackgroundTerminalsCleanParams, ThreadBackgroundTerminalsCleanResponse,
    ThreadBackgroundTerminalsListParams, ThreadBackgroundTerminalsListResponse,
    ThreadBackgroundTerminalsTerminateParams, ThreadBackgroundTerminalsTerminateResponse,
};
use crate::protocol::{
    ConfigReadParams, ConfigReadResponse, InitializeResponse, ModelListParams, ModelListResponse,
    ReviewStartParams, ReviewStartResponse, SkillsListParams, SkillsListResponse,
    ThreadArchiveParams, ThreadArchiveResponse, ThreadCompactStartParams,
    ThreadCompactStartResponse, ThreadDeleteParams, ThreadDeleteResponse, ThreadForkParams,
    ThreadForkResponse, ThreadGoalClearParams, ThreadGoalClearResponse, ThreadGoalGetParams,
    ThreadGoalGetResponse, ThreadGoalSetParams, ThreadGoalSetResponse, ThreadListParams,
    ThreadListResponse, ThreadReadParams, ThreadReadResponse, ThreadResumeParams,
    ThreadResumeResponse, ThreadSearchParams, ThreadSearchResponse, ThreadSetNameParams,
    ThreadSetNameResponse, ThreadStartParams, ThreadStartResponse, ThreadUnarchiveParams,
    ThreadUnarchiveResponse, TurnInterruptParams, TurnInterruptResponse, TurnStartParams,
    TurnStartResponse, TurnSteerParams, TurnSteerResponse,
};
use crate::remote_control::{
    RemoteControlClientsListParams, RemoteControlClientsListResponse,
    RemoteControlClientsRevokeParams, RemoteControlClientsRevokeResponse,
    RemoteControlDisableParams, RemoteControlDisableResponse, RemoteControlEnableParams,
    RemoteControlEnableResponse, RemoteControlPairingStartParams,
    RemoteControlPairingStartResponse, RemoteControlPairingStatusParams,
    RemoteControlPairingStatusResponse, RemoteControlStatusReadResponse,
};
use crate::types::{ConnectionStatus, Result};
use crate::{
    HooksListParams, HooksListResponse, SkillsConfigWriteParams, SkillsConfigWriteResponse,
};

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

    /// Persist one value through Codex `config/value/write`.
    async fn config_value_write(
        &self,
        _params: ConfigValueWriteParams,
    ) -> Result<ConfigWriteResponse> {
        Err(crate::AgentError::NotImplemented(
            "configuration writes are not implemented by this backend".to_owned(),
        ))
    }

    /// Reload configured MCP servers after a successful config write.
    async fn config_mcp_server_reload(&self) -> Result<ConfigMcpServerReloadResponse> {
        Err(crate::AgentError::NotImplemented(
            "MCP configuration reload is not implemented by this backend".to_owned(),
        ))
    }

    /// List permission profiles available for the effective project config.
    async fn permission_profile_list(
        &self,
        _params: PermissionProfileListParams,
    ) -> Result<PermissionProfileListResponse> {
        Err(crate::AgentError::NotImplemented(
            "permission profile listing is not implemented by this backend".to_owned(),
        ))
    }

    /// Read enterprise/managed requirements that narrow effective config choices.
    async fn config_requirements_read(&self) -> Result<ConfigRequirementsReadResponse> {
        Err(crate::AgentError::NotImplemented(
            "configuration requirements are not implemented by this backend".to_owned(),
        ))
    }

    /// Read provider-level tool capabilities for the active model provider.
    async fn model_provider_capabilities_read(
        &self,
        _params: ModelProviderCapabilitiesReadParams,
    ) -> Result<ModelProviderCapabilitiesReadResponse> {
        Err(crate::AgentError::NotImplemented(
            "model provider capabilities are not implemented by this backend".to_owned(),
        ))
    }

    /// Detect importable external-agent configuration without changing it.
    async fn external_agent_config_detect(
        &self,
        _params: ExternalAgentConfigDetectParams,
    ) -> Result<ExternalAgentConfigDetectResponse> {
        Err(crate::AgentError::NotImplemented(
            "external-agent configuration detection is not implemented by this backend".to_owned(),
        ))
    }

    /// Import selected items returned by [`Self::external_agent_config_detect`].
    async fn external_agent_config_import(
        &self,
        _params: ExternalAgentConfigImportParams,
    ) -> Result<ExternalAgentConfigImportResponse> {
        Err(crate::AgentError::NotImplemented(
            "external-agent configuration import is not implemented by this backend".to_owned(),
        ))
    }

    /// Read completed external-agent import history.
    async fn external_agent_config_import_read_histories(
        &self,
    ) -> Result<ExternalAgentConfigImportHistoriesReadResponse> {
        Err(crate::AgentError::NotImplemented(
            "external-agent import history is not implemented by this backend".to_owned(),
        ))
    }

    /// Record an import completed outside app-server.
    async fn external_agent_config_import_record_history(
        &self,
        _params: ExternalAgentConfigImportHistoryRecordParams,
    ) -> Result<ExternalAgentConfigImportHistoryRecordResponse> {
        Err(crate::AgentError::NotImplemented(
            "external-agent import history recording is not implemented by this backend".to_owned(),
        ))
    }

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

    /// Start server-side context compaction for a thread.
    async fn thread_compact_start(
        &self,
        _params: ThreadCompactStartParams,
    ) -> Result<ThreadCompactStartResponse> {
        Err(crate::types::AgentError::NotImplemented(
            "thread/compact/start is not implemented by this backend".to_owned(),
        ))
    }

    /// Start a code-review turn.
    async fn review_start(&self, _params: ReviewStartParams) -> Result<ReviewStartResponse> {
        Err(crate::types::AgentError::NotImplemented(
            "review/start is not implemented by this backend".to_owned(),
        ))
    }

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

    /// Enable or disable one discovered Codex skill.
    async fn skills_config_write(
        &self,
        _params: SkillsConfigWriteParams,
    ) -> Result<SkillsConfigWriteResponse> {
        Err(crate::AgentError::NotImplemented(
            "skill configuration writes are not implemented by this backend".to_owned(),
        ))
    }

    /// List discovered lifecycle hooks via `hooks/list`.
    async fn hooks_list(&self, _params: HooksListParams) -> Result<HooksListResponse> {
        Err(crate::AgentError::NotImplemented(
            "hook catalog is not implemented by this backend".to_owned(),
        ))
    }

    /// List apps/connectors available to the current Codex account.
    async fn apps_list(&self, _params: AppsListParams) -> Result<AppsListResponse> {
        Err(crate::AgentError::NotImplemented(
            "app catalog is not implemented by this backend".to_owned(),
        ))
    }

    /// Read the committed installed connector runtime snapshot.
    async fn apps_installed(&self, _params: AppsInstalledParams) -> Result<AppsInstalledResponse> {
        Err(crate::AgentError::NotImplemented(
            "installed app snapshot is not implemented by this backend".to_owned(),
        ))
    }

    /// Read detailed metadata for specific apps/connectors.
    async fn apps_read(&self, _params: AppsReadParams) -> Result<AppsReadResponse> {
        Err(crate::AgentError::NotImplemented(
            "app metadata is not implemented by this backend".to_owned(),
        ))
    }

    /// Read whether this app-server installation accepts Remote Control clients.
    async fn remote_control_status_read(&self) -> Result<RemoteControlStatusReadResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control status is not implemented by this backend".to_owned(),
        ))
    }

    /// Allow authorized clients to discover and control this Codex installation.
    async fn remote_control_enable(
        &self,
        _params: RemoteControlEnableParams,
    ) -> Result<RemoteControlEnableResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control enablement is not implemented by this backend".to_owned(),
        ))
    }

    /// Stop accepting Remote Control connections for this Codex installation.
    async fn remote_control_disable(
        &self,
        _params: RemoteControlDisableParams,
    ) -> Result<RemoteControlDisableResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control disablement is not implemented by this backend".to_owned(),
        ))
    }

    /// Create a short-lived device-pairing code.
    async fn remote_control_pairing_start(
        &self,
        _params: RemoteControlPairingStartParams,
    ) -> Result<RemoteControlPairingStartResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control pairing is not implemented by this backend".to_owned(),
        ))
    }

    /// Check whether a pairing code has been claimed.
    async fn remote_control_pairing_status(
        &self,
        _params: RemoteControlPairingStatusParams,
    ) -> Result<RemoteControlPairingStatusResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control pairing status is not implemented by this backend".to_owned(),
        ))
    }

    /// List devices authorized for one Remote Control environment.
    async fn remote_control_clients_list(
        &self,
        _params: RemoteControlClientsListParams,
    ) -> Result<RemoteControlClientsListResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control client listing is not implemented by this backend".to_owned(),
        ))
    }

    /// Revoke one authorized Remote Control client.
    async fn remote_control_clients_revoke(
        &self,
        _params: RemoteControlClientsRevokeParams,
    ) -> Result<RemoteControlClientsRevokeResponse> {
        Err(crate::AgentError::NotImplemented(
            "remote control client revocation is not implemented by this backend".to_owned(),
        ))
    }

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

    /// Run a standalone command in the Codex server sandbox. The response resolves on exit.
    async fn command_exec(&self, _params: CommandExecParams) -> Result<CommandExecResponse> {
        Err(crate::AgentError::NotImplemented(
            "standalone command execution is not implemented by this backend".to_owned(),
        ))
    }

    async fn command_exec_write(
        &self,
        _params: CommandExecWriteParams,
    ) -> Result<CommandExecWriteResponse> {
        Err(crate::AgentError::NotImplemented(
            "standalone command stdin is not implemented by this backend".to_owned(),
        ))
    }

    async fn command_exec_resize(
        &self,
        _params: CommandExecResizeParams,
    ) -> Result<CommandExecResizeResponse> {
        Err(crate::AgentError::NotImplemented(
            "standalone command PTY resizing is not implemented by this backend".to_owned(),
        ))
    }

    async fn command_exec_terminate(
        &self,
        _params: CommandExecTerminateParams,
    ) -> Result<CommandExecTerminateResponse> {
        Err(crate::AgentError::NotImplemented(
            "standalone command termination is not implemented by this backend".to_owned(),
        ))
    }

    /// List shell processes retained by one Codex thread.
    async fn thread_background_terminals_list(
        &self,
        _params: ThreadBackgroundTerminalsListParams,
    ) -> Result<ThreadBackgroundTerminalsListResponse> {
        Err(crate::AgentError::NotImplemented(
            "thread background-terminal listing is not implemented by this backend".to_owned(),
        ))
    }

    /// Remove completed background-terminal records for one Codex thread.
    async fn thread_background_terminals_clean(
        &self,
        _params: ThreadBackgroundTerminalsCleanParams,
    ) -> Result<ThreadBackgroundTerminalsCleanResponse> {
        Err(crate::AgentError::NotImplemented(
            "thread background-terminal cleanup is not implemented by this backend".to_owned(),
        ))
    }

    /// Terminate one thread-owned background terminal by process id.
    async fn thread_background_terminals_terminate(
        &self,
        _params: ThreadBackgroundTerminalsTerminateParams,
    ) -> Result<ThreadBackgroundTerminalsTerminateResponse> {
        Err(crate::AgentError::NotImplemented(
            "thread background-terminal termination is not implemented by this backend".to_owned(),
        ))
    }

    /// List direct children of a directory via `fs/readDirectory`.
    async fn fs_read_directory(
        &self,
        params: FsReadDirectoryParams,
    ) -> Result<FsReadDirectoryResponse>;

    /// Read file contents (base64) via `fs/readFile`.
    async fn fs_read_file(&self, params: FsReadFileParams) -> Result<FsReadFileResponse>;

    /// Write file contents (base64) via `fs/writeFile`.
    async fn fs_write_file(&self, _params: FsWriteFileParams) -> Result<FsWriteFileResponse> {
        Err(crate::AgentError::NotImplemented(
            "filesystem writes are not implemented by this backend".to_owned(),
        ))
    }

    /// Create a directory via `fs/createDirectory`.
    async fn fs_create_directory(
        &self,
        _params: FsCreateDirectoryParams,
    ) -> Result<FsCreateDirectoryResponse> {
        Err(crate::AgentError::NotImplemented(
            "directory creation is not implemented by this backend".to_owned(),
        ))
    }

    /// Remove a file or directory via `fs/remove`.
    async fn fs_remove(&self, _params: FsRemoveParams) -> Result<FsRemoveResponse> {
        Err(crate::AgentError::NotImplemented(
            "filesystem removal is not implemented by this backend".to_owned(),
        ))
    }

    /// Copy a file or directory tree via `fs/copy`.
    async fn fs_copy(&self, _params: FsCopyParams) -> Result<FsCopyResponse> {
        Err(crate::AgentError::NotImplemented(
            "filesystem copy is not implemented by this backend".to_owned(),
        ))
    }

    /// Start filesystem notifications via `fs/watch`.
    async fn fs_watch(&self, _params: FsWatchParams) -> Result<FsWatchResponse> {
        Err(crate::AgentError::NotImplemented(
            "filesystem watches are not implemented by this backend".to_owned(),
        ))
    }

    /// Stop filesystem notifications via `fs/unwatch`.
    async fn fs_unwatch(&self, _params: FsUnwatchParams) -> Result<FsUnwatchResponse> {
        Err(crate::AgentError::NotImplemented(
            "filesystem watches are not implemented by this backend".to_owned(),
        ))
    }

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

    async fn mcp_server_oauth_login(
        &self,
        _params: McpServerOauthLoginParams,
    ) -> Result<McpServerOauthLoginResponse> {
        Err(crate::AgentError::NotImplemented(
            "MCP OAuth login is not implemented by this backend".to_owned(),
        ))
    }

    /// List plugins / marketplaces via `plugin/list`.
    async fn plugin_list(&self, params: PluginListParams) -> Result<PluginListResponse>;

    /// Read plugin detail via `plugin/read`.
    async fn plugin_read(&self, params: PluginReadParams) -> Result<PluginReadResponse>;

    /// List installed plugins via `plugin/installed`.
    async fn plugin_installed(
        &self,
        params: PluginInstalledParams,
    ) -> Result<PluginInstalledResponse>;

    /// Install a plugin via `plugin/install`.
    async fn plugin_install(&self, _params: PluginInstallParams) -> Result<PluginInstallResponse> {
        Err(crate::AgentError::NotImplemented(
            "plugin installation is not implemented by this backend".to_owned(),
        ))
    }

    /// Uninstall a plugin via `plugin/uninstall`.
    async fn plugin_uninstall(
        &self,
        _params: PluginUninstallParams,
    ) -> Result<PluginUninstallResponse> {
        Err(crate::AgentError::NotImplemented(
            "plugin removal is not implemented by this backend".to_owned(),
        ))
    }

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

    /// Start login via `account/login/start` (live backends may complete asynchronously).
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
