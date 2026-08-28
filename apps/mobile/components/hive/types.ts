import type {
  HiveChannelsResponse,
  HiveCrewDocumentKind,
  HiveCrewResponse,
  HiveCurrentResponse,
  HiveCurrentRunSummary,
  HiveHomeDocumentKind,
  HiveHomeResponse,
  HiveRunPriority,
  HiveRunWakeEvent,
  HiveSessionStatus,
  ModelInfo,
  ModelKey,
  PermissionMode,
  ThinkingLevel,
} from "@mitsuro/api";
import type { Attachment as ChatBarAttachment } from "../chat/ChatBar";

export type HiveTopLevelView =
  | "hive"
  | "attention"
  | "schedule"
  | "logbook"
  | "runs"
  | "details"
  | "crew"
  | "groups"
  | "channels"
  | "memory";
export type HiveRunSection =
  | "overview"
  | "wake"
  | "tasks"
  | "chat"
  | "artifacts";
export type HiveKnowledgeView = "recent" | "memory";
export type HiveKnowledgeScope = "workspace" | "all";
export type HiveAttentionItemKind =
  | "approval_required"
  | "input_required"
  | "run_completed"
  | "run_failed"
  | "run_stalled"
  | "scheduled_run_started"
  | "scheduled_run_completed"
  | "delegated_task_completed";
export type HiveAttentionSection = "needs_action" | "updates";
export type { HiveCurrentRunSummary };

export interface HiveChatContext {
  sessionId: string | null;
  title: string | null;
  error: string | null;
  isLoading: boolean;
  isStreaming: boolean;
  isThinking: boolean;
  activeToolCallId: string | null;
  thinkingLevel: ThinkingLevel;
  permissionMode: PermissionMode;
  fastModeEnabled: boolean;
  fastModeSupported: boolean;
  mode: "build" | "plan";
  model: string | null;
  modelKey: ModelKey | null;
  models: ModelInfo[];
  tokenCount: number;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
  onSubmitToolResult: (
    sessionId: string,
    toolCallId: string,
    result: string,
  ) => void | Promise<void>;
  onPlanConfirm: (
    sessionId: string,
    toolCallId: string,
    choice: "execute" | "abandon",
  ) => void | Promise<void>;
  onSend: (content: string, attachments?: ChatBarAttachment[]) => Promise<void>;
  onWorkerSend: (sessionId: string, content: string) => Promise<void>;
  onWorkerStop: (sessionId: string) => void;
  onStop: () => void;
  onThinkingChange: (level: ThinkingLevel) => void;
  onPermissionModeToggle: () => void;
  onFastModeToggle: () => void;
  onModeToggle: () => void;
  onModelSelect: (model: ModelInfo) => void;
}

export interface HiveCurrentState {
  current: HiveCurrentResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  isRecovering: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  setCourse: (task: string, options?: {
    projectDir?: string | null;
    model?: string | null;
    modelKey?: ModelKey | null;
    startAt?: string | null;
    priority?: HiveRunPriority | null;
    crewSlug?: string | null;
  }) => Promise<string | null>;
  recoverDaemon: () => Promise<number>;
  isDispatching: boolean;
}

export interface HiveHomeState {
  home: HiveHomeResponse | null;
  crew: HiveCrewResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  isBootstrapping: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  bootstrap: () => Promise<void>;
  updateHomeDocument: (
    kind: HiveHomeDocumentKind,
    content: string,
  ) => Promise<void>;
  updateCrewDocument: (
    slug: string,
    kind: HiveCrewDocumentKind,
    content: string,
  ) => Promise<void>;
}

export interface HiveChannelsState {
  channels: HiveChannelsResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export interface HiveRunState {
  status: HiveSessionStatus | null;
  wake: HiveRunWakeEvent[];
  isLoading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export interface HiveSelectedRun {
  runId: string;
  summary: HiveCurrentRunSummary | null;
}

export interface HiveAttentionItem {
  id: string;
  kind: HiveAttentionItemKind;
  section: HiveAttentionSection;
  title: string;
  summary: string;
  detail: string;
  createdAt: string;
  read: boolean;
  active: boolean;
  runId?: string | null;
  projectDir?: string | null;
  targetBranch?: string | null;
  toolCallId?: string | null;
  sessionId?: string | null;
  threadSessionId?: string | null;
  threadMessageId?: string | null;
}
