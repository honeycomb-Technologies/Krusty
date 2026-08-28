export interface HiveWorkerSendBindingSnapshot {
  activeMode: string;
  sessionId: string | null;
}

export function assertHiveWorkerSendAvailable(available: boolean): void {
  if (!available) {
    throw new Error(
      "The Hive connection is unavailable. Try again when reconnected.",
    );
  }
}

/**
 * Worker drafts and sends belong to one exact durable DM. Call this before
 * and after every awaited send prerequisite so a navigation cannot retarget
 * an already-started action to the newly active conversation.
 */
export function assertCurrentHiveWorkerSendBinding(
  expectedSessionId: string,
  current: HiveWorkerSendBindingSnapshot,
): void {
  if (
    current.activeMode !== "hive" || current.sessionId !== expectedSessionId
  ) {
    throw new Error("The active Worker conversation changed before send");
  }
}
