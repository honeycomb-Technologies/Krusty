//! Mitsuro agent backends and Codex app-server protocol client.

pub mod account;
pub mod approvals;
pub mod backend;
pub mod codex;
pub mod desktop;
pub mod environment;
pub mod extensions;
pub mod fixture;
pub mod fs;
pub mod live_turn;
pub mod methods;
pub mod mitsuro;
pub mod process;
pub mod product;
pub mod protocol;
pub mod types;

pub use account::{
    fixture_demo_account, fixture_demo_account_response, fixture_demo_rate_limits,
    fixture_demo_usage, fixture_login_chatgpt_response, fixture_login_device_code_response,
    fixture_signed_out_account_response, mask_email, Account, AccountTokenUsageDailyBucket,
    AccountTokenUsageSummary, CancelLoginAccountParams, CancelLoginAccountResponse,
    CancelLoginAccountStatus, CreditsSnapshot, GetAccountParams, GetAccountRateLimitsResponse,
    GetAccountResponse, GetAccountTokenUsageResponse, LoginAccountParams, LoginAccountResponse,
    LogoutAccountResponse, PlanType, RateLimitSnapshot, RateLimitWindow, FIXTURE_DEMO_DISPLAY_NAME,
    FIXTURE_DEMO_EMAIL, FIXTURE_DEMO_EMAIL_MASKED, FIXTURE_LOGIN_ID, FIXTURE_LOGIN_USER_CODE,
    FIXTURE_LOGIN_VERIFICATION_URL,
};
pub use approvals::{
    build_approval_result, build_approval_rpc_response, is_approval_method, parse_approval_request,
    ApplyPatchApprovalParams, ApprovalChoice, ApprovalKind, CommandExecutionApprovalDecision,
    CommandExecutionRequestApprovalParams, ExecCommandApprovalParams, FileChange,
    FileChangeApprovalDecision, FileChangeRequestApprovalParams, PendingApproval, ReviewDecision,
    ReviewDecisionResponse, APPLY_PATCH_APPROVAL, EXEC_COMMAND_APPROVAL,
    ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL, ITEM_FILE_CHANGE_REQUEST_APPROVAL,
};
pub use backend::AgentBackend;
pub use codex::{
    codex_bin_available, resolve_codex_bin, CodexAppServerBackend, CodexAppServerConfig,
};
pub use desktop::{
    BackendCapabilities, BackendKind, BackendSelection, BackendSessionId, DesktopBackend,
};
pub use environment::{
    fixture_added_environment_summary, fixture_demo_collaboration_modes, fixture_demo_environments,
    fixture_environment_info, fixture_environment_status, CollaborationModeListParams,
    CollaborationModeListResponse, CollaborationModeMask, EnvironmentAddParams,
    EnvironmentAddResponse, EnvironmentInfoParams, EnvironmentInfoResponse, EnvironmentKind,
    EnvironmentShellInfo, EnvironmentStatusKind, EnvironmentStatusParams,
    EnvironmentStatusResponse, EnvironmentSummary, ModeKind,
};
pub use extensions::{
    fixture_demo_mcp_servers, fixture_demo_plugin_read, fixture_demo_plugins,
    fixture_demo_plugins_installed, fixture_mcp_tool_call, ListMcpServerStatusParams,
    ListMcpServerStatusResponse, McpAuthStatus, McpServerInfo, McpServerStatus,
    McpServerStatusDetail, McpServerToolCallParams, McpServerToolCallResponse, PluginAuthPolicy,
    PluginAvailability, PluginDetail, PluginInstallPolicy, PluginInstalledParams,
    PluginInstalledResponse, PluginInterface, PluginListParams, PluginListResponse,
    PluginMarketplaceEntry, PluginReadParams, PluginReadResponse, PluginSource, PluginSummary,
};
pub use fixture::{
    load_sample_turn_events, replay_events, replay_sample_turn, FixtureBackend, SAMPLE_TURN_JSONL,
};
pub use fs::{
    fixture_fuzzy_search, fixture_get_metadata, fixture_project_tree, fixture_read_directory,
    fixture_read_file, fuzzy_score_name, join_abs, normalize_abs_path, FixtureFsNode,
    FsGetMetadataParams, FsGetMetadataResponse, FsReadDirectoryEntry, FsReadDirectoryParams,
    FsReadDirectoryResponse, FsReadFileParams, FsReadFileResponse, FuzzyFileSearchMatchType,
    FuzzyFileSearchParams, FuzzyFileSearchResponse, FuzzyFileSearchResult,
    FuzzyFileSearchSessionStartParams, FuzzyFileSearchSessionStartResponse,
    FuzzyFileSearchSessionStopParams, FuzzyFileSearchSessionStopResponse,
    FuzzyFileSearchSessionUpdateParams, FuzzyFileSearchSessionUpdateResponse, FIXTURE_PROJECT_ROOT,
};
pub use live_turn::{
    run_live_turn_progressive, run_live_turn_progressive_with_model, run_live_turn_with_bridge,
    run_live_turn_with_bridge_and_model, run_live_turn_with_bridge_blocking,
    run_live_turn_with_bridge_blocking_and_model, run_live_turn_with_policy,
    run_live_turn_with_policy_and_model, run_live_turn_with_policy_blocking, LiveApprovalBridge,
    LiveApprovalPolicy, LiveTurnOutcome, DEFAULT_LIVE_TURN_TIMEOUT,
};
pub use methods::{
    client_methods_txt_path, is_known_client_method, load_client_methods_from_bar, CLIENT_METHODS,
    CLIENT_METHOD_COUNT,
};
pub use mitsuro::MitsuroServerBackend;
pub use process::{
    decode_base64, decode_base64_lossy, encode_base64, parse_process_exited,
    parse_process_output_delta, ProcessKillParams, ProcessKillResponse, ProcessOutputStream,
    ProcessResizePtyParams, ProcessResizePtyResponse, ProcessSpawnParams, ProcessSpawnResponse,
    ProcessTerminalSize, ProcessWriteStdinParams, ProcessWriteStdinResponse,
};
pub use product::{
    ConversationMessage, CreateSession, MessageRole, ProductBackend, ProductDirectoryEntry,
    ProductExtension, ProductFile, ProductFileMatch, ProductHiveRun, ProductHiveSnapshot,
    ProductHiveStatus, ProductMcpServer, ProductModel, ProductProcess, ProductReasoningEffort,
    ProductSchedule, ProductSkill, ProductTurn, SessionConversation, SessionSummary,
};
pub use protocol::{
    command_execution_fields, extract_chat_tail_from_thread, extract_transcript_from_thread,
    file_change_fields, fixture_demo_config, fixture_demo_models, fixture_demo_skills,
    map_notification_to_event, map_server_request_to_event, parse_fixture_jsonl,
    parse_notification_line, summarize_file_changes, user_input_text_value, ClientInfo,
    CommandExecutionFields, ConfigReadParams, ConfigReadResponse, FileChangeFields,
    InitializeCapabilities, InitializeParams, InitializeResponse, JsonRpcError, JsonRpcId,
    JsonRpcMessage, ModelInfo, ModelListParams, ModelListResponse, Notification,
    ReasoningEffortOption, SkillMetadata, SkillsListEntry, SkillsListParams, SkillsListResponse,
    ThreadArchiveParams, ThreadArchiveResponse, ThreadDeleteParams, ThreadDeleteResponse,
    ThreadForkParams, ThreadForkResponse, ThreadGoal, ThreadGoalClearParams,
    ThreadGoalClearResponse, ThreadGoalGetParams, ThreadGoalGetResponse, ThreadGoalSetParams,
    ThreadGoalSetResponse, ThreadGoalStatus, ThreadListParams, ThreadListResponse,
    ThreadReadParams, ThreadReadResponse, ThreadResumeParams, ThreadResumeResponse,
    ThreadSearchParams, ThreadSearchResponse, ThreadSearchResult, ThreadSetNameParams,
    ThreadSetNameResponse, ThreadStartParams, ThreadStartResponse, ThreadSummary,
    ThreadUnarchiveParams, ThreadUnarchiveResponse, TranscriptMessage, TranscriptRole,
    TurnInterruptParams, TurnInterruptResponse, TurnStartParams, TurnStartResponse,
};
pub use types::{AgentError, ConnectionStatus, ItemKind, Result, TurnStreamEvent};
