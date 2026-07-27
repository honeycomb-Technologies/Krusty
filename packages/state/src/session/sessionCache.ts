import type {
  ModelKey,
  SessionResponse,
  SessionStateResponse,
  SessionType,
  SessionWithMessagesResponse,
} from "@krusty/api";

import type { ChatMessage, PermissionMode, SessionMode } from "./types";

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

const DEFAULT_MAX_ENTRIES = 24;

export class SessionSnapshotCache {
  private readonly entries = new Map<string, CachedSessionSnapshot>();
  private readonly maxEntries: number;

  constructor(maxEntries = DEFAULT_MAX_ENTRIES) {
    this.maxEntries = Math.max(1, maxEntries);
  }

  get(sessionId: string): CachedSessionSnapshot | null {
    return this.entries.get(sessionId) ?? null;
  }

  set(snapshot: CachedSessionSnapshot): void {
    // Refresh insertion order so recently used sessions stay hot.
    this.entries.delete(snapshot.sessionId);
    this.entries.set(snapshot.sessionId, snapshot);
    this.trim();
  }

  delete(sessionId: string): void {
    this.entries.delete(sessionId);
  }

  clear(): void {
    this.entries.clear();
  }

  private trim(): void {
    while (this.entries.size > this.maxEntries) {
      const oldestKey = this.entries.keys().next().value;
      if (!oldestKey) {
        return;
      }
      this.entries.delete(oldestKey);
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
