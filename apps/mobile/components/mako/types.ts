import type {
  ChatMessage,
  MakoChannelsResponse,
  MakoCrewDocumentKind,
  MakoCrewResponse,
  MakoCurrentRunSummary,
  MakoCurrentResponse,
  MakoHomeDocumentKind,
  MakoHomeResponse,
  MakoRunPriority,
  MakoRunWakeEvent,
  MakoSessionStatus,
  ModelInfo,
  PermissionMode,
  ThinkingLevel,
} from "@krusty/api";
import type { Attachment as ChatBarAttachment } from "../chat/ChatBar";

export type MakoTopLevelView =
  | "mako"
  | "schedule"
  | "logbook"
  | "runs"
  | "details"
  | "crew"
  | "channels";
export type MakoRunSection = "overview" | "wake" | "tasks" | "chat" | "artifacts";
export type MakoKnowledgeView = "recent" | "memory";
export type MakoKnowledgeScope = "workspace" | "all";
export type { MakoCurrentRunSummary };

export interface MakoChatContext {
  sessionId: string | null;
  title: string | null;
  messages: ChatMessage[];
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
  models: ModelInfo[];
  researchEnabled: boolean;
  tokenCount: number;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
  onSubmitToolResult: (
    toolCallId: string,
    result: string,
  ) => void | Promise<void>;
  onPlanConfirm: (
    toolCallId: string,
    choice: "execute" | "abandon",
  ) => void | Promise<void>;
  onSend: (content: string, attachments?: ChatBarAttachment[]) => Promise<void>;
  onStop: () => void;
  onThinkingChange: (level: ThinkingLevel) => void;
  onPermissionModeToggle: () => void;
  onFastModeToggle: () => void;
  onModeToggle: () => void;
  onModelSelect: (modelId: string) => void;
  onResearchToggle: () => void;
}

export interface MakoCurrentState {
  current: MakoCurrentResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  isRecovering: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  setCourse: (task: string, options?: {
    projectDir?: string | null;
    model?: string | null;
    startAt?: string | null;
    priority?: MakoRunPriority | null;
    crewSlug?: string | null;
  }) => Promise<string | null>;
  recoverDaemon: () => Promise<number>;
  isDispatching: boolean;
}

export interface MakoHomeState {
  home: MakoHomeResponse | null;
  crew: MakoCrewResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  isBootstrapping: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  bootstrap: () => Promise<void>;
  updateHomeDocument: (
    kind: MakoHomeDocumentKind,
    content: string,
  ) => Promise<void>;
  updateCrewDocument: (
    slug: string,
    kind: MakoCrewDocumentKind,
    content: string,
  ) => Promise<void>;
}

export interface MakoChannelsState {
  channels: MakoChannelsResponse | null;
  isLoading: boolean;
  isRefreshing: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export interface MakoRunState {
  status: MakoSessionStatus | null;
  wake: MakoRunWakeEvent[];
  isLoading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export interface MakoSelectedRun {
  runId: string;
  summary: MakoCurrentRunSummary | null;
}
