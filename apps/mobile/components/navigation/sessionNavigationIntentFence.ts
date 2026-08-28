export function isCurrentSessionNavigationIntent(
  intentGeneration: number,
  currentGeneration: number,
  expectedSessionId: string | null,
  currentSessionId: string | null,
): boolean {
  return intentGeneration === currentGeneration &&
    expectedSessionId === currentSessionId;
}

/**
 * Bind a send continuation to the selection that owned its composer.
 *
 * Persisted sessions require an exact id match. A blank optimistic shell may
 * precreate its server session before transport starts, but that captured
 * transport id must remain the exact current destination when the send settles.
 */
export function isCurrentSessionSendIntent(
  intentGeneration: number,
  currentGeneration: number,
  originatingSessionId: string | null,
  transportSessionId: string | null,
  currentSessionId: string | null,
): boolean {
  return intentGeneration === currentGeneration &&
    (originatingSessionId === null ||
      originatingSessionId === transportSessionId) &&
    transportSessionId === currentSessionId;
}
