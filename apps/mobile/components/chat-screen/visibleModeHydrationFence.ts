export function isCurrentVisibleModeHydrationIntent(
  scheduledTargetId: string,
  scheduledStoreSessionId: string | null,
  currentRememberedId: string | null,
  currentStoreSessionId: string | null,
): boolean {
  if (currentRememberedId !== scheduledTargetId) return false;
  return currentStoreSessionId === scheduledTargetId ||
    currentStoreSessionId === scheduledStoreSessionId;
}
