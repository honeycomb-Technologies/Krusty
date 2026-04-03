import * as SecureStore from './secure-store';
import type { KrustyStorage } from '@krusty/state';

class NativeStorage implements KrustyStorage {
  get(key: string): string | null {
    // SecureStore is async but KrustyStorage interface is sync.
    // Use a sync cache with async hydration for stored values.
    return this.cache.get(key) ?? null;
  }

  set(key: string, value: string): void {
    this.cache.set(key, value);
    SecureStore.setItemAsync(key, value).catch(() => {});
  }

  delete(key: string): void {
    this.cache.delete(key);
    SecureStore.deleteItemAsync(key).catch(() => {});
  }

  // In-memory cache for sync access
  private cache = new Map<string, string>();

  async hydrate(keys: string[]): Promise<void> {
    for (const key of keys) {
      const value = await SecureStore.getItemAsync(key);
      if (value !== null) this.cache.set(key, value);
    }
  }
}

export function createStorage(): KrustyStorage {
  return new NativeStorage();
}
