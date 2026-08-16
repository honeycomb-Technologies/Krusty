import type { SessionResponse, SessionType } from "@mitsuro/api";

export type ThreadDensity = "comfortable" | "compact";
export type CodeThreadView = "projects" | "recent";
export type SessionProviderKey =
  | "openai"
  | "anthropic"
  | "xai"
  | "minimax"
  | "openrouter"
  | "zai"
  | "unknown";

export interface CodeProjectThreadGroup {
  directory: string;
  sessions: SessionResponse[];
  updatedAt: number;
  pinnedAt: number;
}

export interface ChronologicalThreadDayGroup {
  key: string;
  label: string;
  sessions: SessionResponse[];
  dayStart: number;
}

function updatedAt(session: SessionResponse): number {
  const value = new Date(session.updated_at).getTime();
  return Number.isFinite(value) ? value : 0;
}

function pinnedAt(session: SessionResponse): number {
  if (!session.pinned_at) return 0;
  const value = new Date(session.pinned_at).getTime();
  return Number.isFinite(value) ? value : 0;
}

function compareThreadOrder(left: SessionResponse, right: SessionResponse): number {
  const leftPinned = pinnedAt(left);
  const rightPinned = pinnedAt(right);
  if (leftPinned !== rightPinned) {
    if (leftPinned === 0) return 1;
    if (rightPinned === 0) return -1;
    return rightPinned - leftPinned;
  }
  return updatedAt(right) - updatedAt(left);
}

export function sessionProjectDirectory(session: SessionResponse): string {
  return session.project_dir ?? session.working_dir ?? "Neutral";
}

export function sessionModelLabel(model: string | null | undefined): string | null {
  if (!model) return null;
  const segments = model.split(/[/:]/).filter(Boolean);
  return segments.at(-1) ?? model;
}

export function sessionProviderKey(
  session: Pick<SessionResponse, "model" | "model_key">,
): SessionProviderKey {
  const raw = `${session.model_key?.provider ?? ""} ${session.model ?? ""}`
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "");
  if (raw.includes("openrouter")) return "openrouter";
  if (raw.includes("openai")) return "openai";
  if (raw.includes("anthropic") || raw.includes("claude")) return "anthropic";
  if (raw.includes("minimax")) return "minimax";
  if (raw.includes("xai") || raw.includes("grok")) return "xai";
  if (raw.includes("zai") || raw.includes("zhipu")) return "zai";
  return "unknown";
}

export function sessionProviderLabel(
  session: Pick<SessionResponse, "model" | "model_key">,
): string {
  switch (sessionProviderKey(session)) {
    case "openai":
      return "OpenAI";
    case "anthropic":
      return "Anthropic";
    case "xai":
      return "Grok";
    case "minimax":
      return "MiniMax";
    case "openrouter":
      return "OpenRouter";
    case "zai":
      return "Z.AI";
    default:
      return "Agent";
  }
}

export function sessionStateLabel(
  agentState: string | null | undefined,
): string | null {
  switch (agentState) {
    case "streaming":
    case "thinking":
    case "tool_executing":
      return "Working";
    case "awaiting_input":
      return "Needs input";
    case "error":
    case "failed":
      return "Error";
    case "idle":
      return "Ready";
    default:
      return null;
  }
}

export function formatThreadMetric(value: number): string {
  if (value < 1_000) return String(value);
  const compact = value < 10_000 ? (value / 1_000).toFixed(1) : Math.round(value / 1_000).toString();
  return `${compact.replace(/\.0$/, "")}k`;
}

export function chronologicalSessions(
  sessions: SessionResponse[],
  type: SessionType,
): SessionResponse[] {
  return sessions
    .filter(
      (session) => session.session_type === type && !session.archived_at,
    )
    .slice()
    .sort(compareThreadOrder);
}

function localDayKey(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function chronologicalDayLabel(date: Date, now: Date): string {
  const key = localDayKey(date);
  if (key === localDayKey(now)) return "Today";

  const yesterday = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() - 1,
  );
  if (key === localDayKey(yesterday)) return "Yesterday";

  return new Intl.DateTimeFormat("en-US", {
    weekday: "long",
    month: "short",
    day: "numeric",
    ...(date.getFullYear() === now.getFullYear()
      ? {}
      : { year: "numeric" as const }),
  }).format(date);
}

export function chronologicalThreadDayGroups(
  sessions: SessionResponse[],
  type: SessionType,
  now = new Date(),
): ChronologicalThreadDayGroup[] {
  const groups = new Map<string, ChronologicalThreadDayGroup>();

  for (const session of chronologicalSessions(sessions, type)) {
    const date = new Date(updatedAt(session));
    const key = localDayKey(date);
    const existing = groups.get(key);
    if (existing) {
      existing.sessions.push(session);
      continue;
    }

    groups.set(key, {
      key,
      label: chronologicalDayLabel(date, now),
      sessions: [session],
      dayStart: new Date(
        date.getFullYear(),
        date.getMonth(),
        date.getDate(),
      ).getTime(),
    });
  }

  return Array.from(groups.values()).sort(
    (left, right) => right.dayStart - left.dayStart,
  );
}

export function archivedSessions(
  sessions: SessionResponse[],
  type: SessionType,
): SessionResponse[] {
  return sessions
    .filter(
      (session) => session.session_type === type && Boolean(session.archived_at),
    )
    .slice()
    .sort((left, right) => {
      const leftArchived = new Date(left.archived_at ?? 0).getTime();
      const rightArchived = new Date(right.archived_at ?? 0).getTime();
      return rightArchived - leftArchived;
    });
}

export function codeProjectThreadGroups(
  sessions: SessionResponse[],
): CodeProjectThreadGroup[] {
  const grouped = new Map<string, SessionResponse[]>();

  for (const session of sessions) {
    if (session.session_type !== "code" || session.archived_at) {
      continue;
    }
    const directory = sessionProjectDirectory(session);
    const existing = grouped.get(directory) ?? [];
    existing.push(session);
    grouped.set(directory, existing);
  }

  return Array.from(grouped.entries())
    .map(([directory, projectSessions]) => {
      const sorted = projectSessions
        .slice()
        .sort(compareThreadOrder);
      return {
        directory,
        sessions: sorted,
        updatedAt: sorted[0] ? updatedAt(sorted[0]) : 0,
        pinnedAt: Math.max(0, ...projectSessions.map(pinnedAt)),
      };
    })
    .sort((left, right) => {
      if (left.pinnedAt !== right.pinnedAt) {
        if (left.pinnedAt === 0) return 1;
        if (right.pinnedAt === 0) return -1;
        return right.pinnedAt - left.pinnedAt;
      }
      return right.updatedAt - left.updatedAt;
    });
}

export function codeDirectoryToAutoExpand(
  sessions: SessionResponse[],
  activeSessionId: string | null,
  lastAutoExpandedSessionId: string | null,
): string | null {
  if (!activeSessionId || activeSessionId === lastAutoExpandedSessionId) {
    return null;
  }

  const active = sessions.find(
    (session) =>
      session.id === activeSessionId && session.session_type === "code",
  );
  if (!active) {
    return null;
  }

  const directory = sessionProjectDirectory(active);
  return directory === "Neutral" ? null : directory;
}
