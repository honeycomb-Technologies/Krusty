import type { KrustyStorage } from '@krusty/state';

class WebStorage implements KrustyStorage {
  get(key: string): string | null {
    return localStorage.getItem(key);
  }

  set(key: string, value: string): void {
    localStorage.setItem(key, value);
  }

  delete(key: string): void {
    localStorage.removeItem(key);
  }
}

export function createStorage(): KrustyStorage {
  return new WebStorage();
}
