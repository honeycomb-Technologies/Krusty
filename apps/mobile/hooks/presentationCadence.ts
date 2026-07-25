export interface LiveActivitySemanticState {
  chatTitle: string;
  status: "working" | "needs_input" | "completed";
  toolCount: number;
  filesAdded: number;
  filesRemoved: number;
  toolApprovalId?: string;
  toolApprovalName?: string;
  toolApprovalSessionId?: string;
}

export interface ChatWidgetCadenceState {
  sessionId: string | null;
  hasActiveSession: boolean;
  sessionTitle: string;
  lastMessage: string;
  model: string;
  isStreaming: boolean;
  tokenCount: number;
  serverConnected: boolean;
}

export function liveActivityStateEqual(
  left: LiveActivitySemanticState | null,
  right: LiveActivitySemanticState | null,
): boolean {
  if (left === right) return true;
  if (!left || !right) return false;

  return (
    left.chatTitle === right.chatTitle &&
    left.status === right.status &&
    left.toolCount === right.toolCount &&
    left.filesAdded === right.filesAdded &&
    left.filesRemoved === right.filesRemoved &&
    left.toolApprovalId === right.toolApprovalId &&
    left.toolApprovalName === right.toolApprovalName &&
    left.toolApprovalSessionId === right.toolApprovalSessionId
  );
}

export function shouldSyncChatWidget(
  previous: ChatWidgetCadenceState | null,
  next: ChatWidgetCadenceState,
): boolean {
  if (!previous) return true;

  const lifecycleChanged =
    previous.sessionId !== next.sessionId ||
    previous.hasActiveSession !== next.hasActiveSession ||
    previous.sessionTitle !== next.sessionTitle ||
    previous.model !== next.model ||
    previous.isStreaming !== next.isStreaming ||
    previous.serverConnected !== next.serverConnected;

  if (lifecycleChanged) return true;

  // A Home Screen widget is a lifecycle snapshot, not a streaming surface.
  // Final content and usage are written after the stream settles.
  if (next.isStreaming) return false;

  return (
    previous.lastMessage !== next.lastMessage ||
    previous.tokenCount !== next.tokenCount
  );
}
