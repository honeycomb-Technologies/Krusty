//! Mitsuro agent backends and Codex app-server protocol client.

pub mod account;
pub mod approvals;
pub mod apps;
pub mod backend;
pub mod codex;
pub mod command;
pub mod desktop;
pub mod environment;
pub mod experimental_features;
pub mod extensions;
pub mod external_agent_config;
pub mod fixture;
pub mod fs;
pub mod hooks;
pub mod live_turn;
pub mod mcp_auth;
pub mod mcp_config;
pub mod memory;
pub mod methods;
pub mod mitsuro;
pub mod notifications;
pub mod permissions;
pub mod plugin_mutations;
pub mod process;
pub mod product;
pub mod protocol;
pub mod realtime;
pub mod remote_control;
pub mod server_requests;
pub mod skill_config;
pub mod thread_configuration;
pub mod thread_history;
pub mod types;

pub use account::{
    fixture_demo_account, fixture_demo_account_response, fixture_demo_rate_limits,
    fixture_demo_usage, fixture_login_chatgpt_response, fixture_login_device_code_response,
    fixture_signed_out_account_response, mask_email, Account, AccountTokenUsageDailyBucket,
    AccountTokenUsageSummary, AddCreditsNudgeCreditType, AddCreditsNudgeEmailStatus,
    CancelLoginAccountParams, CancelLoginAccountResponse, CancelLoginAccountStatus,
    ConsumeAccountRateLimitResetCreditOutcome, ConsumeAccountRateLimitResetCreditParams,
    ConsumeAccountRateLimitResetCreditResponse, CreditsSnapshot, GetAccountParams,
    GetAccountRateLimitsResponse, GetAccountResponse, GetAccountTokenUsageResponse,
    GetWorkspaceMessagesResponse, LoginAccountParams, LoginAccountResponse, LogoutAccountResponse,
    PlanType, RateLimitReachedType, RateLimitResetCredit, RateLimitResetCreditStatus,
    RateLimitResetCreditsSummary, RateLimitResetType, RateLimitSnapshot, RateLimitWindow,
    SendAddCreditsNudgeEmailParams, SendAddCreditsNudgeEmailResponse, SpendControlLimitSnapshot,
    WorkspaceMessage, WorkspaceMessageType, FIXTURE_DEMO_DISPLAY_NAME, FIXTURE_DEMO_EMAIL,
    FIXTURE_DEMO_EMAIL_MASKED, FIXTURE_LOGIN_ID, FIXTURE_LOGIN_USER_CODE,
    FIXTURE_LOGIN_VERIFICATION_URL,
};
pub use approvals::{
    build_approval_result, build_approval_rpc_response, build_pending_approval_result,
    is_approval_method, parse_approval_request, ApplyPatchApprovalParams, ApprovalChoice,
    ApprovalKind, CommandExecutionApprovalDecision, CommandExecutionRequestApprovalParams,
    ExecCommandApprovalParams, FileChange, FileChangeApprovalDecision,
    FileChangeRequestApprovalParams, PendingApproval, PermissionsRequestApprovalParams,
    ReviewDecision, ReviewDecisionResponse, APPLY_PATCH_APPROVAL, EXEC_COMMAND_APPROVAL,
    ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL, ITEM_FILE_CHANGE_REQUEST_APPROVAL,
    ITEM_PERMISSIONS_REQUEST_APPROVAL,
};
pub use apps::{
    AppBranding, AppInfo, AppMetadata, AppReview, AppScreenshot, AppToolSummary,
    AppsInstalledParams, AppsInstalledResponse, AppsListParams, AppsListResponse, AppsReadParams,
    AppsReadResponse, ConnectorMetadata, InstalledApp,
};
pub use backend::AgentBackend;
pub use codex::{
    codex_bin_available, resolve_codex_bin, CodexAppServerBackend, CodexAppServerConfig,
};
pub use command::{
    CommandExecOutputDeltaNotification, CommandExecOutputStream, CommandExecParams,
    CommandExecResizeParams, CommandExecResizeResponse, CommandExecResponse,
    CommandExecTerminalSize, CommandExecTerminateParams, CommandExecTerminateResponse,
    CommandExecWriteParams, CommandExecWriteResponse,
};
pub use desktop::{
    BackendCapabilities, BackendKind, BackendSelection, BackendSessionId, DesktopBackend,
};
pub use environment::{
    fixture_added_environment_summary, fixture_demo_collaboration_modes, fixture_demo_environments,
    fixture_environment_info, fixture_environment_status, registered_environment_summary,
    CollaborationModeListParams, CollaborationModeListResponse, CollaborationModeMask,
    EnvironmentAddParams, EnvironmentAddResponse, EnvironmentInfoParams, EnvironmentInfoResponse,
    EnvironmentKind, EnvironmentShellInfo, EnvironmentStatusKind, EnvironmentStatusParams,
    EnvironmentStatusResponse, EnvironmentSummary, ModeKind,
};
pub use experimental_features::{
    ExperimentalFeature, ExperimentalFeatureEnablementSetParams,
    ExperimentalFeatureEnablementSetResponse, ExperimentalFeatureListParams,
    ExperimentalFeatureListResponse, ExperimentalFeatureStage,
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
pub use external_agent_config::{
    ExternalAgentConfigDetectParams, ExternalAgentConfigDetectResponse,
    ExternalAgentConfigImportCompletedNotification, ExternalAgentConfigImportHistoriesReadResponse,
    ExternalAgentConfigImportHistory, ExternalAgentConfigImportHistoryRecordParams,
    ExternalAgentConfigImportHistoryRecordResponse, ExternalAgentConfigImportItemTypeFailure,
    ExternalAgentConfigImportItemTypeSuccess, ExternalAgentConfigImportParams,
    ExternalAgentConfigImportProgressNotification, ExternalAgentConfigImportResponse,
    ExternalAgentConfigImportStatusNotification, ExternalAgentConfigImportTypeResult,
    ExternalAgentConfigMigrationItem, ExternalAgentConfigMigrationItemType,
    ExternalAgentDetectedConnectorCandidate, ExternalAgentDetectedConnectorSource,
    ExternalAgentImportedConnectorCandidate, ExternalAgentImportedConnectorSource,
    CLAUDE_CODE_MIGRATION_SOURCE, CURSOR_MIGRATION_SOURCE,
};
pub use fixture::{
    load_sample_turn_events, replay_events, replay_sample_turn, FixtureBackend, SAMPLE_TURN_JSONL,
};
pub use fs::{
    fixture_fuzzy_search, fixture_get_metadata, fixture_project_tree, fixture_read_directory,
    fixture_read_file, fuzzy_score_name, join_abs, normalize_abs_path, FixtureFsNode,
    FsChangedNotification, FsCopyParams, FsCopyResponse, FsCreateDirectoryParams,
    FsCreateDirectoryResponse, FsGetMetadataParams, FsGetMetadataResponse, FsReadDirectoryEntry,
    FsReadDirectoryParams, FsReadDirectoryResponse, FsReadFileParams, FsReadFileResponse,
    FsRemoveParams, FsRemoveResponse, FsUnwatchParams, FsUnwatchResponse, FsWatchParams,
    FsWatchResponse, FsWriteFileParams, FsWriteFileResponse, FuzzyFileSearchMatchType,
    FuzzyFileSearchParams, FuzzyFileSearchResponse, FuzzyFileSearchResult,
    FuzzyFileSearchSessionStartParams, FuzzyFileSearchSessionStartResponse,
    FuzzyFileSearchSessionStopParams, FuzzyFileSearchSessionStopResponse,
    FuzzyFileSearchSessionUpdateParams, FuzzyFileSearchSessionUpdateResponse, FIXTURE_PROJECT_ROOT,
};
pub use hooks::{
    HookErrorInfo, HookEventName, HookHandlerType, HookMetadata, HookSource, HookTrustStatus,
    HooksListEntry, HooksListParams, HooksListResponse,
};
pub use live_turn::{
    run_live_review_with_bridge, run_live_turn_progressive, run_live_turn_progressive_with_model,
    run_live_turn_with_bridge, run_live_turn_with_bridge_and_model,
    run_live_turn_with_bridge_blocking, run_live_turn_with_bridge_blocking_and_model,
    run_live_turn_with_policy, run_live_turn_with_policy_and_model,
    run_live_turn_with_policy_blocking, LiveApprovalBridge, LiveApprovalPolicy, LiveReviewOutcome,
    LiveTurnOutcome, DEFAULT_LIVE_TURN_TIMEOUT,
};
pub use mcp_auth::{
    McpServerOauthLoginCompleted, McpServerOauthLoginParams, McpServerOauthLoginResponse,
};
pub use mcp_config::{
    valid_mcp_server_name, ConfigBatchWriteParams, ConfigEdit, ConfigMcpServerReloadResponse,
    ConfigValueWriteParams, ConfigWriteResponse, ConfigWriteStatus, McpServerConfigAddParams,
    McpServerTransportConfig, MergeStrategy,
};
pub use memory::{
    MemoryResetResponse, ThreadMemoryMode, ThreadMemoryModeSetParams, ThreadMemoryModeSetResponse,
};
pub use methods::{
    client_method_coverage, client_methods_txt_path, is_known_client_method,
    is_stable_client_method, load_client_methods_from_bar, load_stable_client_methods_from_bar,
    requires_experimental_api, stable_client_methods_txt_path, ClientMethodCoverage,
    CLIENT_METHODS, CLIENT_METHOD_COUNT, EXPERIMENTAL_ONLY_CLIENT_METHOD_COUNT,
    STABLE_CLIENT_METHODS_TEXT, STABLE_CLIENT_METHOD_COUNT, TYPED_CLIENT_METHODS,
    TYPED_CLIENT_METHOD_COUNT,
};
pub use mitsuro::MitsuroServerBackend;
pub use notifications::{
    is_known_server_notification, known_notification_event, server_notification_methods,
    LifecycleNotification, NotificationFamily, NotificationSeverity, SERVER_NOTIFICATIONS_TEXT,
};
pub use permissions::{
    ApprovalsReviewer, ConfigRequirements, ConfigRequirementsReadResponse,
    ModelProviderCapabilitiesReadParams, ModelProviderCapabilitiesReadResponse,
    PermissionProfileListParams, PermissionProfileListResponse, PermissionProfileSummary,
    SandboxMode, FULL_ACCESS_PROFILE_ID, READ_ONLY_PROFILE_ID, WORKSPACE_PROFILE_ID,
};
pub use plugin_mutations::{
    PluginAppSummary, PluginInstallParams, PluginInstallResponse, PluginUninstallParams,
    PluginUninstallResponse,
};
pub use process::{
    decode_base64, decode_base64_lossy, encode_base64, parse_process_exited,
    parse_process_output_delta, ProcessKillParams, ProcessKillResponse, ProcessOutputStream,
    ProcessResizePtyParams, ProcessResizePtyResponse, ProcessSpawnParams, ProcessSpawnResponse,
    ProcessTerminalSize, ProcessWriteStdinParams, ProcessWriteStdinResponse,
    ThreadBackgroundTerminal, ThreadBackgroundTerminalsCleanParams,
    ThreadBackgroundTerminalsCleanResponse, ThreadBackgroundTerminalsListParams,
    ThreadBackgroundTerminalsListResponse, ThreadBackgroundTerminalsTerminateParams,
    ThreadBackgroundTerminalsTerminateResponse,
};
pub use product::{
    conversation_messages_from_thread_value, CodexSessionSettings, ConversationAudio,
    ConversationImage, ConversationMessage, ConversationReference, ConversationReferenceKind,
    CreateSession, MessageRole, ProductAccessMode, ProductAttachment, ProductBackend,
    ProductDirectoryEntry, ProductDstFoldPolicy, ProductDstGapPolicy, ProductDstPolicy,
    ProductExtension, ProductFile, ProductFileMatch, ProductHiveDispatch,
    ProductHiveDispatchRequest, ProductHivePriority, ProductHiveRun, ProductHiveSessionAction,
    ProductHiveSessionDetail, ProductHiveSessionMutationRequest, ProductHiveSnapshot,
    ProductHiveStatus, ProductHiveTask, ProductMcpServer, ProductMisfireConfig,
    ProductMisfirePolicy, ProductModel, ProductModelKey, ProductMonthlyDayPolicy,
    ProductOverlapPolicy, ProductProcess, ProductReasoningEffort, ProductRetryJitter,
    ProductRetryPolicy, ProductReview, ProductReviewStart, ProductReviewTarget, ProductSchedule,
    ProductScheduleAction, ProductScheduleCreateRequest, ProductScheduleDefinition,
    ProductScheduleMutation, ProductScheduleMutationRequest, ProductScheduleRecurrence,
    ProductScheduleReplaceRequest, ProductScheduleWeekday, ProductSkill, ProductSpeedMode,
    ProductSpeedOption, ProductSteer, ProductTurn, ProductWorkMode, SessionConversation,
    SessionHistoryPage, SessionHistoryState, SessionOpenMode, SessionSummary,
};
pub use protocol::{
    activity_item_fields, command_execution_fields, extract_chat_tail_from_thread,
    extract_transcript_from_thread, file_change_fields, fixture_demo_config, fixture_demo_models,
    fixture_demo_skills, map_notification_to_event, map_server_request_to_event,
    parse_fixture_jsonl, parse_notification_line, summarize_file_changes, user_input_text_value,
    ActivityFields, ClientInfo, CollaborationMode, CollaborationModeSettings,
    CommandExecutionFields, ConfigReadParams, ConfigReadResponse, FileChangeFields,
    InitializeCapabilities, InitializeParams, InitializeResponse, JsonRpcError, JsonRpcId,
    JsonRpcMessage, ModelInfo, ModelListParams, ModelListResponse, ModelServiceTier, NetworkAccess,
    Notification, ReasoningEffortOption, ReviewDelivery, ReviewStartParams, ReviewStartResponse,
    ReviewTarget, SandboxPolicy, SkillMetadata, SkillsListEntry, SkillsListParams,
    SkillsListResponse, ThreadArchiveParams, ThreadArchiveResponse, ThreadCompactStartParams,
    ThreadCompactStartResponse, ThreadDeleteParams, ThreadDeleteResponse, ThreadForkParams,
    ThreadForkResponse, ThreadGoal, ThreadGoalClearParams, ThreadGoalClearResponse,
    ThreadGoalGetParams, ThreadGoalGetResponse, ThreadGoalSetParams, ThreadGoalSetResponse,
    ThreadGoalStatus, ThreadInjectItemsParams, ThreadInjectItemsResponse, ThreadListParams,
    ThreadListResponse, ThreadReadParams, ThreadReadResponse, ThreadResumeInitialTurnsPageParams,
    ThreadResumeParams, ThreadResumeResponse, ThreadSearchParams, ThreadSearchResponse,
    ThreadSearchResult, ThreadSetNameParams, ThreadSetNameResponse, ThreadShellCommandParams,
    ThreadShellCommandResponse, ThreadStartParams, ThreadStartResponse, ThreadSummary,
    ThreadUnarchiveParams, ThreadUnarchiveResponse, ThreadUnsubscribeParams,
    ThreadUnsubscribeResponse, ThreadUnsubscribeStatus, TranscriptAudio, TranscriptAudioSource,
    TranscriptImage, TranscriptImageSource, TranscriptMessage, TranscriptReference,
    TranscriptReferenceKind, TranscriptRole, TurnInterruptParams, TurnInterruptResponse,
    TurnStartParams, TurnStartResponse, TurnSteerParams, TurnSteerResponse,
};
pub use realtime::{
    CodexResponseHandoffMode, ConversationTextRole, RealtimeConversationVersion, RealtimeEvent,
    RealtimeOutputModality, RealtimeVoice, RealtimeVoicesList, ThreadRealtimeAppendAudioParams,
    ThreadRealtimeAppendAudioResponse, ThreadRealtimeAppendSpeechParams,
    ThreadRealtimeAppendSpeechResponse, ThreadRealtimeAppendTextParams,
    ThreadRealtimeAppendTextResponse, ThreadRealtimeAudioChunk, ThreadRealtimeInitialItem,
    ThreadRealtimeListVoicesParams, ThreadRealtimeListVoicesResponse, ThreadRealtimeStartParams,
    ThreadRealtimeStartResponse, ThreadRealtimeStartTransport, ThreadRealtimeStopParams,
    ThreadRealtimeStopResponse,
};
pub use remote_control::{
    RemoteControlClient, RemoteControlClientsListOrder, RemoteControlClientsListParams,
    RemoteControlClientsListResponse, RemoteControlClientsRevokeParams,
    RemoteControlClientsRevokeResponse, RemoteControlConnectionStatus, RemoteControlDisableParams,
    RemoteControlDisableResponse, RemoteControlEnableParams, RemoteControlEnableResponse,
    RemoteControlPairingStartParams, RemoteControlPairingStartResponse,
    RemoteControlPairingStatusParams, RemoteControlPairingStatusResponse,
    RemoteControlStatusChangedNotification, RemoteControlStatusReadResponse,
};
pub use server_requests::{
    automatic_server_response, is_known_server_request, parse_mcp_elicitation_request,
    parse_user_input_request, AutomaticServerResponse, McpElicitationMode, PendingMcpElicitation,
    PendingUserInput, ToolRequestUserInputParams, UserInputOption, UserInputQuestion,
    ATTESTATION_GENERATE, CHATGPT_AUTH_TOKENS_REFRESH, CURRENT_TIME_READ, DYNAMIC_TOOL_CALL,
    MCP_SERVER_ELICITATION_REQUEST, SERVER_REQUEST_METHODS, TOOL_REQUEST_USER_INPUT,
};
pub use skill_config::{SkillsConfigWriteParams, SkillsConfigWriteResponse};
pub use thread_configuration::{
    ActivePermissionProfile, ApprovalPolicyMode, AskForApproval, GranularApprovalPolicy,
    ThreadMetadataGitInfoUpdateParams, ThreadMetadataUpdateParams, ThreadMetadataUpdateResponse,
    ThreadMultiAgentMode, ThreadMultiAgentModeName, ThreadPersonality, ThreadReasoningSummary,
    ThreadSettings, ThreadSettingsUpdateParams, ThreadSettingsUpdateResponse,
    ThreadSettingsUpdatedNotification,
};
pub use thread_history::{
    list_items_in_thread, list_turns_in_thread, search_occurrences_in_thread, ThreadItemEntry,
    ThreadItemsListParams, ThreadItemsListResponse, ThreadItemsSortDirection, ThreadRollbackParams,
    ThreadRollbackResponse, ThreadSearchOccurrence, ThreadSearchOccurrencesParams,
    ThreadSearchOccurrencesResponse, ThreadSearchTextRange, ThreadTurnItemsView,
    ThreadTurnsListParams, ThreadTurnsListResponse, ThreadTurnsSortDirection,
};
pub use types::{
    AgentError, ConnectionStatus, DelegatedProgressProjection, DelegationExecution,
    DelegationGroupProjection, DelegationGroupStatus, DelegationKind,
    DelegationParentContinuationStatus, DelegationRole, DelegationRunStage,
    DelegationTaskProjection, DelegationTaskStatus, DurableDelegationEvent,
    DurableDelegationEventKind, ItemKind, Result, SessionDelegationProjection, TurnStreamEvent,
};
