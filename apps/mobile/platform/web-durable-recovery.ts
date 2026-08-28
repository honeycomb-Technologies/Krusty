import {
  durableRecoveryEpochKey,
  durableRecoveryLockName,
  durableRecoveryStorageKey,
  durableRecoveryStoragePrefix,
} from "./recovery-connection-scope";

export interface SyncStringStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

interface OriginLock {
  readonly name?: string;
}

export interface OriginLockManager {
  request<T>(
    name: string,
    options: { mode: "exclusive"; ifAvailable: true },
    callback: (lock: OriginLock | null) => T | Promise<T>,
  ): Promise<T>;
}

export type StorageInvalidationSource = (
  listener: (physicalKey: string | null) => void,
) => () => void;

export class DurableRecoveryPeerTabError extends Error {
  constructor() {
    super(
      "Queued recovery for this connection is active in another tab. Use that tab or close it before sending.",
    );
    this.name = "DurableRecoveryPeerTabError";
  }
}

export class DurableRecoverySnapshotError extends Error {
  constructor() {
    super(
      "Queued recovery ownership changed. The conversation is refreshing; retry after it finishes.",
    );
    this.name = "DurableRecoverySnapshotError";
  }
}

export class DurableRecoveryConflictError extends Error {
  constructor() {
    super(
      "Queued recovery changed in another tab. The conversation is refreshing; retry after it finishes.",
    );
    this.name = "DurableRecoveryConflictError";
  }
}

export class DurableRecoveryLockUnavailableError extends Error {
  constructor() {
    super(
      "This browser cannot safely coordinate queued recovery across tabs. Use a supported browser or the native app before sending.",
    );
    this.name = "DurableRecoveryLockUnavailableError";
  }
}

function incrementDecimal(value: string | null): string {
  if (!value || !/^\d+$/.test(value)) return "1";
  const digits = value.split("").map(Number);
  for (let index = digits.length - 1; index >= 0; index -= 1) {
    if (digits[index] < 9) {
      digits[index] += 1;
      return digits.join("");
    }
    digits[index] = 0;
  }
  return `1${digits.join("")}`;
}

type OwnershipResult = "existing" | "acquired" | "busy";

/**
 * A Web Lock is retained across the full claim -> in-flight -> settlement
 * lifecycle. CAS still protects each envelope write, but ownership—not
 * last-writer-wins timing—is the authority that permits transport.
 */
export class LinearizableWebDurableRecovery {
  private readonly epochKey: string;
  private readonly lockName: string;
  private readonly storagePrefix: string;
  private readonly expectedValues = new Map<string, string | null>();
  private readonly listeners = new Set<() => void>();
  private acquisition: Promise<OwnershipResult> | null = null;
  private releaseOwner: (() => void) | null = null;
  private unsubscribeSource: (() => void) | null = null;
  private knownEpoch: string | null;
  private ownsLock = false;
  private snapshotReady = false;
  private disposed = false;

  constructor(
    private readonly connectionScope: string,
    private readonly storage: SyncStringStorage,
    private readonly lockManager: OriginLockManager | null,
    invalidationSource?: StorageInvalidationSource,
  ) {
    this.epochKey = durableRecoveryEpochKey(connectionScope);
    this.lockName = durableRecoveryLockName(connectionScope);
    this.storagePrefix = durableRecoveryStoragePrefix(connectionScope);
    this.knownEpoch = storage.getItem(this.epochKey);
    if (invalidationSource) {
      this.unsubscribeSource = invalidationSource((physicalKey) => {
        if (
          physicalKey === null || physicalKey.startsWith(this.storagePrefix)
        ) {
          this.invalidate();
        }
      });
    }
  }

  async activate(): Promise<boolean> {
    const ownership = await this.acquireOwnership();
    return ownership !== "busy";
  }

  get(logicalKey: string): string | null {
    const physicalKey = durableRecoveryStorageKey(
      this.connectionScope,
      logicalKey,
    );
    const value = this.storage.getItem(physicalKey);
    this.expectedValues.set(physicalKey, value);
    return value;
  }

  async set(logicalKey: string, value: string): Promise<void> {
    await this.requireMutationAuthority();
    const physicalKey = durableRecoveryStorageKey(
      this.connectionScope,
      logicalKey,
    );
    this.assertExpectedValue(physicalKey);
    this.bumpEpochBeforeMutation();
    try {
      this.storage.setItem(physicalKey, value);
      this.expectedValues.set(physicalKey, value);
    } catch (error) {
      this.invalidate();
      throw error;
    }
  }

  async delete(logicalKey: string): Promise<void> {
    await this.requireMutationAuthority();
    const physicalKey = durableRecoveryStorageKey(
      this.connectionScope,
      logicalKey,
    );
    this.assertExpectedValue(physicalKey);
    this.bumpEpochBeforeMutation();
    try {
      this.storage.removeItem(physicalKey);
      this.expectedValues.set(physicalKey, null);
    } catch (error) {
      this.invalidate();
      throw error;
    }
  }

  async ensureAuthority(): Promise<void> {
    const ownership = await this.acquireOwnership();
    if (ownership === "busy") throw new DurableRecoveryPeerTabError();
    if (ownership === "acquired") {
      this.invalidate();
      throw new DurableRecoverySnapshotError();
    }
    if (
      !this.snapshotReady ||
      this.storage.getItem(this.epochKey) !== this.knownEpoch
    ) {
      this.invalidate();
      throw new DurableRecoverySnapshotError();
    }
  }

  beginSnapshot(): void {
    this.expectedValues.clear();
    this.snapshotReady = false;
  }

  acknowledgeSnapshot(): void {
    this.knownEpoch = this.storage.getItem(this.epochKey);
    this.snapshotReady = this.ownsLock && !this.disposed;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.snapshotReady = false;
    this.unsubscribeSource?.();
    this.unsubscribeSource = null;
    this.releaseOwner?.();
    this.releaseOwner = null;
    this.listeners.clear();
  }

  private async requireMutationAuthority(): Promise<void> {
    await this.ensureAuthority();
  }

  private assertExpectedValue(physicalKey: string): void {
    if (
      !this.expectedValues.has(physicalKey) ||
      this.storage.getItem(physicalKey) !== this.expectedValues.get(physicalKey)
    ) {
      this.invalidate();
      throw new DurableRecoveryConflictError();
    }
  }

  private bumpEpochBeforeMutation(): void {
    const nextEpoch = incrementDecimal(this.storage.getItem(this.epochKey));
    try {
      this.storage.setItem(this.epochKey, nextEpoch);
      this.knownEpoch = nextEpoch;
    } catch (error) {
      this.invalidate();
      throw error;
    }
  }

  private invalidate(): void {
    if (this.disposed) return;
    this.snapshotReady = false;
    this.expectedValues.clear();
    for (const listener of this.listeners) {
      try {
        listener();
      } catch {
        // A UI refresh callback cannot weaken the storage ownership fence.
      }
    }
  }

  private async acquireOwnership(): Promise<OwnershipResult> {
    if (this.disposed) throw new DurableRecoverySnapshotError();
    if (this.ownsLock) return "existing";
    if (!this.lockManager) throw new DurableRecoveryLockUnavailableError();
    if (!this.acquisition) {
      this.acquisition = this.requestOwnership().finally(() => {
        this.acquisition = null;
      });
    }
    return await this.acquisition;
  }

  private requestOwnership(): Promise<OwnershipResult> {
    return new Promise<OwnershipResult>((resolve, reject) => {
      let settled = false;
      let release: (() => void) | null = null;
      const held = new Promise<void>((releaseHeld) => {
        release = releaseHeld;
      });
      const request = this.lockManager!.request(
        this.lockName,
        { mode: "exclusive", ifAvailable: true },
        async (lock) => {
          if (!lock || this.disposed) {
            settled = true;
            resolve("busy");
            return;
          }
          this.ownsLock = true;
          this.snapshotReady = false;
          this.releaseOwner = release;
          settled = true;
          resolve("acquired");
          await held;
          this.ownsLock = false;
          this.snapshotReady = false;
          this.releaseOwner = null;
        },
      );
      void request.catch((error) => {
        if (!settled) reject(error);
        else this.invalidate();
      });
    });
  }
}
