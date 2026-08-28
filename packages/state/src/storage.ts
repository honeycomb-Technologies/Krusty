export interface MitsuroStorage {
  get(key: string): string | null;
  set(key: string, value: string): void;
  delete(key: string): void;
  hydrate?(keys: string[]): Promise<void>;
  /**
   * Opaque identity of the physical durable-recovery authority. Adapter
   * instances for one connection scope share it; different principals must
   * not. Callers must not derive or expose credentials through this value.
   */
  readonly durableRecoveryNamespace?: string;
  /**
   * Large, non-secret recovery records such as interrupted drafts. Native
   * clients back these with AsyncStorage rather than credential storage.
   */
  getDurable?(key: string): Promise<string | null>;
  /** Optional synchronous mirror used by web/tests to avoid a first-send tick. */
  getDurableSync?(key: string): string | null;
  setDurable?(key: string, value: string): Promise<void>;
  deleteDurable?(key: string): Promise<void>;
  /**
   * Web clients use an origin-wide owner for recovery transitions. A second
   * tab may inspect recovery state, but it must fail closed before either a
   * durable mutation or its matching transport can begin.
   */
  activateDurableRecovery?(): Promise<boolean>;
  ensureDurableRecoveryAuthority?(): Promise<void>;
  beginDurableRecoverySnapshot?(): void;
  acknowledgeDurableRecoverySnapshot?(): void;
  subscribeDurableRecoveryInvalidation?(listener: () => void): () => void;
  disposeDurableRecovery?(): void;
}

export class MemoryStorage implements MitsuroStorage {
  private data = new Map<string, string>();
  get(key: string) {
    return this.data.get(key) ?? null;
  }
  set(key: string, value: string) {
    this.data.set(key, value);
  }
  delete(key: string) {
    this.data.delete(key);
  }
  getDurable(key: string): Promise<string | null> {
    return Promise.resolve(this.get(key));
  }
  getDurableSync(key: string) {
    return this.get(key);
  }
  setDurable(key: string, value: string): Promise<void> {
    this.set(key, value);
    return Promise.resolve();
  }
  deleteDurable(key: string): Promise<void> {
    this.delete(key);
    return Promise.resolve();
  }
}
