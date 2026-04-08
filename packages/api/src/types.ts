// ============================================================================
// Session Types
// ============================================================================

export interface SessionResponse {
  id: string;
  title: string;
  token_count?: number | null;
  working_dir: string | null;
  project_dir: string | null;
  workspace_mode: WorkspaceMode;
  session_type: SessionType;
  parent_session_id: string | null;
  mode: SessionMode;
  updated_at: string;
  model?: string | null;
  target_branch?: string | null;
}

export interface SessionWithMessagesResponse {
  session: SessionResponse;
  messages: MessageResponse[];
}

export interface RecoveryToolCall {
  id: string;
  name: string;
}

export interface SessionRecoveryState {
  schema_version: number;
  status: string;
  stop_reason: string | null;
  last_error: string | null;
  partial_assistant: PartialAssistantState;
  decision: Record<string, unknown>;
}

export interface PartialAssistantState {
  text: string;
  thinking?: string;
  tool_calls: RecoveryToolCall[];
}

export interface SessionStateResponse {
  id: string;
  agent_state: string;
  started_at: string | null;
  last_event_at: string | null;
  mode: SessionMode;
  recovery?: SessionRecoveryState | null;
  live_partial_assistant?: PartialAssistantState | null;
  delegated_tools?: DelegatedToolStateResponse[];
  recent_delegated_runs?: DelegatedRunResponse[];
  last_event_sequence?: number | null;
}

// ============================================================================
// Message Types
// ============================================================================

export interface MessageContent {
  type: 'text' | 'tool_use' | 'tool_result' | 'thinking';
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  tool_use_id?: string;
  content?: string;
  thinking?: string;
}

export interface MessageResponse {
  role: 'user' | 'assistant';
  content: MessageContent[];
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  thinking?: string;
  toolCalls?: ToolCall[];
  isQueued?: boolean;
  kind?: 'recovery_notice' | 'live_partial' | 'streaming';
}

export interface ToolCall {
  id: string;
  name: string;
  description?: string;
  arguments?: Record<string, unknown>;
  output?: string;
  delegatedRunId?: string;
  delegated?: DelegatedArtifactState;
  status: 'pending' | 'running' | 'success' | 'partial' | 'error' | 'awaiting_approval';
}

// ============================================================================
// Chat Request
// ============================================================================

export interface ChatRequest {
  session_id?: string;
  message: string;
  content?: ContentBlock[];
  project_dir?: string | null;
  working_dir?: string | null;
  workspace_mode?: WorkspaceMode;
  session_type?: SessionType;
  research_enabled?: boolean;
  model?: string;
  thinking_enabled?: boolean | string;
  permission_mode?: PermissionMode;
  mode?: SessionMode;
}

export type ContentBlock = TextContent | ImageContent;

export interface TextContent {
  type: 'text';
  text: string;
}

export interface ImageContent {
  type: 'image';
  source: {
    type: 'base64';
    media_type: string;
    data: string;
  };
}

// ============================================================================
// Delegation Types
// ============================================================================

export type DelegatedToolKind = 'explore' | 'build';
export type DelegatedProgressStatus = 'running' | 'complete' | 'failed';
export type DelegatedRunStage =
  | 'created' | 'running' | 'synthesizing' | 'complete'
  | 'degraded' | 'failed' | 'cancelled';

export interface DelegatedProgressEvent {
  delegated_run_id: string;
  tool_call_id: string;
  kind: DelegatedToolKind;
  stage: DelegatedRunStage;
  parent_session_id: string;
  task_id: string;
  agent_name: string;
  status: DelegatedProgressStatus;
  tool_count: number;
  tokens: number;
  current_action?: string | null;
  completion_summary?: string | null;
  lines_added: number;
  lines_removed: number;
  completed_plan_task?: string | null;
}

export interface DelegatedAgentState {
  taskId: string;
  name: string;
  status: 'pending' | 'running' | 'complete' | 'failed';
  outcomeReason?: string;
  toolCount: number;
  tokens: number;
  currentAction?: string;
  completionSummary?: string;
  linesAdded: number;
  linesRemoved: number;
  completedPlanTask?: string;
}

export interface DelegatedArtifactState {
  kind: DelegatedToolKind;
  delegatedRunId?: string;
  stage?: DelegatedRunStage;
  thinking?: string;
  message?: string;
  investigationSummary?: string;
  humanReview?: string;
  outcome?: 'success' | 'partial' | 'failed';
  confidence?: 'high' | 'medium' | 'low';
  structuralCoverage?: 'high' | 'medium' | 'low';
  semanticCoverage?: 'high' | 'medium' | 'low';
  agents: DelegatedAgentState[];
  filesExamined: string[];
  errors: string[];
  agentCount?: number;
  usableAgents?: number;
  degradedAgents?: number;
  successfulAgents?: number;
  failedAgents?: number;
  filesExaminedCount?: number;
  outcomeReason?: string;
  totalTurns?: number;
  totalDurationMs?: number;
  coverageGapNotice?: string;
  linesAdded?: number;
  linesRemoved?: number;
  filesModified?: number;
  lockContentions?: number;
  totalLockWaitMs?: number;
  totalTargets?: number;
  activeTargets?: number;
  completedTargets?: number;
  pendingTargets?: number;
}

export interface DelegatedAgentStateResponse {
  task_id: string;
  agent_name: string;
  status: DelegatedProgressStatus;
  tool_count: number;
  tokens: number;
  current_action?: string | null;
  completion_summary?: string | null;
  lines_added: number;
  lines_removed: number;
  completed_plan_task?: string | null;
}

export interface DelegatedToolStateResponse {
  delegated_run_id: string;
  tool_call_id: string;
  kind: DelegatedToolKind;
  stage: DelegatedRunStage;
  parent_session_id?: string | null;
  agents: DelegatedAgentStateResponse[];
}

export interface DelegatedRunScopeResponse {
  label: string;
  path: string;
  kind: string;
}

export interface DelegatedRunResponse {
  delegated_run_id: string;
  parent_tool_call_id?: string | null;
  kind: DelegatedToolKind;
  stage: DelegatedRunStage;
  provider?: string | null;
  model?: string | null;
  resumable: boolean;
  resumed_from_run_id?: string | null;
  target_scope: DelegatedRunScopeResponse[];
  human_review?: string | null;
  artifact?: Record<string, unknown> | null;
  updated_at: string;
}

// ============================================================================
// Mako Types
// ============================================================================

export type MakoRuntimeStatus = 'idle' | 'running' | 'sleeping' | 'awaiting_input' | 'paused' | 'error' | 'cancelled';
export type MakoHomeStatus = 'awake' | 'sleeping' | 'paused' | 'blocked' | 'idle';
export type MakoRunPriority = 'low' | 'normal' | 'high';
export type AutonomousTaskStatus = 'pending' | 'in_progress' | 'completed' | 'failed';

export interface MakoDispatchResponse {
  session_id: string;
  status: string;
}

export interface SimpleOkResponse {
  ok: boolean;
}

export interface ApnsRegisterResponse {
  id: string;
}

export interface ApnsStatusResponse {
  apns_configured: boolean;
  device_count: number;
}

export type MemoryType = 'user' | 'feedback' | 'project' | 'reference';

export interface AgentMemory {
  id: string;
  memory_type: MemoryType;
  title: string;
  content: string;
  project_dir?: string;
  created_at: string;
  updated_at: string;
}

export interface PromoteReportToMemoryResponse {
  created: boolean;
  memory: AgentMemory;
}

export interface MemorySnapshotResponse {
  snapshot: AgentMemory | null;
}

export interface AutonomousTask {
  id: string;
  session_id: string;
  subject: string;
  description: string;
  status: AutonomousTaskStatus;
  owner?: string | null;
  blocked_by: string[];
  created_at: string;
  updated_at: string;
  completed_at?: string | null;
  result?: string | null;
}

export interface MakoRuntimeState {
  session_id: string;
  status: MakoRuntimeStatus;
  next_wake_at?: string | null;
  sleep_reason?: string | null;
  last_error?: string | null;
  current_run_id?: string | null;
  last_wake_reason?: string | null;
  priority: MakoRunPriority;
  updated_at: string;
}

export interface MakoSessionSummary {
  session_id: string;
  title: string;
  updated_at: string;
  project_dir?: string | null;
  agent_state: string;
  runtime?: MakoRuntimeState | null;
}

export interface MakoSessionStatus {
  session_id: string;
  session_type: SessionType;
  title: string;
  tasks: AutonomousTask[];
  agent_state: string;
  runtime?: MakoRuntimeState | null;
  cadence: MakoCadenceSummary;
}

export interface MakoCurrentRunSummary {
  session_id: string;
  title: string;
  updated_at: string;
  project_dir?: string | null;
  agent_state: string;
  runtime?: MakoRuntimeState | null;
  pending_tasks: number;
  in_progress_tasks: number;
  completed_tasks: number;
  failed_tasks: number;
  blocked_tasks: number;
  cadence: MakoCadenceSummary;
}

export interface MakoPendingApproval {
  session_id: string;
  session_title: string;
  project_dir?: string | null;
  tool_call_id: string;
  tool_name: string;
  arguments: unknown;
  requested_at: string;
  priority: MakoRunPriority;
}

export interface MakoStatusSummary {
  home_status: MakoHomeStatus;
  total_count: number;
  running_count: number;
  sleeping_count: number;
  scheduled_count: number;
  high_priority_count: number;
  paused_count: number;
  waiting_count: number;
  failed_count: number;
  idle_count: number;
  pending_approvals_count: number;
  next_wake_at?: string | null;
}

export interface MakoCadenceSummary {
  tick_interval_secs: number;
  max_ticks: number;
}

export interface MakoCurrentResponse {
  status: MakoStatusSummary;
  runs: MakoCurrentRunSummary[];
  approvals: MakoPendingApproval[];
}

export interface MakoRunWakeEvent {
  id: string;
  timestamp: string;
  title: string;
  detail?: string | null;
  kind: 'runtime' | 'task';
  status: string;
}

// ============================================================================
// Stream Types
// ============================================================================

export interface PlanItem {
  id: string;
  content: string;
  completed: boolean;
}

export type StreamEvent =
  | { type: 'text_delta'; delta: string }
  | { type: 'text_delta_with_citations'; delta: string; citations: unknown[] }
  | { type: 'thinking_delta'; thinking: string }
  | { type: 'thinking_complete'; thinking: string; signature: string }
  | { type: 'tool_call_start'; id: string; name: string }
  | { type: 'tool_call_complete'; id: string; name: string; arguments: Record<string, unknown> }
  | { type: 'tool_executing'; id: string; name: string }
  | { type: 'tool_output_delta'; id: string; delta: string }
  | ({ type: 'delegated_progress' } & DelegatedProgressEvent)
  | { type: 'tool_result'; id: string; output: string; is_error: boolean }
  | { type: 'server_tool_start'; id: string; name: string }
  | { type: 'server_tool_complete'; id: string; name: string }
  | { type: 'web_search_results'; tool_use_id: string; results: unknown[] }
  | { type: 'web_fetch_result'; tool_use_id: string; content: unknown }
  | { type: 'server_tool_error'; tool_use_id: string; error_code: string }
  | { type: 'plan_update'; items: PlanItem[] }
  | { type: 'mode_change'; mode: string; reason?: string }
  | { type: 'plan_complete'; tool_call_id: string; title: string; task_count: number }
  | { type: 'usage'; prompt_tokens: number; completion_tokens: number }
  | { type: 'context_compacted'; reason: string; estimated_tokens_before: number; estimated_tokens_after: number; replaced_messages: number }
  | { type: 'lagged'; skipped: number }
  | { type: 'title_update'; title: string }
  | { type: 'tool_approval_required'; id: string; name: string; arguments: Record<string, unknown> }
  | { type: 'tool_approved'; id: string }
  | { type: 'tool_denied'; id: string }
  | { type: 'turn_complete'; turn: number; has_more: boolean }
  | { type: 'finish'; session_id: string; stop_reason: string }
  | { type: 'error'; error: string }
  // Mako autonomous agent events
  | { type: 'user_message'; title?: string; message: string; level: string }
  | { type: 'agent_sleeping'; duration_secs: number; reason: string }
  | { type: 'tick_injected'; tick_number: number }
  | { type: 'classifier_decision'; tool_name: string; decision: string; reason: string; stage: number }
  | { type: 'teammate_spawned'; name: string; role: string }
  | { type: 'teammate_task_completed'; name: string; task_id: string; result: string }
  | { type: 'teammate_task_failed'; name: string; task_id: string; error: string }
  | { type: 'teammate_cancelled'; name: string };

export interface StreamCallbacks {
  onTextDelta: (delta: string) => void;
  onThinkingDelta: (thinking: string) => void;
  onToolCallStart: (id: string, name: string) => void;
  onToolCallComplete: (id: string, name: string, args: Record<string, unknown>) => void;
  onToolResult: (id: string, output: string, isError: boolean) => void;
  onToolOutputDelta: (id: string, delta: string) => void;
  onDelegatedProgress?: (event: DelegatedProgressEvent) => void;
  onToolApprovalRequired?: (id: string, name: string, args: Record<string, unknown>) => void;
  onToolApproved?: (id: string) => void;
  onToolDenied?: (id: string) => void;
  onTurnComplete?: (turn: number, hasMore: boolean) => void;
  onPlanUpdate: (items: PlanItem[]) => void;
  onModeChange: (mode: string, reason?: string) => void;
  onPlanComplete: (toolCallId: string, title: string, taskCount: number) => void;
  onUsage: (promptTokens: number, completionTokens: number) => void;
  onTitleUpdate: (title: string) => void;
  onFinish: (sessionId: string) => void;
  onError: (error: string) => void;
  // Mako autonomous agent callbacks
  onUserMessage?: (title: string | undefined, message: string, level: string) => void;
  onAgentSleeping?: (durationSecs: number, reason: string) => void;
  onTickInjected?: (tickNumber: number) => void;
  onClassifierDecision?: (toolName: string, decision: string, reason: string, stage: number) => void;
  onTeammateSpawned?: (name: string, role: string) => void;
  onTeammateTaskCompleted?: (name: string, taskId: string, result: string) => void;
  onTeammateTaskFailed?: (name: string, taskId: string, error: string) => void;
  onTeammateCancelled?: (name: string) => void;
}

// ============================================================================
// Git Types
// ============================================================================

export interface GitStatusResponse {
  in_repo: boolean;
  repo_root: string | null;
  branch: string | null;
  head: string | null;
  upstream: string | null;
  branch_files: number;
  branch_additions: number;
  branch_deletions: number;
  pr_number: number | null;
  ahead: number;
  behind: number;
  staged: number;
  modified: number;
  untracked: number;
  conflicted: number;
  total_changes: number;
}

export interface GitBranch {
  name: string;
  is_current: boolean;
  upstream: string | null;
  is_remote: boolean;
}

export interface GitBranchesResponse {
  repo_root: string;
  branches: GitBranch[];
}

export interface GitWorktree {
  path: string;
  branch: string | null;
  head: string | null;
  is_current: boolean;
}

export interface GitWorktreesResponse {
  repo_root: string;
  worktrees: GitWorktree[];
}

// ============================================================================
// Auth & Provider Types
// ============================================================================

export interface ProviderStatus {
  id: string;
  name: string;
  configured: boolean;
  has_oauth: boolean;
  supports_oauth: boolean;
}

export interface OAuthDeviceCodeInfo {
  user_code: string;
  verification_uri: string;
  verification_uri_complete?: string | null;
  expires_in: number;
}

export interface OAuthStartResponse {
  auth_url: string;
  provider: string;
  flow_type: 'browser_callback' | 'device' | 'paste_code';
  paste_code: boolean;
  device_code?: OAuthDeviceCodeInfo | null;
}

export interface OAuthStatusResponse {
  has_token: boolean;
  flow_active: boolean;
}

export interface OAuthExchangeResponse {
  success: boolean;
}

export interface TailscaleAccessResponse {
  status: string;
  url?: string | null;
  detail?: string | null;
}

export interface ServerAccessResponse {
  local_url: string;
  remote_access_enabled: boolean;
  remote_access_token: string;
  remote_launch_url?: string | null;
  tailscale: TailscaleAccessResponse;
}

// ============================================================================
// Presence Types
// ============================================================================

export type PresenceCapability = 'observer' | 'controller';

export interface SessionPresenceClientResponse {
  client_id: string;
  surface: string;
  capability: PresenceCapability;
  user_id?: string | null;
  last_seen_at: string;
  last_event_sequence?: number | null;
  stale: boolean;
}

export interface SessionPresenceResponse {
  session_id: string;
  active_viewers: number;
  active_controllers: number;
  stale_clients: number;
  clients: SessionPresenceClientResponse[];
}

// ============================================================================
// Model Types
// ============================================================================

export interface ModelInfo {
  id: string;
  display_name: string;
  provider: string;
  context_window: number;
  max_output: number;
  supports_thinking: boolean;
  supports_tools: boolean;
}

export interface ModelsResponse {
  models: ModelInfo[];
  default_model: string;
}

// ============================================================================
// File Types
// ============================================================================

export interface TreeEntry {
  name: string;
  path: string;
  is_dir: boolean;
  children?: TreeEntry[];
}

// ============================================================================
// Preview / MCP / Skills Types
// ============================================================================

export interface PreviewSettings {
  enabled: boolean;
  auto_refresh_secs: number;
  show_only_http_like: boolean;
  probe_timeout_ms: number;
  allow_force_open_non_http: boolean;
  pinned_ports: number[];
  hidden_ports: number[];
  blocked_ports: number[];
}

export interface PreviewSettingsPatch {
  enabled?: boolean;
  auto_refresh_secs?: number;
  show_only_http_like?: boolean;
  probe_timeout_ms?: number;
  allow_force_open_non_http?: boolean;
  pinned_ports?: number[];
  hidden_ports?: number[];
  blocked_ports?: number[];
}

export type PortProbeStatus = 'ok' | 'timeout' | 'conn_refused' | 'non_http' | 'error';

export interface PortEntry {
  port: number;
  name: string;
  description: string | null;
  command: string | null;
  pid: number | null;
  source: string;
  active: boolean;
  pinned: boolean;
  is_http_like: boolean;
  is_previewable_http: boolean;
  probe_status: PortProbeStatus;
  last_probe_ms: number | null;
  preview_path: string;
}

export interface PortListResponse {
  ports: PortEntry[];
  settings: PreviewSettings;
  discovery_error?: string | null;
}

export interface McpToolResponse {
  name: string;
  description?: string | null;
}

export interface McpServerResponse {
  name: string;
  server_type: string;
  status: string;
  connected: boolean;
  tool_count: number;
  tools: McpToolResponse[];
  error?: string | null;
}

export type SkillSource = 'global' | 'project';

export interface SkillInfo {
  name: string;
  description: string;
  version?: string | null;
  author?: string | null;
  tags: string[];
  source: SkillSource;
}

// ============================================================================
// Common Types
// ============================================================================

export type SessionMode = 'build' | 'plan';
export type SessionType = 'chat' | 'code' | 'mako';
export type PermissionMode = 'supervised' | 'autonomous';
export type WorkspaceMode = 'neutral' | 'selected' | 'created';
export type ThinkingLevel = 'off' | 'low' | 'medium' | 'high' | 'xhigh';

// ============================================================================
// Report Types
// ============================================================================

export interface ReportSummary {
  id: string;
  title: string;
  summary: string;
  tags: string[];
  created_at: string;
  project_dir?: string;
}

export interface Report extends ReportSummary {
  content: string;
  sources: string[];
  session_id: string;
}
