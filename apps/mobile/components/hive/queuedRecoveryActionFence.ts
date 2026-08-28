export function isExactQueuedRecoveryActionTarget(
  expectedSessionId: string | null,
  currentSessionId: string | null,
  queuedRecoveryBlocked: boolean,
): boolean {
  return expectedSessionId !== null &&
    expectedSessionId === currentSessionId &&
    queuedRecoveryBlocked;
}
