import type { SessionResponse, SessionType } from "@mitsuro/api";

export interface CodeProjectThreadGroup {
  directory: string;
  sessions: SessionResponse[];
  updatedAt: number;
}

function updatedAt(session: SessionResponse): number {
  const value = new Date(session.updated_at).getTime();
  return Number.isFinite(value) ? value : 0;
}

export function chronologicalSessions(
  sessions: SessionResponse[],
  type: SessionType,
): SessionResponse[] {
  return sessions
    .filter((session) => session.session_type === type)
    .slice()
    .sort((left, right) => updatedAt(right) - updatedAt(left));
}

export function codeProjectThreadGroups(
  sessions: SessionResponse[],
): CodeProjectThreadGroup[] {
  const grouped = new Map<string, SessionResponse[]>();

  for (const session of sessions) {
    if (session.session_type !== "code") {
      continue;
    }
    const directory = session.project_dir ?? session.working_dir ?? "Neutral";
    const existing = grouped.get(directory) ?? [];
    existing.push(session);
    grouped.set(directory, existing);
  }

  return Array.from(grouped.entries())
    .map(([directory, projectSessions]) => {
      const sorted = projectSessions
        .slice()
        .sort((left, right) => updatedAt(right) - updatedAt(left));
      return {
        directory,
        sessions: sorted,
        updatedAt: sorted[0] ? updatedAt(sorted[0]) : 0,
      };
    })
    .sort((left, right) => right.updatedAt - left.updatedAt);
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

  return active.project_dir ?? active.working_dir ?? "Neutral";
}
