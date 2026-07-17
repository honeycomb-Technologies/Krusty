import type {
  DelegatedRunStage,
  DelegatedToolKind,
  SessionType,
  UsageMetrics,
  WorkspaceMode,
} from '@krusty/api';

export interface ToolCall {
  id: string;
  name: string;
  description?: string;
  arguments?: Record<string, unknown>;
  output?: string;
  delegatedRunId?: string;
  delegated?: DelegatedArtifactState;
  status:
    | 'pending'
    | 'running'
    | 'success'
    | 'partial'
    | 'error'
    | 'awaiting_approval';
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

export interface ChatMessageAttachment {
  type: 'image' | 'file';
  name?: string;
  mimeType?: string;
  uri?: string;
  base64?: string;
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  thinking?: string;
  attachments?: ChatMessageAttachment[];
  toolCalls?: ToolCall[];
  renderParts?: ChatRenderPart[];
  isQueued?: boolean;
  queuedUntilNextRun?: boolean;
  kind?: 'recovery_notice' | 'live_partial' | 'streaming';
}

export type ChatRenderPart =
  | {
      type: 'text';
      id: string;
      content: string;
    }
  | {
      type: 'thinking';
      id: string;
      content: string;
    }
  | {
      type: 'tool';
      id: string;
      toolCallId: string;
    }
  | {
      type: 'attachments';
      id: string;
    };

export type SessionMode = 'build' | 'plan';
export type PermissionMode = 'supervised' | 'autonomous';
export type ThinkingLevel = 'off' | 'low' | 'medium' | 'high' | 'xhigh';

export interface Attachment {
  name: string;
  type: 'image' | 'file';
  mimeType: string;
  uri?: string;
  base64?: string;
  text?: string;
}

export interface SendMessageOptions {
  projectDir?: string | null;
  workingDir?: string | null;
  workspaceMode?: WorkspaceMode;
  sessionType?: SessionType;
  targetBranch?: string | null;
}

export interface QueuedMessage {
  content: string;
  attachments: Attachment[];
  researchEnabled: boolean;
  sendOptions?: SendMessageOptions;
}

export interface SessionStoreState {
  sessionId: string | null;
  title: string;
  mode: SessionMode;
  permissionMode: PermissionMode;
  messages: ChatMessage[];
  queuedMessages: QueuedMessage[];
  isLoading: boolean;
  isStreaming: boolean;
  isThinking: boolean;
  thinkingContent: string;
  thinkingEnabled: boolean;
  thinkingLevel: ThinkingLevel;
  fastModeEnabled: boolean;
  tokenCount: number;
  /** Last live usage snapshot, retaining uncached/cache/output buckets. */
  tokenUsage: UsageMetrics | null;
  lastEventSequence: number | null;
  error: string | null;
  model: string | null;
  modelProvider: string | null;

  sendMessage: (
    content: string,
    attachments?: Attachment[],
    researchEnabled?: boolean,
    sendOptions?: SendMessageOptions,
  ) => Promise<void>;
  loadSession: (sessionId: string, isRefresh?: boolean) => Promise<void>;
  clearSession: () => void;
  initSession: (sessionId: string, title: string, permissionMode?: PermissionMode) => void;
  setTitle: (title: string) => void;
  updateTitle: (sessionId: string, title: string) => Promise<void>;
  setMode: (mode: SessionMode) => void;
  setModel: (model: string | null, provider?: string | null) => void;
  setThinkingLevel: (level: ThinkingLevel) => void;
  toggleThinking: () => void;
  setFastModeEnabled: (enabled: boolean) => void;
  toggleFastMode: () => void;
  togglePermissionMode: () => void;
  submitToolResult: (toolCallId: string, result: string) => Promise<void>;
  submitToolApproval: (toolCallId: string, approved: boolean) => Promise<void>;
  stopStreaming: () => void;
  startStatePolling: (sessionId: string) => void;
  stopStatePolling: () => void;
  startPresenceHeartbeat: (sessionId: string) => void;
  stopPresenceHeartbeat: (sessionId?: string | null) => void;
  cleanup: () => void;
}

export interface AssistantMessageRef {
  current: ChatMessage;
}
