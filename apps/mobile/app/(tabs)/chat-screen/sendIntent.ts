import type { SessionResponse, SessionType, WorkspaceMode } from "@mitsuro/api";
import type { SendMessageOptions } from "@mitsuro/state";

const TAB_SESSION_TYPES: SessionType[] = ["chat", "code", "hive"];

interface ResolveSendIntentArgs {
  activeTab: number;
  currentSessionId: string | null;
  workspaceDirectory: string | null;
  workspaceMode: WorkspaceMode;
  targetBranch?: string | null;
}

interface PrecreateSessionIntent {
  projectDir?: string | null;
  targetBranch?: string | null;
  workspaceMode?: WorkspaceMode;
  sessionType: SessionType;
}

export interface ResolvedSendIntent {
  shouldPrecreate: boolean;
  precreate?: PrecreateSessionIntent;
  sendOptions?: SendMessageOptions;
}

function sessionTypeForTab(index: number): SessionType {
  return TAB_SESSION_TYPES[index] ?? "code";
}

function normalizeNullable(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

function sameTargetBranch(left: string | null | undefined, right: string | null | undefined): boolean {
  return normalizeNullable(left) === normalizeNullable(right);
}

export function findCodeSessionForProject(
  sessions: SessionResponse[],
  projectDir: string,
  targetBranch?: string | null,
): SessionResponse | null {
  const normalizedProjectDir = projectDir.trim();
  if (!normalizedProjectDir) {
    return null;
  }

  return (
    sessions.find((session) => {
      const sessionProjectDir = session.project_dir ?? session.working_dir ?? null;
      return (
        session.session_type === "code" &&
        sessionProjectDir === normalizedProjectDir &&
        sameTargetBranch(session.target_branch, targetBranch)
      );
    }) ?? null
  );
}

export function resolveSendIntent({
  activeTab,
  currentSessionId,
  workspaceDirectory,
  workspaceMode,
  targetBranch,
}: ResolveSendIntentArgs): ResolvedSendIntent {
  if (currentSessionId) {
    return { shouldPrecreate: false };
  }

  const sessionType = sessionTypeForTab(activeTab);
  const normalizedDirectory = workspaceDirectory?.trim() || null;

  if (
    sessionType === "code" &&
    workspaceMode !== "neutral" &&
    normalizedDirectory
  ) {
    return {
      shouldPrecreate: false,
      sendOptions: {
        projectDir: normalizedDirectory,
        workingDir: normalizedDirectory,
        workspaceMode,
        sessionType: "code",
        targetBranch: normalizeNullable(targetBranch),
      },
    };
  }

  return {
    shouldPrecreate: true,
    precreate: {
      workspaceMode: "neutral",
      sessionType,
    },
  };
}
