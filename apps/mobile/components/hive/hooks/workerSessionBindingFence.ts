export function isCurrentWorkerSessionLookup(
  requestGeneration: number,
  requestSessionId: string,
  currentGeneration: number,
  currentSessionId: string | null,
): boolean {
  return requestGeneration === currentGeneration &&
    requestSessionId === currentSessionId;
}

export function canAdoptWorkerSessionBinding(
  requestGeneration: number,
  requestSessionId: string,
  currentGeneration: number,
  currentSessionId: string | null,
  responseSessionId: string,
): boolean {
  return isCurrentWorkerSessionLookup(
    requestGeneration,
    requestSessionId,
    currentGeneration,
    currentSessionId,
  ) && responseSessionId === requestSessionId;
}
