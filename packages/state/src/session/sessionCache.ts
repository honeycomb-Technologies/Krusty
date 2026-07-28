import type {
  ModelKey,
  SessionResponse,
  SessionStateResponse,
  SessionType,
  SessionWithMessagesResponse,
} from "@krusty/api";

import type { ChatMessage, PermissionMode, SessionMode } from "./types";
import {
  MAX_CACHED_SESSION_MESSAGES,
  MAX_LIVE_MESSAGE_CONTENT_LENGTH,
  MAX_LIVE_THINKING_CONTENT_LENGTH,
  MAX_LIVE_TOOL_OUTPUT_LENGTH,
} from "./constants";

export interface CachedSessionSnapshot {
  sessionId: string;
  sessionType: SessionType | null;
  title: string;
  mode: SessionMode;
  permissionMode: PermissionMode;
  model: string | null;
  modelKey: ModelKey | null;
  tokenCount: number;
  messages: ChatMessage[];
  projectDir: string | null;
  workingDir: string | null;
  workspaceMode: SessionResponse["workspace_mode"] | null;
  targetBranch: string | null;
  serverState: SessionStateResponse | null;
  updatedAt: number;
}


function truncateText(value: string | undefined, max: number): string | undefined {
  if (!value) return value;
  if (value.length <= max) return value;
  return `${value.slice(0, max)}
…[truncated for cache]`;
}

/** Strip heavy payloads so warm-session cache cannot retain multi-MB transcripts. */
export function compactMessagesForCache(messages: ChatMessage[]): ChatMessage[] {
  const sliced =
    messages.length > MAX_CACHED_SESSION_MESSAGES
      ? messages.slice(messages.length - MAX_CACHED_SESSION_MESSAGES)
      : messages;

  return sliced.map((message) => {
    const next: ChatMessage = {
      ...message,
      content: truncateText(message.content, MAX_LIVE_MESSAGE_CONTENT_LENGTH) || "",
      thinking: truncateText(message.thinking, MAX_LIVE_THINKING_CONTENT_LENGTH),
      attachments: message.attachments?.map((attachment) => ({
        type: attachment.type,
        name: attachment.name,
        mimeType: attachment.mimeType,
        // Never keep base64 blobs in the warm session cache.
        uri: attachment.uri,
      })),
      toolCalls: message.toolCalls?.map((toolCall) => ({
        ...toolCall,
        output: truncateText(toolCall.output, MAX_LIVE_TOOL_OUTPUT_LENGTH),
        delegated: toolCall.delegated
          ? {
              ...toolCall.delegated,
              thinking: truncateText(
                toolCall.delegated.thinking,
                MAX_LIVE_THINKING_CONTENT_LENGTH,
              ),
              message: truncateText(toolCall.delegated.message, 2_000),
              investigationSummary: truncateText(
                toolCall.delegated.investigationSummary,
                2_000,
              ),
              humanReview: truncateText(toolCall.delegated.humanReview, 2_000),
              // Keep agent list short in cache.
              agents: toolCall.delegated.agents.slice(0, 8),
              filesExamined: toolCall.delegated.filesExamined.slice(0, 20),
              errors: toolCall.delegated.errors.slice(0, 10),
            }
          : undefined,
      })),
      renderParts: message.renderParts?.map((part) => {
        if (part.type === "text" || part.type === "thinking") {
          return {
            ...part,
            content:
              truncateText(
                part.content,
                part.type === "thinking"
                  ? MAX_LIVE_THINKING_CONTENT_LENGTH
                  : MAX_LIVE_MESSAGE_CONTENT_LENGTH,
              ) || "",
          };
        }
        return part;
      }),
    };
    return next;
  });
}

const DEFAULT_MAX_ENTRIES = 8;

export class SessionSnapshotCache {
  private readonly entries = new Map<string, CachedSessionSnapshot>();
  private readonly sourceMessages = new Map<string, ChatMessage[]>();
  private readonly maxEntries: number;

  constructor(maxEntries = DEFAULT_MAX_ENTRIES) {
    this.maxEntries = Math.max(1, maxEntries);
  }

  get(sessionId: string): CachedSessionSnapshot | null {
    const snapshot = this.entries.get(sessionId);
    if (!snapshot) return null;
    // Touch LRU so warm reopens stay hot even without a rewrite.
    this.entries.delete(sessionId);
    this.entries.set(sessionId, snapshot);
    return snapshot;
  }

  set(snapshot: CachedSessionSnapshot): void {
    const existing = this.entries.get(snapshot.sessionId);
    const previousSource = this.sourceMessages.get(snapshot.sessionId);
    const canReuseMessages = Boolean(existing) && (
      existing!.messages === snapshot.messages
      || previousSource === snapshot.messages
    );
    // Refresh insertion order so recently used sessions stay hot.
    const compact: CachedSessionSnapshot = {
      ...snapshot,
      messages: canReuseMessages
        ? existing!.messages
        : compactMessagesForCache(snapshot.messages),
      // Keep only lightweight server metadata, not full live partials.
      serverState: snapshot.serverState
        ? {
            ...snapshot.serverState,
            live_partial_assistant: null,
            delegated_tools: [],
            recent_delegated_runs: [],
          }
        : null,
    };
    this.entries.delete(compact.sessionId);
    this.entries.set(compact.sessionId, compact);
    this.sourceMessages.set(compact.sessionId, snapshot.messages);
    this.trim();
  }

  delete(sessionId: string): void {
    this.entries.delete(sessionId);
    this.sourceMessages.delete(sessionId);
  }

  clear(): void {
    this.entries.clear();
    this.sourceMessages.clear();
  }

  private trim(): void {
    while (this.entries.size > this.maxEntries) {
      const oldestKey = this.entries.keys().next().value;
      if (!oldestKey) {
        return;
      }
      this.entries.delete(oldestKey);
      this.sourceMessages.delete(oldestKey);
    }
  }
}

export function normalizeDisplayTitle(title: string | null | undefined): string {
  const trimmed = title?.trim() ?? "";
  const placeholder = trimmed.toLowerCase();
  return placeholder === "new chat" || placeholder === "new session"
    ? ""
    : trimmed;
}

export function buildSessionSnapshotFromResponse(
  data: SessionWithMessagesResponse,
  messages: ChatMessage[],
  serverState: SessionStateResponse | null,
): CachedSessionSnapshot {
  const session = data.session;
  return {
    sessionId: session.id,
    sessionType: session.session_type,
    title: normalizeDisplayTitle(session.title),
    mode: serverState?.mode ?? session.mode ?? "build",
    permissionMode:
      serverState?.permission_mode ?? session.permission_mode ?? "autonomous",
    model: session.model?.trim() || null,
    modelKey: session.model_key ?? null,
    tokenCount: session.token_count ?? 0,
    messages,
    projectDir: session.project_dir ?? null,
    workingDir: session.working_dir ?? null,
    workspaceMode: session.workspace_mode ?? null,
    targetBranch: session.target_branch ?? null,
    serverState,
    updatedAt: Date.now(),
  };
}

export function buildOptimisticSessionShell(
  sessionId: string,
  listItem?: Partial<SessionResponse> | null,
  previous?: CachedSessionSnapshot | null,
): CachedSessionSnapshot {
  return {
    sessionId,
    sessionType: listItem?.session_type ?? previous?.sessionType ?? null,
    title: normalizeDisplayTitle(listItem?.title ?? previous?.title ?? ""),
    mode: listItem?.mode ?? previous?.mode ?? "build",
    permissionMode:
      listItem?.permission_mode ?? previous?.permissionMode ?? "autonomous",
    model: listItem?.model?.trim() || previous?.model || null,
    modelKey: listItem?.model_key ?? previous?.modelKey ?? null,
    tokenCount: listItem?.token_count ?? previous?.tokenCount ?? 0,
    messages: previous?.messages ?? [],
    projectDir:
      listItem?.project_dir
      ?? previous?.projectDir
      ?? listItem?.working_dir
      ?? previous?.workingDir
      ?? null,
    workingDir:
      listItem?.working_dir
      ?? previous?.workingDir
      ?? listItem?.project_dir
      ?? previous?.projectDir
      ?? null,
    workspaceMode: listItem?.workspace_mode ?? previous?.workspaceMode ?? null,
    targetBranch: listItem?.target_branch ?? previous?.targetBranch ?? null,
    serverState: previous?.serverState ?? null,
    updatedAt: previous?.updatedAt ?? 0,
  };
}
