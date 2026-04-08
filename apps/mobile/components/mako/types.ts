import type {
  ChatMessage,
  MakoCurrentRunSummary,
  MakoCurrentResponse,
  MakoRunPriority,
  MakoRunWakeEvent,
  MakoSessionStatus,
  ModelInfo,
  PermissionMode,
  ThinkingLevel,
} from "@krusty/api";
import type { Attachment as ChatBarAttachment } from "../chat/ChatBar";

export type MakoTopLevelView = "current" | "chat" | "runs" | "reports" | "status";
export type MakoRunSection = "overview" | "wake" | "tasks" | "chat" | "artifacts";
export type MakoKnowledgeView = "reports" | "memory";
export type MakoKnowledgeScope = "workspace" | "all";
export type { MakoCurrentRunSummary };

export interface MakoChatContext {
  sessionId: string | null;
  title: string | null;
  messages: ChatMessage[];
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
  }) => Promise<string | null>;
  recoverDaemon: () => Promise<number>;
  isDispatching: boolean;
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
