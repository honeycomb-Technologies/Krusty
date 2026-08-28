import AsyncStorage from "@react-native-async-storage/async-storage";
import * as SecureStore from "./secure-store";
import type { MitsuroStorage } from "@mitsuro/state";
import {
  deleteMigratedAsyncValue,
  migrationForCanonicalKey,
  readMigratedAsyncValue,
  writeCanonicalAsyncValue,
} from "./identity-storage";
import { durableRecoveryStorageKey } from "./recovery-connection-scope";

class NativeStorage implements MitsuroStorage {
  constructor(readonly durableRecoveryNamespace: string) {}

  get(key: string): string | null {
    // SecureStore is async but MitsuroStorage interface is sync.
    // Use a sync cache with async hydration for stored values.
    return this.cache.get(key) ?? null;
  }

  set(key: string, value: string): void {
    this.cache.set(key, value);
    const migration = migrationForCanonicalKey(key);
    const write = migration
      ? writeCanonicalAsyncValue(SecureStore, migration, value)
      : SecureStore.setItemAsync(key, value);
    write.catch(() => {});
  }

  delete(key: string): void {
    this.cache.delete(key);
    const migration = migrationForCanonicalKey(key);
    const deletion = migration
      ? deleteMigratedAsyncValue(SecureStore, migration)
      : SecureStore.deleteItemAsync(key);
    deletion.catch(() => {});
  }

  // In-memory cache for sync access
  private cache = new Map<string, string>();

  async hydrate(keys: string[]): Promise<void> {
    for (const key of keys) {
      const migration = migrationForCanonicalKey(key);
      const value = migration
        ? await readMigratedAsyncValue(SecureStore, migration)
        : await SecureStore.getItemAsync(key);
      if (value !== null) this.cache.set(key, value);
    }
  }

  getDurable(key: string): Promise<string | null> {
    // Legacy unscoped records are intentionally not migrated because their
    // historical connection principal cannot be established.
    return AsyncStorage.getItem(
      durableRecoveryStorageKey(this.durableRecoveryNamespace, key),
    );
  }

  setDurable(key: string, value: string): Promise<void> {
    return AsyncStorage.setItem(
      durableRecoveryStorageKey(this.durableRecoveryNamespace, key),
      value,
    );
  }

  deleteDurable(key: string): Promise<void> {
    return AsyncStorage.removeItem(
      durableRecoveryStorageKey(this.durableRecoveryNamespace, key),
    );
  }
}

export function createStorage(connectionScope: string): MitsuroStorage {
  return new NativeStorage(connectionScope);
}
