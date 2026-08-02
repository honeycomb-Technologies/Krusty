import type { MitsuroStorage } from '@mitsuro/state';
import {
  deleteMigratedSyncValue,
  migrationForCanonicalKey,
  readMigratedSyncValue,
  writeCanonicalSyncValue,
} from './identity-storage';

class WebStorage implements MitsuroStorage {
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
}

export function createStorage(): MitsuroStorage {
  return new WebStorage();
}
