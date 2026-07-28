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

/** Grace window to coalesce end→start thrash on session/stream transitions. */
export const LIVE_ACTIVITY_TRANSITION_GRACE_MS = 400;

/** Minimum spacing between non-urgent Live Activity native updates. */
export const MIN_LIVE_ACTIVITY_UPDATE_INTERVAL_MS = 2_000;

export type LiveActivityTransition =
  | { action: "none" }
  | { action: "start"; sessionId: string }
  | { action: "update"; sessionId: string }
  | { action: "end"; sessionId: string };

/**
 * Resolve the next Live Activity lifecycle step for the currently focused session.
 * Keeps semantic correctness while letting the hook batch destroy/create churn.
 */
export function resolveLiveActivityTransition(input: {
  trackedSessionId: string | null;
  focusedSessionId: string | null;
  shouldKeepFocused: boolean;
}): LiveActivityTransition {
  const { trackedSessionId, focusedSessionId, shouldKeepFocused } = input;

  if (shouldKeepFocused && focusedSessionId) {
    if (trackedSessionId === focusedSessionId) {
      return { action: "update", sessionId: focusedSessionId };
    }
    return { action: "start", sessionId: focusedSessionId };
  }

  // Only end when the tracked activity belongs to the focused session (or focus
  // is empty). Leaving a streaming session for an idle one intentionally keeps
  // the previous activity alive so lock-screen state does not thrash.
  if (
    trackedSessionId &&
    (!focusedSessionId || trackedSessionId === focusedSessionId)
  ) {
    return { action: "end", sessionId: trackedSessionId };
  }

  return { action: "none" };
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
