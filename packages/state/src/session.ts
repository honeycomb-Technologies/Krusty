import { create } from "zustand";
import type {
  KrustyClient,
  ContentBlock,
  DelegatedProgressEvent,
  DelegatedToolStateResponse,
  DelegatedRunResponse,
  PlanItem,
  PartialAssistantState as ApiPartialAssistantState,
  SessionRecoveryState as ApiRecoveryState,
  SessionStateResponse as ApiSessionStateResponse,
  StreamCallbacks,
} from "@krusty/api";
import type { KrustyStorage } from "./storage";
import type { createWorkspaceStore } from "./workspace";
import type { createSessionsStore } from "./sessions";
import type { createPlanStore } from "./plan";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const STATE_POLL_INTERVAL = 3000;
const PRESENCE_HEARTBEAT_INTERVAL = 10_000;
const MAX_QUEUED_MESSAGES = 50;
const MAX_MESSAGE_CONTENT_LENGTH = 500_000;
const PRESENCE_CLIENT_STORAGE_KEY = "krusty:presence-client-id";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ToolCall {
  id: string;
  name: string;
  description?: string;
  arguments?: Record<string, unknown>;
  output?: string;
  delegatedRunId?: string;
  delegated?: DelegatedArtifactState;
  status:
    | "pending"
    | "running"
    | "success"
    | "partial"
    | "error"
    | "awaiting_approval";
}

export interface DelegatedAgentState {
  taskId: string;
  name: string;
  status: "pending" | "running" | "complete" | "failed";
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
  kind: "explore" | "build";
  delegatedRunId?: string;
  stage?:
    | "created"
    | "running"
    | "synthesizing"
    | "complete"
    | "degraded"
    | "failed"
    | "cancelled";
  thinking?: string;
  message?: string;
  investigationSummary?: string;
  humanReview?: string;
  outcome?: "success" | "partial" | "failed";
  confidence?: "high" | "medium" | "low";
  structuralCoverage?: "high" | "medium" | "low";
  semanticCoverage?: "high" | "medium" | "low";
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

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  toolCalls?: ToolCall[];
  isQueued?: boolean;
  kind?: "recovery_notice" | "live_partial" | "streaming";
}

export type SessionMode = "build" | "plan";
export type PermissionMode = "supervised" | "autonomous";
export type ThinkingLevel = "off" | "low" | "medium" | "high" | "xhigh";

export interface Attachment {
  name: string;
  type: "image" | "file";
  mimeType: string;
  base64?: string;
  text?: string;
}

interface QueuedMessage {
  content: string;
  attachments: Attachment[];
  researchEnabled: boolean;
}

interface FastModelPair {
  standard: string;
  fast: string;
}

const FAST_MODEL_PAIRS: FastModelPair[] = [
  { standard: "gpt-5.4", fast: "gpt-5.4-mini" },
  { standard: "claude-opus-4-6", fast: "claude-haiku-4-5-20251001" },
  { standard: "claude-opus-4.6", fast: "claude-haiku-4.5" },
  { standard: "anthropic/claude-opus-4.6", fast: "anthropic/claude-haiku-4.5" },
];

// ---------------------------------------------------------------------------
// Thinking helpers
// ---------------------------------------------------------------------------

export function isThinkingEnabled(level: ThinkingLevel): boolean {
  return level !== "off";
}

export function cycleThinkingLevel(
  current: ThinkingLevel,
  model: string | null,
): ThinkingLevel {
  const modelLower = (model ?? "").toLowerCase();
  const isCodex = modelLower.includes("codex");
  const isOpus =
    modelLower.includes("opus-4-6")
    || modelLower.includes("opus-4.6")
    || modelLower.includes("opus 4.6");

  if (isCodex) {
    switch (current) {
      case "off":
        return "low";
      case "low":
        return "medium";
      case "medium":
        return "high";
      case "high":
        return "xhigh";
      case "xhigh":
        return "off";
    }
  }

  if (isOpus) {
    switch (current) {
      case "off":
        return "low";
      case "low":
        return "medium";
      case "medium":
        return "high";
      case "high":
        return "xhigh";
      case "xhigh":
        return "off";
    }
  }

  if (current === "off") return "medium";
  return "off";
}

export function thinkingLevelToApiValue(
  level: ThinkingLevel,
): string | undefined {
  if (level === "off") return undefined;
  switch (level) {
    case "low":
      return "low";
    case "medium":
      return "medium";
    case "high":
      return "high";
    case "xhigh":
      return "xhigh";
  }
  return undefined;
}

export function thinkingLevelLabel(level: ThinkingLevel): string {
  return level;
}

function findFastModelPair(model: string | null): FastModelPair | undefined {
  if (!model) return undefined;
  return FAST_MODEL_PAIRS.find(
    (pair) => pair.standard === model || pair.fast === model,
  );
}

export function supportsFastMode(model: string | null): boolean {
  return findFastModelPair(model) !== undefined;
}

export function isFastModeModel(model: string | null): boolean {
  const pair = findFastModelPair(model);
  return pair !== undefined && pair.fast === model;
}

export function toggleFastModeModel(model: string | null): string | null {
  const pair = findFastModelPair(model);
  if (!pair || !model) return model;
  return pair.fast === model ? pair.standard : pair.fast;
}

// ---------------------------------------------------------------------------
// Store state interface
// ---------------------------------------------------------------------------

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
  tokenCount: number;
  lastEventSequence: number | null;
  error: string | null;
  model: string | null;

  sendMessage: (
    content: string,
    attachments?: Attachment[],
    researchEnabled?: boolean,
  ) => Promise<void>;
  loadSession: (sessionId: string, isRefresh?: boolean) => Promise<void>;
  clearSession: () => void;
  initSession: (sessionId: string, title: string) => void;
  setTitle: (title: string) => void;
  updateTitle: (sessionId: string, title: string) => Promise<void>;
  setMode: (mode: SessionMode) => void;
  setModel: (model: string) => void;
  setThinkingLevel: (level: ThinkingLevel) => void;
  toggleThinking: () => void;
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

// ---------------------------------------------------------------------------
// Helper: error coercion
// ---------------------------------------------------------------------------

function toErrorMessage(err: unknown, fallback = "Unknown error"): string {
  return err instanceof Error ? err.message : fallback;
}

function createChatMessageId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

function buildStoredMessageId(index: number, message: ChatMessage): string {
  const firstToolId = message.toolCalls?.[0]?.id;
  if (firstToolId) {
    return `stored-${index}-${message.role}-${firstToolId}`;
  }

  return [
    "stored",
    index,
    message.role,
    message.content.length,
    message.thinking?.length ?? 0,
  ].join("-");
}

function isTransientAssistantKind(
  kind: ChatMessage["kind"] | undefined,
): kind is "live_partial" | "streaming" {
  return kind === "live_partial" || kind === "streaming";
}

function isTransientAssistantMessage(
  message: ChatMessage | undefined,
): boolean {
  return message?.role === "assistant" && isTransientAssistantKind(message.kind);
}

function upsertTransientAssistantMessage(
  messages: ChatMessage[],
  message: ChatMessage,
): ChatMessage[] {
  const nextMessages = messages.filter((entry) => entry.kind !== "live_partial");
  const lastIndex = nextMessages.length - 1;
  const lastMessage = nextMessages[lastIndex];

  if (lastIndex >= 0 && isTransientAssistantMessage(lastMessage)) {
    nextMessages[lastIndex] = { ...message, id: lastMessage.id };
    return nextMessages;
  }

  return [...nextMessages, message];
}

function finalizeTransientAssistantMessages(
  messages: ChatMessage[],
): ChatMessage[] {
  return messages.map((message) =>
    isTransientAssistantMessage(message)
      ? { ...message, kind: undefined }
      : message,
  );
}

function pruneEmptyAssistantMessages(messages: ChatMessage[]): ChatMessage[] {
  return messages.filter((message) => {
    if (message.role !== "assistant") {
      return true;
    }

    return (
      message.content.trim().length > 0 ||
      (message.thinking?.trim().length ?? 0) > 0 ||
      (message.toolCalls?.length ?? 0) > 0
    );
  });
}

// ---------------------------------------------------------------------------
// Recovery / live-partial / delegation helpers
// ---------------------------------------------------------------------------

function buildRecoveryNotice(recovery: ApiRecoveryState): string {
  const headline =
    recovery.stop_reason === "stream_idle_timeout"
      ? "Previous turn stopped after the provider stream went idle."
      : recovery.stop_reason === "provider_error"
        ? "Previous turn stopped after a provider error."
        : recovery.stop_reason === "user_abort"
          ? "Previous turn was interrupted by user cancellation."
          : recovery.status === "tool_executing"
            ? "Previous turn ended while tool execution was in progress."
            : recovery.status === "streaming"
              ? "Previous turn ended while the assistant was still streaming."
              : "Previous turn ended before Krusty could safely finalize it.";

  const details: string[] = [];
  if (recovery.partial_assistant.text.trim()) {
    details.push(`Partial output: ${recovery.partial_assistant.text.trim()}`);
  }
  if (recovery.last_error?.trim()) {
    details.push(`Last error: ${recovery.last_error.trim()}`);
  }

  return details.length > 0 ? `${headline}\n\n${details.join("\n")}` : headline;
}

function applyRecoveryParity(
  messages: ChatMessage[],
  recovery: ApiRecoveryState | null | undefined,
  agentState: string,
): ChatMessage[] {
  let nextMessages = messages.filter((m) => m.kind !== "recovery_notice");

  if (recovery && agentState === "idle") {
    nextMessages = nextMessages.map((m) => ({
      ...m,
      toolCalls: m.toolCalls?.map((tc) => {
        if (
          (tc.status === "pending" || tc.status === "running") &&
          !tc.output
        ) {
          return {
            ...tc,
            status: "error" as const,
            output: "[Session interrupted - tool execution was cancelled]",
          };
        }
        return tc;
      }),
    }));

    nextMessages.unshift({
      id: [
        "recovery-notice",
        recovery.schema_version,
        recovery.status,
        recovery.stop_reason ?? "unknown",
      ].join("-"),
      role: "assistant",
      content: `[Recovery Notice] ${buildRecoveryNotice(recovery)}`,
      kind: "recovery_notice",
    });
  }

  return nextMessages;
}

function livePartialToolStatus(agentState: string): ToolCall["status"] {
  switch (agentState) {
    case "tool_executing":
      return "running";
    case "awaiting_input":
      return "awaiting_approval";
    default:
      return "pending";
  }
}

function isDelegatedToolName(name: string): name is "explore" | "build" {
  return name === "explore" || name === "build";
}

function delegatedDisplayName(path: string, fallback: string): string {
  const parts = path.split("/").filter(Boolean);
  return (parts[parts.length - 1] || fallback).slice(0, 24);
}

function buildSeedDelegatedAgents(
  kind: "explore" | "build",
  args?: Record<string, unknown>,
): DelegatedAgentState[] {
  if (!args) return [];
  const directories = Array.isArray(args.directories) ? args.directories : [];
  const files = Array.isArray(args.files) ? args.files : [];
  const components = Array.isArray(args.components) ? args.components : [];
  const sources = [...directories, ...files, ...components].filter(
    (v): v is string => typeof v === "string" && v.trim().length > 0,
  );

  if (sources.length === 0) {
    return [
      {
        taskId: "main",
        name: kind === "build" ? "builder" : "agent",
        status: "pending",
        toolCount: 0,
        tokens: 0,
        linesAdded: 0,
        linesRemoved: 0,
      },
    ];
  }

  return sources.map((source, index) => ({
    taskId: kind === "explore" ? `dir-${index}` : `${kind}-${index}`,
    name: delegatedDisplayName(source, kind === "build" ? "builder" : "agent"),
    status: "pending" as const,
    toolCount: 0,
    tokens: 0,
    currentAction:
      typeof args.prompt === "string" ? args.prompt.slice(0, 80) : undefined,
    linesAdded: 0,
    linesRemoved: 0,
  }));
}

function createDelegatedArtifactState(
  kind: "explore" | "build",
  args?: Record<string, unknown>,
): DelegatedArtifactState {
  return {
    kind,
    delegatedRunId: undefined,
    stage: "created",
    thinking: undefined,
    agents: buildSeedDelegatedAgents(kind, args),
    filesExamined: [],
    errors: [],
    totalTargets: buildSeedDelegatedAgents(kind, args).length,
  };
}

function mergeDelegatedAgents(
  current: DelegatedAgentState[] | undefined,
  next: DelegatedAgentState[],
): DelegatedAgentState[] {
  if (!current || current.length === 0) return next;
  if (next.length === 0) return current;

  const merged = [...current];
  for (const nextAgent of next) {
    const index = merged.findIndex(
      (a) => a.taskId === nextAgent.taskId || a.name === nextAgent.name,
    );
    if (index >= 0) {
      merged[index] = { ...merged[index], ...nextAgent };
    } else {
      merged.push(nextAgent);
    }
  }
  return merged;
}

function annotateDelegatedArtifactState(
  artifact: DelegatedArtifactState,
): DelegatedArtifactState {
  const totalTargets =
    artifact.totalTargets ?? artifact.agentCount ?? artifact.agents.length;
  const activeTargets = artifact.agents.filter(
    (a) => a.status === "running",
  ).length;
  const completedTargets = artifact.agents.filter(
    (a) => a.status === "complete",
  ).length;
  const pendingTargets = Math.max(
    totalTargets -
      activeTargets -
      completedTargets -
      artifact.agents.filter((a) => a.status === "failed").length,
    artifact.agents.filter((a) => a.status === "pending").length,
  );
  return {
    ...artifact,
    totalTargets,
    activeTargets,
    completedTargets,
    pendingTargets,
  };
}

function mergeDelegatedArtifactState(
  current: DelegatedArtifactState | undefined,
  next: DelegatedArtifactState,
): DelegatedArtifactState {
  return annotateDelegatedArtifactState({
    ...current,
    ...next,
    agents: mergeDelegatedAgents(current?.agents, next.agents),
    filesExamined:
      next.filesExamined.length > 0
        ? next.filesExamined
        : current?.filesExamined || [],
    errors: next.errors.length > 0 ? next.errors : current?.errors || [],
  });
}

function parseDelegatedArtifactState(
  toolName: string,
  output?: string,
): DelegatedArtifactState | undefined {
  if (!output || !isDelegatedToolName(toolName)) return undefined;

  try {
    const parsed = JSON.parse(output) as Record<string, unknown>;
    const payload =
      parsed?.data && typeof parsed.data === "object"
        ? (parsed.data as Record<string, unknown>)
        : parsed;

    const listKey = toolName === "build" ? "builders" : "agents";
    const artifact: DelegatedArtifactState = {
      kind: toolName,
      delegatedRunId:
        typeof payload.delegated_run_id === "string"
          ? payload.delegated_run_id
          : undefined,
      stage:
        payload.outcome === "success"
          ? "complete"
          : payload.outcome === "partial"
            ? "degraded"
            : payload.outcome === "failed"
              ? "failed"
              : undefined,
      message:
        typeof payload.message === "string" ? payload.message : undefined,
      investigationSummary:
        typeof payload.investigation_summary === "string"
          ? payload.investigation_summary
          : typeof payload.findings === "string"
            ? payload.findings
            : undefined,
      humanReview:
        typeof payload.human_review === "string"
          ? payload.human_review
          : undefined,
      outcome:
        payload.outcome === "success" ||
        payload.outcome === "partial" ||
        payload.outcome === "failed"
          ? payload.outcome
          : typeof payload.success === "boolean"
            ? payload.success
              ? "success"
              : "failed"
            : undefined,
      confidence:
        payload.confidence === "high" ||
        payload.confidence === "medium" ||
        payload.confidence === "low"
          ? payload.confidence
          : undefined,
      structuralCoverage:
        payload.structural_coverage === "high" ||
        payload.structural_coverage === "medium" ||
        payload.structural_coverage === "low"
          ? payload.structural_coverage
          : undefined,
      semanticCoverage:
        payload.semantic_coverage === "high" ||
        payload.semantic_coverage === "medium" ||
        payload.semantic_coverage === "low"
          ? payload.semantic_coverage
          : undefined,
      agents: [],
      filesExamined: Array.isArray(payload.paths_examined)
        ? payload.paths_examined.filter(
            (v): v is string => typeof v === "string",
          )
        : Array.isArray(payload.files_examined)
          ? payload.files_examined.filter(
              (v): v is string => typeof v === "string",
            )
          : [],
      errors: Array.isArray(payload.errors)
        ? payload.errors.filter((v): v is string => typeof v === "string")
        : [],
      agentCount:
        typeof payload.agent_count === "number"
          ? payload.agent_count
          : typeof payload.builder_count === "number"
            ? payload.builder_count
            : undefined,
      usableAgents:
        typeof payload.usable_agents === "number"
          ? payload.usable_agents
          : undefined,
      degradedAgents:
        typeof payload.degraded_agents === "number"
          ? payload.degraded_agents
          : undefined,
      successfulAgents:
        typeof payload.successful_agents === "number"
          ? payload.successful_agents
          : undefined,
      failedAgents:
        typeof payload.failed_agents === "number"
          ? payload.failed_agents
          : undefined,
      filesExaminedCount:
        typeof payload.paths_examined_count === "number"
          ? payload.paths_examined_count
          : typeof payload.files_examined_count === "number"
            ? payload.files_examined_count
            : undefined,
      outcomeReason:
        typeof payload.outcome_reason === "string"
          ? payload.outcome_reason
          : undefined,
      totalTurns:
        typeof payload.total_turns === "number"
          ? payload.total_turns
          : typeof payload.turns_used === "number"
            ? payload.turns_used
            : undefined,
      totalDurationMs:
        typeof payload.total_duration_ms === "number"
          ? payload.total_duration_ms
          : typeof payload.duration_ms === "number"
            ? payload.duration_ms
            : undefined,
      linesAdded:
        typeof payload.lines_added === "number"
          ? payload.lines_added
          : undefined,
      linesRemoved:
        typeof payload.lines_removed === "number"
          ? payload.lines_removed
          : undefined,
      filesModified:
        typeof payload.files_modified === "number"
          ? payload.files_modified
          : undefined,
      lockContentions:
        typeof payload.lock_contentions === "number"
          ? payload.lock_contentions
          : undefined,
      totalLockWaitMs:
        typeof payload.total_lock_wait_ms === "number"
          ? payload.total_lock_wait_ms
          : undefined,
      coverageGapNotice:
        typeof payload.coverage_gap_notice === "string"
          ? payload.coverage_gap_notice
          : undefined,
    };

    const agents = payload[listKey];
    if (Array.isArray(agents)) {
      artifact.agents = agents.flatMap((entry) => {
        if (!entry || typeof entry !== "object") return [];
        const record = entry as Record<string, unknown>;
        const taskId =
          typeof record.agent === "string"
            ? record.agent
            : typeof record.task_id === "string"
              ? record.task_id
              : toolName;
        const summary =
          typeof record.summary === "string"
            ? record.summary
            : typeof record.output === "string"
              ? record.output
              : undefined;
        const error =
          typeof record.error === "string" ? record.error : undefined;
        return [
          {
            taskId,
            name: delegatedDisplayName(
              taskId,
              toolName === "build" ? "builder" : "agent",
            ),
            status: error ? ("failed" as const) : ("complete" as const),
            outcomeReason:
              typeof record.outcome_reason === "string"
                ? record.outcome_reason
                : undefined,
            toolCount:
              typeof record.tool_calls === "number"
                ? record.tool_calls
                : typeof record.turns_used === "number"
                  ? record.turns_used
                  : 0,
            tokens: 0,
            currentAction: summary?.slice(0, 120),
            completionSummary: summary,
            linesAdded:
              typeof record.lines_added === "number" ? record.lines_added : 0,
            linesRemoved:
              typeof record.lines_removed === "number"
                ? record.lines_removed
                : 0,
            completedPlanTask:
              typeof record.completed_plan_task === "string"
                ? record.completed_plan_task
                : undefined,
          },
        ];
      });
    }

    return artifact;
  } catch {
    return undefined;
  }
}

function applyDelegatedProgress(
  toolCall: ToolCall,
  event: DelegatedProgressEvent,
): ToolCall {
  const delegated = toolCall.delegated
    ? { ...toolCall.delegated, agents: [...toolCall.delegated.agents] }
    : createDelegatedArtifactState(
        event.kind as "explore" | "build",
        toolCall.arguments,
      );
  delegated.delegatedRunId = event.delegated_run_id;
  delegated.stage = event.stage;
  const index = delegated.agents.findIndex((a) => a.taskId === event.task_id);
  const agent: DelegatedAgentState = {
    taskId: event.task_id,
    name: event.agent_name,
    status: event.status as DelegatedAgentState["status"],
    outcomeReason: undefined,
    toolCount: event.tool_count,
    tokens: event.tokens,
    currentAction: event.current_action || undefined,
    completionSummary: event.completion_summary || undefined,
    linesAdded: event.lines_added,
    linesRemoved: event.lines_removed,
    completedPlanTask: event.completed_plan_task || undefined,
  };

  if (index >= 0) {
    delegated.agents[index] = agent;
  } else {
    delegated.agents.push(agent);
  }

  delegated.agentCount = Math.max(
    delegated.agentCount || 0,
    delegated.agents.length,
  );
  const successfulAgents = delegated.agents.filter(
    (a) => a.status === "complete",
  ).length;
  const failedAgents = delegated.agents.filter(
    (a) => a.status === "failed",
  ).length;
  delegated.successfulAgents = successfulAgents;
  delegated.failedAgents = failedAgents;
  delegated.outcome =
    failedAgents === 0
      ? "success"
      : successfulAgents === 0
        ? "failed"
        : "partial";
  return {
    ...toolCall,
    delegatedRunId: event.delegated_run_id,
    delegated: annotateDelegatedArtifactState(delegated),
  };
}

function applyLivePartialAssistant(
  messages: ChatMessage[],
  livePartial: ApiPartialAssistantState | null | undefined,
  agentState: string,
): ChatMessage[] {
  const nextMessages = messages.filter((m) => m.kind !== "live_partial");
  if (
    !livePartial ||
    !["streaming", "tool_executing", "awaiting_input"].includes(agentState)
  ) {
    return nextMessages;
  }

  const hasContent = livePartial.text.trim().length > 0;
  const hasThinking = (livePartial.thinking?.trim().length ?? 0) > 0;
  const toolCalls = livePartial.tool_calls.map(
    (tc) =>
      ({
        id: tc.id,
        name: tc.name,
        delegated: isDelegatedToolName(tc.name)
          ? createDelegatedArtifactState(tc.name)
          : undefined,
        status: livePartialToolStatus(agentState),
      }) satisfies ToolCall,
  );

  if (!hasContent && !hasThinking && toolCalls.length === 0) {
    return nextMessages;
  }

  return upsertTransientAssistantMessage(nextMessages, {
    id: createChatMessageId("live-partial"),
    role: "assistant",
    content: livePartial.text,
    thinking: livePartial.thinking,
    toolCalls,
    kind: "live_partial",
  });
}

function applyDelegatedSessionState(
  messages: ChatMessage[],
  delegatedTools: DelegatedToolStateResponse[] | null | undefined,
  recentRuns?: DelegatedRunResponse[] | null | undefined,
): ChatMessage[] {
  if (
    (!delegatedTools || delegatedTools.length === 0) &&
    (!recentRuns || recentRuns.length === 0)
  ) {
    return messages;
  }

  const delegatedByToolCall = new Map(
    (delegatedTools || []).map((snapshot) => [snapshot.tool_call_id, snapshot]),
  );
  const recentRunByToolCall = new Map(
    (recentRuns || [])
      .filter(
        (run) =>
          typeof run.parent_tool_call_id === "string" &&
          run.parent_tool_call_id!.length > 0,
      )
      .map((run) => [run.parent_tool_call_id as string, run]),
  );

  return messages.map((message) => ({
    ...message,
    toolCalls: message.toolCalls?.map((toolCall) => {
      if (!isDelegatedToolName(toolCall.name)) return toolCall;

      const snapshot = delegatedByToolCall.get(toolCall.id);
      const recentRun = recentRunByToolCall.get(toolCall.id);
      if (!snapshot && !recentRun) return toolCall;

      if (!snapshot) {
        const delegated = toolCall.delegated
          ? { ...toolCall.delegated }
          : createDelegatedArtifactState(toolCall.name, toolCall.arguments);
        if (recentRun?.stage) {
          delegated.stage = recentRun.stage;
        }
        return {
          ...toolCall,
          delegatedRunId:
            recentRun?.delegated_run_id || toolCall.delegatedRunId,
          delegated,
        };
      }

      const delegated: DelegatedArtifactState = {
        kind: snapshot.kind as "explore" | "build",
        delegatedRunId:
          snapshot.delegated_run_id ||
          recentRun?.delegated_run_id ||
          toolCall.delegatedRunId,
        stage: snapshot.stage,
        agents: snapshot.agents.map((a) => ({
          taskId: a.task_id,
          name: a.agent_name,
          status: a.status as DelegatedAgentState["status"],
          outcomeReason: undefined,
          toolCount: a.tool_count,
          tokens: a.tokens,
          currentAction: a.current_action || undefined,
          completionSummary: a.completion_summary || undefined,
          linesAdded: a.lines_added,
          linesRemoved: a.lines_removed,
          completedPlanTask: a.completed_plan_task || undefined,
        })),
        thinking: toolCall.delegated?.thinking,
        filesExamined: toolCall.delegated?.filesExamined || [],
        errors: toolCall.delegated?.errors || [],
        agentCount: Math.max(
          toolCall.delegated?.agentCount || 0,
          snapshot.agents.length,
        ),
        usableAgents: snapshot.agents.filter((a) => a.status === "complete")
          .length,
        degradedAgents: 0,
        successfulAgents: snapshot.agents.filter((a) => a.status === "complete")
          .length,
        failedAgents: snapshot.agents.filter((a) => a.status === "failed")
          .length,
        filesExaminedCount:
          toolCall.delegated?.filesExaminedCount ||
          toolCall.delegated?.filesExamined?.length ||
          0,
        totalTargets:
          toolCall.delegated?.totalTargets ||
          toolCall.delegated?.agentCount ||
          snapshot.agents.length,
        outcome: snapshot.agents.some((a) => a.status === "failed")
          ? snapshot.agents.some((a) => a.status === "complete")
            ? "partial"
            : "failed"
          : "success",
      };

      return {
        ...toolCall,
        delegatedRunId:
          snapshot.delegated_run_id ||
          recentRun?.delegated_run_id ||
          toolCall.delegatedRunId,
        delegated: mergeDelegatedArtifactState(toolCall.delegated, delegated),
      };
    }),
  }));
}

// ---------------------------------------------------------------------------
// Content extraction helpers
// ---------------------------------------------------------------------------

function extractTextContent(content: unknown): string {
  if (typeof content === "string") return content;

  if (Array.isArray(content)) {
    let text = "";
    for (const block of content) {
      if (!block || typeof block !== "object") continue;
      if (block.type !== "text" || typeof block.text !== "string") continue;
      text += text ? `\n${block.text}` : block.text;
    }
    return text;
  }

  if (content && typeof content === "object" && "text" in content) {
    const textValue = (content as Record<string, unknown>).text;
    if (typeof textValue === "string") return textValue;
  }

  return "";
}

function parseStoredMessage(
  m: { role: string; content: unknown },
  toolResults?: Map<string, { output: string; isError: boolean }>,
): ChatMessage {
  const role: "user" | "assistant" =
    m.role === "user" || m.role === "assistant" ? m.role : "assistant";
  const msg: ChatMessage = {
    id: "",
    role,
    content: "",
    thinking: "",
    toolCalls: [],
  };
  const contentArray = Array.isArray(m.content) ? m.content : [];

  for (const block of contentArray) {
    if (!block || typeof block !== "object") continue;

    if (block.type === "text" || ("text" in block && !block.type)) {
      if (msg.content.length < MAX_MESSAGE_CONTENT_LENGTH) {
        msg.content += (msg.content ? "\n" : "") + (block.text || "");
      }
    } else if (block.type === "thinking" || "thinking" in block) {
      const thinkingContent = block.thinking || "";
      msg.thinking = msg.thinking
        ? msg.thinking + "\n\n" + thinkingContent
        : thinkingContent;
    } else if (
      block.type === "tool_use" ||
      ("id" in block && "name" in block && "input" in block)
    ) {
      msg.toolCalls = msg.toolCalls || [];
      const toolResult = toolResults?.get(block.id);
      const delegated = parseDelegatedArtifactState(
        block.name,
        toolResult?.output,
      );
      const status: ToolCall["status"] = toolResult
        ? delegated?.outcome === "partial"
          ? "partial"
          : delegated?.outcome === "failed"
            ? "error"
            : toolResult.isError
              ? "error"
              : "success"
        : "pending";
      msg.toolCalls.push({
        id: block.id,
        name: block.name,
        arguments: block.input,
        output: toolResult?.output,
        delegatedRunId: delegated?.delegatedRunId,
        delegated,
        status,
      });
    }
  }

  if (
    !msg.content &&
    !msg.thinking &&
    (!msg.toolCalls || msg.toolCalls.length === 0)
  ) {
    msg.content = extractTextContent(m.content);
  }

  return msg;
}

function processStoredMessages(
  rawMessages: { role: string; content: unknown }[],
): ChatMessage[] {
  const result: ChatMessage[] = [];
  const toolResults = new Map<string, { output: string; isError: boolean }>();

  for (const m of rawMessages) {
    const contentArray = Array.isArray(m.content) ? m.content : [];
    for (const block of contentArray) {
      if (!block || typeof block !== "object") continue;
      if (block.type === "tool_result" || "tool_use_id" in block) {
        if (block.tool_use_id) {
          const output =
            typeof block.output === "string"
              ? block.output
              : typeof block.content === "string"
                ? block.content
                : JSON.stringify(block.output || block.content || "");
          toolResults.set(block.tool_use_id, {
            output,
            isError: block.is_error === true,
          });
        }
      }
    }
  }

  for (const m of rawMessages) {
    const msg = parseStoredMessage(m, toolResults);
    const hasContent = msg.content.trim().length > 0;
    const hasThinking = (msg.thinking?.trim().length ?? 0) > 0;
    const hasToolCalls = (msg.toolCalls?.length ?? 0) > 0;
    if (hasContent || hasThinking || hasToolCalls) {
      msg.id = buildStoredMessageId(result.length, msg);
      result.push(msg);
    }
  }

  return result;
}

// ---------------------------------------------------------------------------
// Content block builder for attachments
// ---------------------------------------------------------------------------

function buildContentBlocks(
  text: string,
  attachments: Attachment[],
): ContentBlock[] {
  const blocks: ContentBlock[] = [];

  for (const att of attachments) {
    if (att.type === "image" && att.base64) {
      blocks.push({
        type: "image",
        source: {
          type: "base64",
          media_type: att.mimeType || "image/png",
          data: att.base64,
        },
      });
    }
  }

  const fileSections: string[] = [];
  const attachedFileNames: string[] = [];
  for (const att of attachments) {
    if (att.type !== "file") continue;
    attachedFileNames.push(att.name);
    if (att.text) {
      fileSections.push(`

--- ${att.name} ---
${att.text}`);
    }
  }

  const fileContent = fileSections.join("");
  const fallbackFileLabel =
    attachedFileNames.length > 0 && fileContent.length === 0
      ? `

[Attached files: ${attachedFileNames.join(", ")}]`
      : "";
  const fullText = `${text}${fileContent}${fallbackFileLabel}`.trim();
  blocks.push({
    type: "text",
    text: fullText.length > 0 ? fullText : "Attached content",
  });

  return blocks;
}

// ---------------------------------------------------------------------------
// Mutable ref for stream callbacks
// ---------------------------------------------------------------------------

interface AssistantMessageRef {
  current: ChatMessage;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

export function createSessionStore(
  client: KrustyClient,
  storage: KrustyStorage,
  workspace: ReturnType<typeof createWorkspaceStore>,
  sessionsStore: ReturnType<typeof createSessionsStore>,
  planStore: ReturnType<typeof createPlanStore>,
) {
  let statePollingInterval: ReturnType<typeof setInterval> | null = null;
  let presenceHeartbeatInterval: ReturnType<typeof setInterval> | null = null;
  let abortController: AbortController | null = null;
  let presenceClientId: string | null = null;

  function getPresenceClientId(): string | null {
    if (presenceClientId) return presenceClientId;
    try {
      const existing = storage.get(PRESENCE_CLIENT_STORAGE_KEY);
      if (existing) {
        presenceClientId = existing;
        return existing;
      }
      const generated = crypto.randomUUID();
      storage.set(PRESENCE_CLIENT_STORAGE_KEY, generated);
      presenceClientId = generated;
      return generated;
    } catch {
      return null;
    }
  }

  function loadPermissionMode(): PermissionMode {
    try {
      const stored = storage.get("krusty-permission-mode");
      if (stored === "supervised" || stored === "autonomous") return stored;
    } catch {
      /* ignore */
    }
    return "supervised";
  }

  const initialState: Omit<
    SessionStoreState,
    | "sendMessage"
    | "loadSession"
    | "clearSession"
    | "initSession"
    | "setTitle"
    | "updateTitle"
    | "setMode"
    | "setModel"
    | "setThinkingLevel"
    | "toggleThinking"
    | "togglePermissionMode"
    | "submitToolResult"
    | "submitToolApproval"
    | "stopStreaming"
    | "startStatePolling"
    | "stopStatePolling"
    | "startPresenceHeartbeat"
    | "stopPresenceHeartbeat"
    | "cleanup"
  > = {
    sessionId: null,
    title: "New Chat",
    mode: "build",
    permissionMode: loadPermissionMode(),
    messages: [],
    queuedMessages: [],
    isLoading: false,
    isStreaming: false,
    isThinking: false,
    thinkingContent: "",
    thinkingEnabled: true,
    thinkingLevel: "medium",
    tokenCount: 0,
    lastEventSequence: null,
    error: null,
    model: null,
  };

  // -------------------------------------------------------------------------
  // Streaming callbacks factory
  // -------------------------------------------------------------------------

  function createStreamCallbacks(
    ref: AssistantMessageRef,
    set: (
      partial:
        | Partial<SessionStoreState>
        | ((s: SessionStoreState) => Partial<SessionStoreState>),
    ) => void,
    get: () => SessionStoreState,
  ): StreamCallbacks {
    function updateLastAssistantMessage(
      updater?: (s: SessionStoreState) => Partial<SessionStoreState>,
    ) {
      set((s) => {
        const messages = upsertTransientAssistantMessage(
          s.messages,
          { ...ref.current },
        );
        return { messages, ...updater?.(s) };
      });
    }

    function mapToolCalls(id: string, mapper: (tc: ToolCall) => ToolCall) {
      const toolCalls = ref.current.toolCalls;
      if (!toolCalls || toolCalls.length === 0) return;

      const index = toolCalls.findIndex((tc) => tc.id === id);
      if (index < 0) return;

      const nextToolCalls = [...toolCalls];
      nextToolCalls[index] = mapper(nextToolCalls[index]);
      ref.current.toolCalls = nextToolCalls;
      updateLastAssistantMessage();
    }

    return {
      onTextDelta: (delta) => {
        ref.current.content += delta;
        updateLastAssistantMessage(() => ({
          isLoading: false,
          isThinking: false,
        }));
      },

      onThinkingDelta: (thinking) => {
        ref.current.thinking = (ref.current.thinking || "") + thinking;
        const delegatedIndex = (ref.current.toolCalls || []).findIndex((tc) =>
          isDelegatedToolName(tc.name),
        );
        if (delegatedIndex >= 0) {
          const toolCalls = [...(ref.current.toolCalls || [])];
          const delegatedTool = toolCalls[delegatedIndex];
          if (!isDelegatedToolName(delegatedTool.name)) {
            updateLastAssistantMessage(() => ({
              isThinking: true,
              thinkingContent: ref.current.thinking || "",
            }));
            return;
          }
          toolCalls[delegatedIndex] = {
            ...delegatedTool,
            delegated: mergeDelegatedArtifactState(delegatedTool.delegated, {
              ...(delegatedTool.delegated ||
                createDelegatedArtifactState(
                  delegatedTool.name,
                  delegatedTool.arguments,
                )),
              thinking: ref.current.thinking || "",
            }),
          };
          ref.current.toolCalls = toolCalls;
        }
        updateLastAssistantMessage(() => ({
          isThinking: true,
          thinkingContent: ref.current.thinking || "",
        }));
      },

      onToolCallStart: (id, name) => {
        ref.current.toolCalls = [
          ...(ref.current.toolCalls || []),
          {
            id,
            name,
            delegated: isDelegatedToolName(name)
              ? createDelegatedArtifactState(name)
              : undefined,
            status: "running",
          },
        ];
        updateLastAssistantMessage();
      },

      onToolCallComplete: (id, _name, args) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          arguments: args,
          delegated: isDelegatedToolName(tc.name)
            ? mergeDelegatedArtifactState(
                tc.delegated,
                createDelegatedArtifactState(tc.name, args),
              )
            : tc.delegated,
        }));
      },

      onToolResult: (id, output, isError) => {
        mapToolCalls(id, (tc) => {
          const delegated = isDelegatedToolName(tc.name)
            ? mergeDelegatedArtifactState(
                tc.delegated,
                parseDelegatedArtifactState(tc.name, output) ||
                  createDelegatedArtifactState(tc.name, tc.arguments),
              )
            : tc.delegated;
          const status: ToolCall["status"] =
            delegated?.outcome === "partial"
              ? "partial"
              : delegated?.outcome === "failed"
                ? "error"
                : isError
                  ? "error"
                  : "success";
          return { ...tc, output, delegated, status };
        });
      },

      onToolOutputDelta: (id, delta) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          output: (tc.output || "") + delta,
        }));
      },

      onDelegatedProgress: (event) => {
        mapToolCalls(event.tool_call_id, (tc) =>
          applyDelegatedProgress(tc, event),
        );
      },

      onPlanUpdate: (items: PlanItem[]) => {
        planStore.getState().setItems(items);
      },

      onModeChange: (mode) => {
        const nextMode: SessionMode = mode === "plan" ? "plan" : "build";
        set({ mode: nextMode });
        planStore.getState().setVisible(nextMode === "plan");
        void persistSessionMode(get, nextMode);
      },

      onPlanComplete: (toolCallId, title, taskCount) => {
        const planConfirmCall: ToolCall = {
          id: toolCallId,
          name: "PlanConfirm",
          arguments: { title, task_count: taskCount },
          status: "pending",
        };
        ref.current.toolCalls = [
          ...(ref.current.toolCalls || []),
          planConfirmCall,
        ];
        updateLastAssistantMessage();
      },

      onTurnComplete: (_turn, hasMore) => {
        if (hasMore) {
          updateLastAssistantMessage();
        }
      },

      onToolApprovalRequired: (id, _name, args) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          arguments: args,
          status: "awaiting_approval",
        }));
      },

      onToolApproved: (id) => {
        mapToolCalls(id, (tc) => ({ ...tc, status: "running" }));
      },

      onToolDenied: (id) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          status: "error",
          output: "Denied by user",
        }));
      },

      onUsage: (promptTokens, completionTokens) => {
        set({ tokenCount: promptTokens + completionTokens });
      },

      onTitleUpdate: (title) => {
        set({ title });
        sessionsStore.getState().loadSessions();
      },

      onFinish: (sessionId) => {
        const currentState = get();
        const queued = currentState.queuedMessages;

        const messages = finalizeTransientAssistantMessages(
          currentState.messages.map((m) =>
            m.isQueued ? { ...m, isQueued: false } : m,
          ),
        );

        set({
          sessionId,
          messages: pruneEmptyAssistantMessages(messages),
          queuedMessages: [],
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
        });
        sessionsStore.getState().loadSessions();

        if (queued.length > 0) {
          const combinedContent = queued.map((q) => q.content).join("\n\n");
          const combinedAttachments = queued.flatMap((q) => q.attachments);
          const queuedResearchEnabled = queued.some((q) => q.researchEnabled);
          setTimeout(
            () =>
              get().sendMessage(
                combinedContent,
                combinedAttachments,
                queuedResearchEnabled,
              ),
            50,
          );
        }
      },

      onError: (error) => {
        set((s) => ({
          isLoading: false,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          messages: pruneEmptyAssistantMessages(
            finalizeTransientAssistantMessages(s.messages),
          ),
          error,
        }));
      },
    };
  }

  // -------------------------------------------------------------------------
  // Presence helpers
  // -------------------------------------------------------------------------

  async function syncSessionPresence(
    sessionId: string,
    getState: () => SessionStoreState,
  ) {
    const clientId = getPresenceClientId();
    if (!clientId) return;

    const state = getState();
    try {
      await client.heartbeatSessionPresence(sessionId, {
        client_id: clientId,
        surface: "mobile",
        capability: "controller",
        last_event_sequence: state.lastEventSequence,
      });
    } catch {
      // Presence heartbeat failed silently
    }
  }

  // -------------------------------------------------------------------------
  // Persist helpers
  // -------------------------------------------------------------------------

  async function persistSessionMode(
    getState: () => SessionStoreState,
    mode: SessionMode,
  ) {
    const state = getState();
    if (!state.sessionId) return;
    try {
      await client.updateSession(state.sessionId, { mode });
      sessionsStore.getState().loadSessions();
    } catch {
      // Failed to persist
    }
  }

  async function persistSessionModel(
    getState: () => SessionStoreState,
    model: string,
  ) {
    const state = getState();
    if (!state.sessionId) return;
    try {
      await client.updateSession(state.sessionId, { model });
      sessionsStore.getState().loadSessions();
    } catch {
      // Failed to persist
    }
  }

  // -------------------------------------------------------------------------
  // Apply session snapshot from server state
  // -------------------------------------------------------------------------

  function applySessionSnapshot(
    sessionId: string,
    serverState: ApiSessionStateResponse | null,
    isRefresh: boolean,
    set: (
      partial:
        | Partial<SessionStoreState>
        | ((s: SessionStoreState) => Partial<SessionStoreState>),
    ) => void,
    get: () => SessionStoreState,
  ) {
    if (!serverState) return;

    const nextMode: SessionMode = serverState.mode ?? "build";
    set((s) => ({
      mode: nextMode,
      isStreaming:
        serverState.agent_state === "streaming" ||
        serverState.agent_state === "tool_executing",
      isThinking:
        serverState.agent_state === "streaming"
          ? Boolean(serverState.live_partial_assistant?.thinking?.trim()) ||
            s.isThinking
          : false,
      thinkingContent: serverState.live_partial_assistant?.thinking || "",
      lastEventSequence: serverState.last_event_sequence ?? null,
      messages: applyDelegatedSessionState(
        applyLivePartialAssistant(
          applyRecoveryParity(
            s.messages,
            serverState.recovery,
            serverState.agent_state,
          ),
          serverState.live_partial_assistant,
          serverState.agent_state,
        ),
        serverState.delegated_tools,
        serverState.recent_delegated_runs,
      ),
    }));
    planStore.getState().setVisible(nextMode === "plan");

    if (
      (serverState.agent_state === "streaming" ||
        serverState.agent_state === "tool_executing") &&
      !isRefresh
    ) {
      get().startStatePolling(sessionId);
    }
  }

  // -------------------------------------------------------------------------
  // Create the Zustand store
  // -------------------------------------------------------------------------

  return create<SessionStoreState>((set, get) => ({
    ...initialState,

    // -- sendMessage --------------------------------------------------------

    async sendMessage(
      content: string,
      attachments: Attachment[] = [],
      researchEnabled = false,
    ) {
      const state = get();
      const ws = workspace.getState();
      const normalizedContent = content.trim();
      const requestMessage =
        normalizedContent.length > 0
          ? normalizedContent
          : attachments.length > 0
            ? "Please review the attached content."
            : content;

      const attachmentLabel =
        attachments.length > 0
          ? `[Attachments: ${attachments.map((a) => a.name).join(", ")}]`
          : "";
      const displayContent = attachmentLabel
        ? normalizedContent.length > 0
          ? `${normalizedContent}\n\n${attachmentLabel}`
          : attachmentLabel
        : requestMessage;

      if (state.isStreaming) {
        set((s) => {
          if (s.queuedMessages.length >= MAX_QUEUED_MESSAGES) {
            return {
              error:
                "Message queue is full. Please wait for the current response to finish.",
            };
          }
          return {
            queuedMessages: [
              ...s.queuedMessages,
              { content, attachments, researchEnabled },
            ],
            messages: [
              ...s.messages,
              {
                id: createChatMessageId("user-queued"),
                role: "user",
                content: displayContent,
                isQueued: true,
              },
            ],
          };
        });
        return;
      }

      const ref: AssistantMessageRef = {
        current: {
          id: createChatMessageId("assistant-stream"),
          role: "assistant",
          content: "",
          thinking: "",
          toolCalls: [],
          kind: "streaming",
        },
      };

      set((s) => ({
        messages: [
          ...s.messages,
          {
            id: createChatMessageId("user"),
            role: "user",
            content: displayContent,
          },
          ref.current,
        ],
        isLoading: true,
        isStreaming: true,
        error: null,
      }));

      abortController = new AbortController();

      const pollingSessionId = state.sessionId;
      if (pollingSessionId) {
        get().startStatePolling(pollingSessionId);
      }

      try {
        const contentBlocks =
          attachments.length > 0
            ? buildContentBlocks(requestMessage, attachments)
            : undefined;

        await client.streamChat(
          {
            session_id: state.sessionId ?? undefined,
            message: requestMessage,
            content: contentBlocks,
            project_dir: state.sessionId ? undefined : ws.directory,
            working_dir: state.sessionId ? undefined : ws.directory,
            workspace_mode: state.sessionId ? undefined : ws.mode,
            research_enabled: researchEnabled || undefined,
            model: state.model ?? undefined,
            thinking_enabled: thinkingLevelToApiValue(state.thinkingLevel),
            permission_mode: state.permissionMode,
            mode: state.mode,
          },
          createStreamCallbacks(ref, set, get),
          abortController.signal,
        );
      } catch (err) {
        set((s) => ({
          isLoading: false,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          messages: pruneEmptyAssistantMessages(
            finalizeTransientAssistantMessages(s.messages),
          ),
          error: toErrorMessage(err),
        }));
      } finally {
        get().stopStatePolling();
      }
    },

    // -- loadSession --------------------------------------------------------

    async loadSession(sessionId: string, isRefresh = false) {
      set({ isLoading: true });

      try {
        const data = await client.getSession(sessionId);
        const processedMessages = processStoredMessages(data.messages);

        let serverState: ApiSessionStateResponse | null = null;
        try {
          serverState = await client.getSessionState(sessionId);
        } catch {
          // State endpoint may not exist
        }

        const mode = serverState?.mode ?? data.session.mode ?? "build";
        set((s) => ({
          ...s,
          sessionId: data.session.id,
          title: data.session.title || "Untitled",
          mode,
          model: data.session.model ?? s.model,
          tokenCount: data.session.token_count ?? 0,
          messages: applyLivePartialAssistant(
            applyRecoveryParity(
              processedMessages,
              serverState?.recovery,
              serverState?.agent_state ?? "idle",
            ),
            serverState?.live_partial_assistant,
            serverState?.agent_state ?? "idle",
          ),
          isLoading: false,
        }));
        planStore.getState().setVisible(mode === "plan");

        workspace
          .getState()
          .initFromSession(
            data.session.id,
            data.session.project_dir ?? data.session.working_dir ?? null,
            (data.session.workspace_mode ??
              ((data.session.project_dir ?? data.session.working_dir)
                ? "selected"
                : "neutral")) as "neutral" | "selected" | "created",
          );

        applySessionSnapshot(sessionId, serverState, isRefresh, set, get);
        get().startPresenceHeartbeat(sessionId);
      } catch (err) {
        set({
          isLoading: false,
          error: toErrorMessage(err, "Failed to load session"),
        });
      }
    },

    // -- clearSession -------------------------------------------------------

    clearSession() {
      const current = get();
      get().stopPresenceHeartbeat(current.sessionId);
      set({
        ...initialState,
        permissionMode: current.permissionMode,
        model: current.model,
        thinkingLevel: current.thinkingLevel,
        thinkingEnabled: current.thinkingEnabled,
      });
      workspace.getState().clear();
    },

    // -- initSession --------------------------------------------------------

    initSession(sessionId: string, title: string) {
      const current = get();
      get().stopPresenceHeartbeat(current.sessionId);
      set({
        ...initialState,
        permissionMode: current.permissionMode,
        model: current.model,
        thinkingLevel: current.thinkingLevel,
        thinkingEnabled: current.thinkingEnabled,
        sessionId,
        title,
      });
      get().startPresenceHeartbeat(sessionId);
    },

    // -- setTitle ------------------------------------------------------------

    setTitle(title: string) {
      set({ title });
    },

    // -- updateTitle --------------------------------------------------------

    async updateTitle(sessionId: string, title: string) {
      try {
        await client.updateSession(sessionId, { title });
        set({ title });
        sessionsStore.getState().loadSessions();
      } catch {
        // Failed to update title
      }
    },

    // -- setMode ------------------------------------------------------------

    setMode(mode: SessionMode) {
      set({ mode });
      planStore.getState().setVisible(mode === "plan");
      void persistSessionMode(get, mode);
    },

    // -- setModel -----------------------------------------------------------

    setModel(model: string) {
      set({ model });
      void persistSessionModel(get, model);
    },

    // -- setThinkingLevel ---------------------------------------------------

    setThinkingLevel(level: ThinkingLevel) {
      set({
        thinkingLevel: level,
        thinkingEnabled: isThinkingEnabled(level),
      });
    },

    // -- toggleThinking -----------------------------------------------------

    toggleThinking() {
      set((s) => {
        const newLevel = cycleThinkingLevel(s.thinkingLevel, s.model);
        return {
          thinkingEnabled: isThinkingEnabled(newLevel),
          thinkingLevel: newLevel,
        };
      });
    },

    // -- togglePermissionMode -----------------------------------------------

    togglePermissionMode() {
      set((s) => {
        const newMode: PermissionMode =
          s.permissionMode === "supervised" ? "autonomous" : "supervised";
        try {
          storage.set("krusty-permission-mode", newMode);
        } catch {
          /* ignore */
        }
        return { permissionMode: newMode };
      });
    },

    // -- submitToolResult ---------------------------------------------------

    async submitToolResult(toolCallId: string, result: string) {
      const state = get();
      if (!state.sessionId) {
        throw new Error("No active session");
      }

      set((s) => ({
        messages: s.messages.map((m) => ({
          ...m,
          toolCalls: m.toolCalls?.map((tc) =>
            tc.id === toolCallId
              ? { ...tc, output: result, status: "success" as const }
              : tc,
          ),
        })),
        isStreaming: true,
        isLoading: true,
      }));

      abortController = new AbortController();
      get().startStatePolling(state.sessionId);

      const ref: AssistantMessageRef = {
        current: {
          id: createChatMessageId("assistant-stream"),
          role: "assistant",
          content: "",
          thinking: "",
          toolCalls: [],
          kind: "streaming",
        },
      };

      set((s) => ({
        messages: upsertTransientAssistantMessage(s.messages, ref.current),
      }));

      try {
        await client.streamToolResult(
          {
            session_id: state.sessionId,
            tool_call_id: toolCallId,
            result,
          },
          createStreamCallbacks(ref, set, get),
          abortController.signal,
        );
      } catch (err) {
        set((s) => ({
          isLoading: false,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          messages: pruneEmptyAssistantMessages(
            finalizeTransientAssistantMessages(s.messages),
          ),
          error: toErrorMessage(err),
        }));
      } finally {
        get().stopStatePolling();
      }
    },

    // -- submitToolApproval -------------------------------------------------

    async submitToolApproval(toolCallId: string, approved: boolean) {
      const state = get();
      if (!state.sessionId) return;

      set((s) => ({
        messages: s.messages.map((m) => ({
          ...m,
          toolCalls: m.toolCalls?.map((tc) =>
            tc.id === toolCallId
              ? approved
                ? { ...tc, status: "running" as const, output: undefined }
                : {
                    ...tc,
                    status: "error" as const,
                    output: tc.output ?? "Denied by user",
                  }
              : tc,
          ),
        })),
      }));

      await client.submitToolApproval(state.sessionId, toolCallId, approved);
      set({ isStreaming: true, isLoading: true });
      get().startStatePolling(state.sessionId);
    },

    // -- stopStreaming ------------------------------------------------------

    stopStreaming() {
      abortController?.abort();
      get().stopStatePolling();
      set((s) => ({
        isLoading: false,
        isStreaming: false,
        isThinking: false,
        thinkingContent: "",
        messages: pruneEmptyAssistantMessages(
          finalizeTransientAssistantMessages(s.messages),
        ),
      }));
    },

    // -- state polling ------------------------------------------------------

    startStatePolling(sessionId: string) {
      get().stopStatePolling();

      statePollingInterval = setInterval(async () => {
        try {
          const serverState = await client.getSessionState(sessionId);
          applySessionSnapshot(sessionId, serverState, true, set, get);

          if (
            serverState.agent_state === "idle" ||
            serverState.agent_state === "awaiting_input"
          ) {
            get().stopStatePolling();
            set({ isStreaming: false, isThinking: false });
            await get().loadSession(sessionId, true);
          }
        } catch {
          get().stopStatePolling();
        }
      }, STATE_POLL_INTERVAL);
    },

    stopStatePolling() {
      if (statePollingInterval) {
        clearInterval(statePollingInterval);
        statePollingInterval = null;
      }
    },

    // -- presence heartbeat -------------------------------------------------

    startPresenceHeartbeat(sessionId: string) {
      get().stopPresenceHeartbeat();
      void syncSessionPresence(sessionId, get);
      presenceHeartbeatInterval = setInterval(() => {
        void syncSessionPresence(sessionId, get);
      }, PRESENCE_HEARTBEAT_INTERVAL);
    },

    stopPresenceHeartbeat(sessionId?: string | null) {
      if (presenceHeartbeatInterval) {
        clearInterval(presenceHeartbeatInterval);
        presenceHeartbeatInterval = null;
      }

      if (!sessionId) return;

      const clientId = getPresenceClientId();
      if (!clientId) return;

      void client.removeSessionPresence(sessionId, clientId).catch(() => {});
    },

    // -- cleanup ------------------------------------------------------------

    cleanup() {
      get().stopStatePolling();
      const state = get();
      get().stopPresenceHeartbeat(state.sessionId);
    },
  }));
}
