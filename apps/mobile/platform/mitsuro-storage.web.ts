import type { MitsuroStorage } from "@mitsuro/state";
import {
  deleteMigratedSyncValue,
  migrationForCanonicalKey,
  readMigratedSyncValue,
  writeCanonicalSyncValue,
} from "./identity-storage";
import {
  LinearizableWebDurableRecovery,
  type OriginLockManager,
  type StorageInvalidationSource,
} from "./web-durable-recovery";

class WebStorage implements MitsuroStorage {
  private readonly durableRecovery: LinearizableWebDurableRecovery;

  constructor(readonly durableRecoveryNamespace: string) {
    const lockManager = typeof navigator === "undefined"
      ? null
      : (navigator as Navigator & { locks?: OriginLockManager }).locks ?? null;
    const invalidationSource: StorageInvalidationSource | undefined =
      typeof globalThis.addEventListener !== "function"
        ? undefined
        : (listener) => {
          const onStorage = (event: StorageEvent) => {
            if (event.storageArea === localStorage) listener(event.key);
          };
          globalThis.addEventListener("storage", onStorage);
          return () => globalThis.removeEventListener("storage", onStorage);
        };
    this.durableRecovery = new LinearizableWebDurableRecovery(
      this.durableRecoveryNamespace,
      {
        getItem: (key) => localStorage.getItem(key),
        setItem: (key, value) => localStorage.setItem(key, value),
        removeItem: (key) => localStorage.removeItem(key),
      },
      lockManager,
      invalidationSource,
    );
    // Do not adopt legacy unscoped recovery records here: after a historical
    // connection switch their principal owner cannot be proven safely.
  }

  get(key: string): string | null {
    const migration = migrationForCanonicalKey(key);
    return migration
      ? readMigratedSyncValue(localStorage, migration)
      : localStorage.getItem(key);
  }

  set(key: string, value: string): void {
    const migration = migrationForCanonicalKey(key);
    if (migration) writeCanonicalSyncValue(localStorage, migration, value);
    else localStorage.setItem(key, value);
  }

  delete(key: string): void {
    const migration = migrationForCanonicalKey(key);
    if (migration) deleteMigratedSyncValue(localStorage, migration);
    else localStorage.removeItem(key);
  }

  getDurable(key: string): Promise<string | null> {
    return Promise.resolve(this.durableRecovery.get(key));
  }

  getDurableSync(key: string): string | null {
    return this.durableRecovery.get(key);
  }

  async setDurable(key: string, value: string): Promise<void> {
    await this.durableRecovery.set(key, value);
  }

  async deleteDurable(key: string): Promise<void> {
    await this.durableRecovery.delete(key);
  }

  activateDurableRecovery(): Promise<boolean> {
    return this.durableRecovery.activate();
  }

  ensureDurableRecoveryAuthority(): Promise<void> {
    return this.durableRecovery.ensureAuthority();
  }

  beginDurableRecoverySnapshot(): void {
    this.durableRecovery.beginSnapshot();
  }

  acknowledgeDurableRecoverySnapshot(): void {
    this.durableRecovery.acknowledgeSnapshot();
  }

  subscribeDurableRecoveryInvalidation(listener: () => void): () => void {
    return this.durableRecovery.subscribe(listener);
  }

  disposeDurableRecovery(): void {
    this.durableRecovery.dispose();
  }
}

export function createStorage(connectionScope: string): MitsuroStorage {
  return new WebStorage(connectionScope);
}
