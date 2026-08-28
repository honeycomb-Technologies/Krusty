export type WorkerInputOperation = "chat" | "steer";

interface PendingWorkerInput {
  sessionId: string;
  operation: WorkerInputOperation;
  fingerprint: string;
  key: string;
}

function createWorkerInputKey(): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  if (typeof randomUuid === "string" && randomUuid.length > 0) {
    return `worker-input-${randomUuid}`;
  }
  return [
    "worker-input",
    Date.now().toString(36),
    Math.random().toString(36).slice(2, 10),
  ].join("-");
}

/**
 * Keeps bounded exact pending direct-Worker input retry identities. A changed
 * operation, session, or canonical request fingerprint is a new user intent.
 * An unchanged identity survives failures until exact acceptance or an
 * explicit store cleanup; navigation and intervening intents are not proof
 * that an uncertain request was rejected remotely.
 */
export class WorkerInputIdempotency {
  private static readonly MAX_PENDING = 16;
  /** Indexed by exact server key so identical distinct turns can coexist. */
  private readonly pending = new Map<string, PendingWorkerInput>();

  private pendingFor(
    sessionId: string,
    operation: WorkerInputOperation,
    fingerprint: string,
  ): PendingWorkerInput | undefined {
    return [...this.pending.values()].find((pending) =>
      pending.sessionId === sessionId && pending.operation === operation &&
      pending.fingerprint === fingerprint
    );
  }

  keyFor(
    sessionId: string,
    operation: WorkerInputOperation,
    fingerprint: string,
  ): string {
    const pending = this.pendingFor(sessionId, operation, fingerprint);
    if (pending) {
      return pending.key;
    }

    return this.reserve(sessionId, operation, fingerprint);
  }

  /** Reserve a new user-turn identity even when its payload is identical. */
  reserve(
    sessionId: string,
    operation: WorkerInputOperation,
    fingerprint: string,
  ): string {
    if (this.pending.size >= WorkerInputIdempotency.MAX_PENDING) {
      throw new Error(
        "Too many Worker inputs are awaiting exact delivery. Reopen or discard an earlier Worker conversation first.",
      );
    }

    let key = createWorkerInputKey();
    while (this.pending.has(key)) key = createWorkerInputKey();
    this.pending.set(key, { sessionId, operation, fingerprint, key });
    return key;
  }

  restore(
    sessionId: string,
    operation: WorkerInputOperation,
    fingerprint: string,
    key: string,
  ): void {
    if (!key.trim()) return;
    const restored = this.pending.get(key);
    if (restored) {
      if (
        restored.sessionId !== sessionId || restored.operation !== operation ||
        restored.fingerprint !== fingerprint
      ) {
        throw new Error(
          "A Worker input key was reused for a different request.",
        );
      }
      return;
    }
    if (this.pending.size >= WorkerInputIdempotency.MAX_PENDING) {
      throw new Error(
        "Too many Worker inputs are awaiting exact delivery. Reopen or discard an earlier Worker conversation first.",
      );
    }
    this.pending.set(key, { sessionId, operation, fingerprint, key });
  }

  accept(
    sessionId: string,
    operation: WorkerInputOperation,
    key: string,
  ): void {
    const pending = this.pending.get(key);
    if (
      pending?.sessionId === sessionId && pending.operation === operation
    ) {
      this.pending.delete(key);
    }
  }

  transitionTo(_sessionId: string | null): void {
    // Navigation is not proof that a transport attempt was rejected. Retain
    // each bounded per-session identity until exact acceptance or cleanup so
    // reopening a Worker cannot duplicate an uncertain queued successor.
  }

  discardSession(sessionId: string): void {
    for (const [key, pending] of this.pending) {
      if (pending.sessionId === sessionId) {
        this.pending.delete(key);
      }
    }
  }

  clear(): void {
    this.pending.clear();
  }
}
