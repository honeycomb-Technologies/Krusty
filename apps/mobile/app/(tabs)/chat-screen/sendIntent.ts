import type { SessionType } from "@krusty/api";
import type { SendMessageOptions } from "@krusty/state";

export interface WorkspaceSnapshot {
  directory: string | null;
  mode: "neutral" | "selected" | "created";
  sessionId: string | null;
  /**
   * TODO(targetBranch-mobile): workspace state does not expose first-send branch
   * intent yet. When that dependent card lands, pass it through this typed field;
   * @krusty/state intentionally keeps it out of /api/chat until the server chat
   * contract accepts target_branch.
   */
  targetBranch?: string | null;
}

export interface FirstSendIntent {
  shouldCreateSessionBeforeSend: boolean;
  sendOptions: SendMessageOptions;
}

function selectedDirectory(workspace: WorkspaceSnapshot): string | null {
  const directory = workspace.directory?.trim();
  if (!directory || workspace.mode === "neutral") {
    return null;
  }
  return workspace.directory;
}

export function resolveFirstSendIntent({
  currentSessionId,
  sessionType,
  workspace,
}: {
  currentSessionId: string | null;
  sessionType: SessionType;
  workspace: WorkspaceSnapshot;
}): FirstSendIntent {
  const sendOptions: SendMessageOptions = { sessionType };
  const directory = selectedDirectory(workspace);
  const shouldStreamCodeWorkspaceSession =
    !currentSessionId && sessionType === "code" && directory !== null;

  if (shouldStreamCodeWorkspaceSession) {
    sendOptions.projectDir = directory;
    sendOptions.workingDir = directory;
    sendOptions.workspaceMode = workspace.mode;
    if (workspace.targetBranch?.trim()) {
      sendOptions.targetBranch = workspace.targetBranch;
    }
  }

  return {
    shouldCreateSessionBeforeSend:
      !currentSessionId && !shouldStreamCodeWorkspaceSession,
    sendOptions,
  };
}
