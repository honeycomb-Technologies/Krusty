export interface HiveGroupMessageRequest {
  message: string;
}

export interface HiveGroupMessageAttempt {
  readonly groupId: string;
  readonly requestFingerprint: string;
  readonly idempotencyKey: string;
}

type IdempotencyKeyFactory = (groupId: string) => string;

function createHiveGroupMessageIdempotencyKey(groupId: string): string {
  const nonce = globalThis.crypto?.randomUUID?.() ??
    `${Date.now()}:${Math.random().toString(36).slice(2)}`;
  return `group-send:${groupId}:${nonce}`;
}

function requestFingerprint(request: HiveGroupMessageRequest): string {
  return JSON.stringify(request);
}

export function sameHiveGroupMessageAttempt(
  left: HiveGroupMessageAttempt | null,
  right: HiveGroupMessageAttempt,
): boolean {
  return left?.groupId === right.groupId &&
    left.requestFingerprint === right.requestFingerprint &&
    left.idempotencyKey === right.idempotencyKey;
}

/**
 * Reuse an uncertain attempt only while its exact group and request body match.
 * A changed destination or body is a new user intent and receives a new key.
 */
export function retainHiveGroupMessageAttempt(
  current: HiveGroupMessageAttempt | null,
  groupId: string,
  request: HiveGroupMessageRequest,
  createKey: IdempotencyKeyFactory = createHiveGroupMessageIdempotencyKey,
): HiveGroupMessageAttempt {
  const fingerprint = requestFingerprint(request);
  if (
    current?.groupId === groupId &&
    current.requestFingerprint === fingerprint
  ) {
    return current;
  }
  return {
    groupId,
    requestFingerprint: fingerprint,
    idempotencyKey: createKey(groupId),
  };
}

/**
 * Clear only the attempt whose HTTP response was accepted. An older response
 * racing a newer send must never discard the newer send's replay identity.
 */
export function clearAcceptedHiveGroupMessageAttempt(
  current: HiveGroupMessageAttempt | null,
  accepted: HiveGroupMessageAttempt,
): HiveGroupMessageAttempt | null {
  return sameHiveGroupMessageAttempt(current, accepted) ? null : current;
}
