import * as SecureStore from './secure-store';
import type { MitsuroStorage } from '@mitsuro/state';
import {
  deleteMigratedAsyncValue,
  migrationForCanonicalKey,
  readMigratedAsyncValue,
  writeCanonicalAsyncValue,
} from './identity-storage';

class NativeStorage implements MitsuroStorage {
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
}

export function createStorage(): MitsuroStorage {
  return new NativeStorage();
}
