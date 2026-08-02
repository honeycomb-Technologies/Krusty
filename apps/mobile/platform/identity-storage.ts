export type MigratedStorageKey = Readonly<{
  canonical: string;
  legacy: readonly string[];
}>;

/**
 * The only client-side map from the former identity to Mitsuro/Hive storage.
 * Readers prefer the canonical key and copy a prior value only when necessary.
 * Prior keys remain available to an older installed client for the announced
 * compatibility window; only an explicit delete clears both generations.
 */
export const IDENTITY_STORAGE_KEYS = {
  serverConnection: {
    canonical: "mitsuro_server_connection_v1",
    legacy: [],
  },
  serverUrl: {
    canonical: "mitsuro_server_url",
    legacy: ["krusty_server_url"],
  },
  serverToken: {
    canonical: "mitsuro_server_token",
    legacy: ["krusty_server_token"],
  },
  workspaceCode: {
    canonical: "mitsuro:workspace",
    legacy: ["krusty:workspace"],
  },
  workspaceChat: {
    canonical: "mitsuro:workspace:chat",
    legacy: ["krusty:workspace:chat"],
  },
  workspaceHive: {
    canonical: "mitsuro:workspace:hive",
    legacy: [
      "krusty:workspace:mako",
      "krusty:workspace:hive",
      "mitsuro:workspace:mako",
    ],
  },
  permissionMode: {
    canonical: "mitsuro-permission-mode",
    legacy: ["krusty-permission-mode"],
  },
  presenceClientId: {
    canonical: "mitsuro:presence-client-id",
    legacy: ["krusty:presence-client-id"],
  },
  selectedModel: {
    canonical: "mitsuro_selected_model",
    legacy: ["krusty_selected_model"],
  },
  providerFilterOrder: {
    canonical: "mitsuro-provider-filter-order-v1",
    legacy: ["krusty-provider-filter-order-v1"],
  },
  pushToken: {
    canonical: "mitsuro_push_token",
    legacy: ["krusty_push_token"],
  },
  notificationLevel: {
    canonical: "mitsuro_notification_level",
    legacy: ["krusty_notification_level"],
  },
  pendingNotificationActions: {
    canonical: "mitsuro_pending_notification_actions_v1",
    legacy: ["krusty_pending_notification_actions_v1"],
  },
  handledNotificationActions: {
    canonical: "mitsuro_handled_notification_actions_v1",
    legacy: ["krusty_handled_notification_actions_v1"],
  },
  previewTabs: {
    canonical: "mitsuro_preview_tabs_v1",
    legacy: ["krusty_preview_tabs_v1"],
  },
  diagnosticsInstallation: {
    canonical: "mitsuro:diagnostics:installation-v1",
    legacy: ["krusty:diagnostics:installation-v1"],
  },
  diagnosticsPending: {
    canonical: "mitsuro:diagnostics:pending-v1",
    legacy: ["krusty:diagnostics:pending-v1"],
  },
} as const satisfies Record<string, MigratedStorageKey>;

export const APP_STORE_MIGRATIONS = [
  IDENTITY_STORAGE_KEYS.workspaceCode,
  IDENTITY_STORAGE_KEYS.workspaceChat,
  IDENTITY_STORAGE_KEYS.workspaceHive,
  IDENTITY_STORAGE_KEYS.permissionMode,
  IDENTITY_STORAGE_KEYS.presenceClientId,
] as const;

export interface AsyncKeyValueStorage {
  getItemAsync?(key: string): Promise<string | null>;
  setItemAsync?(key: string, value: string): Promise<void>;
  deleteItemAsync?(key: string): Promise<void>;
  getItem?(key: string): Promise<string | null>;
  setItem?(key: string, value: string): Promise<void>;
  removeItem?(key: string): Promise<void>;
}

export type ConnectionCredentials = Readonly<{
  serverUrl: string;
  token: string;
}>;

type StoredConnectionCredentialsV1 = {
  version: 1;
  server_url: string;
  token: string;
};

async function asyncGet(
  storage: AsyncKeyValueStorage,
  key: string,
): Promise<string | null> {
  if (storage.getItemAsync) return storage.getItemAsync(key);
  if (storage.getItem) return storage.getItem(key);
  throw new Error("Storage adapter does not implement a get operation");
}

async function asyncSet(
  storage: AsyncKeyValueStorage,
  key: string,
  value: string,
): Promise<void> {
  if (storage.setItemAsync) return storage.setItemAsync(key, value);
  if (storage.setItem) return storage.setItem(key, value);
  throw new Error("Storage adapter does not implement a set operation");
}

async function asyncDelete(
  storage: AsyncKeyValueStorage,
  key: string,
): Promise<void> {
  if (storage.deleteItemAsync) return storage.deleteItemAsync(key);
  if (storage.removeItem) return storage.removeItem(key);
  throw new Error("Storage adapter does not implement a delete operation");
}

async function removeLegacyAsync(
  storage: AsyncKeyValueStorage,
  key: MigratedStorageKey,
): Promise<void> {
  await Promise.all(key.legacy.map((legacy) => asyncDelete(storage, legacy).catch(() => {})));
}

function encodeConnectionCredentials(
  credentials: ConnectionCredentials,
): string {
  const stored: StoredConnectionCredentialsV1 = {
    version: 1,
    server_url: credentials.serverUrl,
    token: credentials.token,
  };
  return JSON.stringify(stored);
}

function decodeConnectionCredentials(
  value: string,
): ConnectionCredentials {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    throw new Error("Stored Mitsuro connection is not valid JSON");
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Stored Mitsuro connection has an invalid shape");
  }
  const record = parsed as Partial<StoredConnectionCredentialsV1>;
  if (
    record.version !== 1 ||
    typeof record.server_url !== "string" ||
    !record.server_url ||
    typeof record.token !== "string" ||
    !record.token
  ) {
    throw new Error("Stored Mitsuro connection is incomplete or unsupported");
  }
  return { serverUrl: record.server_url, token: record.token };
}

async function readConnectionPair(
  storage: AsyncKeyValueStorage,
  serverUrlKey: string,
  tokenKey: string,
): Promise<ConnectionCredentials | null> {
  const [serverUrl, token] = await Promise.all([
    asyncGet(storage, serverUrlKey),
    asyncGet(storage, tokenKey),
  ]);
  return serverUrl && token ? { serverUrl, token } : null;
}

/**
 * Read the connection as one committed record.
 *
 * Split-key generations are considered only as complete pairs and are copied
 * into one canonical value with a single storage write. Their source keys are
 * deliberately retained so an older installed client still has rollback data.
 */
export async function readConnectionCredentials(
  storage: AsyncKeyValueStorage,
): Promise<ConnectionCredentials | null> {
  const committed = await asyncGet(
    storage,
    IDENTITY_STORAGE_KEYS.serverConnection.canonical,
  );
  if (committed !== null) return decodeConnectionCredentials(committed);

  const canonicalPair = await readConnectionPair(
    storage,
    IDENTITY_STORAGE_KEYS.serverUrl.canonical,
    IDENTITY_STORAGE_KEYS.serverToken.canonical,
  );
  const legacyPair = canonicalPair ?? await readConnectionPair(
    storage,
    IDENTITY_STORAGE_KEYS.serverUrl.legacy[0],
    IDENTITY_STORAGE_KEYS.serverToken.legacy[0],
  );
  if (!legacyPair) return null;

  await asyncSet(
    storage,
    IDENTITY_STORAGE_KEYS.serverConnection.canonical,
    encodeConnectionCredentials(legacyPair),
  );
  return legacyPair;
}

/** Persist URL and token atomically as one versioned storage value. */
export async function writeConnectionCredentials(
  storage: AsyncKeyValueStorage,
  credentials: ConnectionCredentials,
): Promise<void> {
  if (!credentials.serverUrl || !credentials.token) {
    throw new Error("A complete Mitsuro connection is required");
  }
  await asyncSet(
    storage,
    IDENTITY_STORAGE_KEYS.serverConnection.canonical,
    encodeConnectionCredentials(credentials),
  );
}

/** Explicit disconnect removes both the committed record and transition data. */
export async function deleteConnectionCredentials(
  storage: AsyncKeyValueStorage,
): Promise<void> {
  const keys = [
    IDENTITY_STORAGE_KEYS.serverConnection.canonical,
    IDENTITY_STORAGE_KEYS.serverUrl.canonical,
    IDENTITY_STORAGE_KEYS.serverToken.canonical,
    ...IDENTITY_STORAGE_KEYS.serverUrl.legacy,
    ...IDENTITY_STORAGE_KEYS.serverToken.legacy,
  ];
  await Promise.all(keys.map((key) => asyncDelete(storage, key).catch(() => {})));
}

export async function readMigratedAsyncValue(
  storage: AsyncKeyValueStorage,
  key: MigratedStorageKey,
): Promise<string | null> {
  const canonical = await asyncGet(storage, key.canonical);
  if (canonical !== null) return canonical;

  for (const legacy of key.legacy) {
    const value = await asyncGet(storage, legacy);
    if (value === null) continue;
    await asyncSet(storage, key.canonical, value);
    return value;
  }
  return null;
}

export async function writeCanonicalAsyncValue(
  storage: AsyncKeyValueStorage,
  key: MigratedStorageKey,
  value: string,
): Promise<void> {
  await asyncSet(storage, key.canonical, value);
}

export async function deleteMigratedAsyncValue(
  storage: AsyncKeyValueStorage,
  key: MigratedStorageKey,
): Promise<void> {
  await asyncDelete(storage, key.canonical).catch(() => {});
  await removeLegacyAsync(storage, key);
}

export interface SyncKeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export function readMigratedSyncValue(
  storage: SyncKeyValueStorage,
  key: MigratedStorageKey,
): string | null {
  const canonical = storage.getItem(key.canonical);
  if (canonical !== null) return canonical;

  for (const legacy of key.legacy) {
    const value = storage.getItem(legacy);
    if (value === null) continue;
    storage.setItem(key.canonical, value);
    return value;
  }
  return null;
}

export function writeCanonicalSyncValue(
  storage: SyncKeyValueStorage,
  key: MigratedStorageKey,
  value: string,
): void {
  storage.setItem(key.canonical, value);
}

export function deleteMigratedSyncValue(
  storage: SyncKeyValueStorage,
  key: MigratedStorageKey,
): void {
  storage.removeItem(key.canonical);
  for (const legacy of key.legacy) storage.removeItem(legacy);
}

export function migrationForCanonicalKey(
  canonical: string,
): MigratedStorageKey | undefined {
  return Object.values(IDENTITY_STORAGE_KEYS).find(
    (migration) => migration.canonical === canonical,
  );
}
