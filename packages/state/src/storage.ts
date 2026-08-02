export interface MitsuroStorage {
  get(key: string): string | null;
  set(key: string, value: string): void;
  delete(key: string): void;
  hydrate?(keys: string[]): Promise<void>;
}

export class MemoryStorage implements MitsuroStorage {
  private data = new Map<string, string>();
  get(key: string) { return this.data.get(key) ?? null; }
  set(key: string, value: string) { this.data.set(key, value); }
  delete(key: string) { this.data.delete(key); }
}
