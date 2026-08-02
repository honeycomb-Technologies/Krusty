import {
  canonicalNotificationAction,
  canonicalNotificationData,
  HIVE_NOTIFICATION_ACTION,
  connectionFromInjectedGlobals,
  parseConnectionLaunchUrl,
} from "../platform/identity-compatibility";
import {
  IDENTITY_STORAGE_KEYS,
  readConnectionCredentials,
  readMigratedAsyncValue,
  readMigratedSyncValue,
  writeConnectionCredentials,
} from "../platform/identity-storage";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

class AsyncStorageFixture {
  readonly values = new Map<string, string>();
  failKey: string | null = null;

  async getItem(key: string): Promise<string | null> {
    return this.values.get(key) ?? null;
  }

  async setItem(key: string, value: string): Promise<void> {
    if (this.failKey === key) {
      throw new Error("write failed");
    }
    this.values.set(key, value);
  }

  async removeItem(key: string): Promise<void> {
    this.values.delete(key);
  }
}

class SyncStorageFixture {
  readonly values = new Map<string, string>();
  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
  removeItem(key: string): void {
    this.values.delete(key);
  }
}

Deno.test("persisted identity migration writes canonical and retains prior state", async () => {
  const storage = new AsyncStorageFixture();
  const key = IDENTITY_STORAGE_KEYS.serverUrl;
  storage.values.set(key.legacy[0], "https://old.example");

  assert(
    await readMigratedAsyncValue(storage, key) === "https://old.example",
    "the prior value should remain readable",
  );
  assert(
    storage.values.get(key.canonical) === "https://old.example",
    "the prior value should be copied to the canonical key",
  );
  assert(
    storage.values.get(key.legacy[0]) === "https://old.example",
    "the prior key must remain available to an older installed client",
  );

  const failed = new AsyncStorageFixture();
  failed.values.set(key.legacy[0], "https://recover.example");
  failed.failKey = IDENTITY_STORAGE_KEYS.serverUrl.canonical;
  let rejected = false;
  try {
    await readMigratedAsyncValue(failed, key);
  } catch {
    rejected = true;
  }
  assert(rejected, "a canonical write failure must surface");
  assert(
    failed.values.get(key.legacy[0]) === "https://recover.example",
    "a failed canonical write must not delete recoverable prior state",
  );
});

Deno.test("connection migration commits one pair and retains rollback keys", async () => {
  const storage = new AsyncStorageFixture();
  const oldUrlKey = IDENTITY_STORAGE_KEYS.serverUrl.legacy[0];
  const oldTokenKey = IDENTITY_STORAGE_KEYS.serverToken.legacy[0];
  storage.values.set(oldUrlKey, "https://old.example");
  storage.values.set(oldTokenKey, "old-token");

  const migrated = await readConnectionCredentials(storage);
  assert(migrated?.serverUrl === "https://old.example", "the complete old URL must migrate");
  assert(migrated?.token === "old-token", "the matching old token must migrate");
  assert(
    storage.values.has(IDENTITY_STORAGE_KEYS.serverConnection.canonical),
    "the pair must be committed in one canonical value",
  );
  assert(
    storage.values.get(oldUrlKey) === "https://old.example" &&
      storage.values.get(oldTokenKey) === "old-token",
    "the complete old pair must remain available for rollback",
  );
});

Deno.test("connection migration never combines split generations", async () => {
  const storage = new AsyncStorageFixture();
  storage.values.set(IDENTITY_STORAGE_KEYS.serverUrl.canonical, "https://partial-new.example");
  storage.values.set(IDENTITY_STORAGE_KEYS.serverUrl.legacy[0], "https://complete-old.example");
  storage.values.set(IDENTITY_STORAGE_KEYS.serverToken.legacy[0], "complete-old-token");

  const migrated = await readConnectionCredentials(storage);
  assert(
    migrated?.serverUrl === "https://complete-old.example" &&
      migrated.token === "complete-old-token",
    "an incomplete canonical generation must not borrow a token from the old generation",
  );

  const failed = new AsyncStorageFixture();
  failed.values.set(IDENTITY_STORAGE_KEYS.serverUrl.legacy[0], "https://recover.example");
  failed.values.set(IDENTITY_STORAGE_KEYS.serverToken.legacy[0], "recover-token");
  failed.failKey = IDENTITY_STORAGE_KEYS.serverConnection.canonical;
  let rejected = false;
  try {
    await readConnectionCredentials(failed);
  } catch {
    rejected = true;
  }
  assert(rejected, "a failed atomic commit must surface");
  assert(
    failed.values.get(IDENTITY_STORAGE_KEYS.serverToken.legacy[0]) === "recover-token",
    "a failed commit must leave the rollback pair intact",
  );
});

Deno.test("connection writes replace the committed pair with one operation", async () => {
  const storage = new AsyncStorageFixture();
  await writeConnectionCredentials(storage, {
    serverUrl: "https://new.example",
    token: "new-token",
  });
  const restored = await readConnectionCredentials(storage);
  assert(
    restored?.serverUrl === "https://new.example" && restored.token === "new-token",
    "the committed pair must round-trip together",
  );
});

Deno.test("workspace migration prefers canonical and upgrades the old Hive slot", () => {
  const storage = new SyncStorageFixture();
  const key = IDENTITY_STORAGE_KEYS.workspaceHive;
  storage.values.set(key.legacy[0], "prior-hive-workspace");
  assert(
    readMigratedSyncValue(storage, key) === "prior-hive-workspace",
    "the old autonomous workspace should hydrate the Hive slot",
  );
  assert(storage.values.get(key.canonical) === "prior-hive-workspace", "Hive state must be canonicalized");

  storage.values.set(key.canonical, "canonical-wins");
  storage.values.set(key.legacy[0], "must-not-overwrite");
  assert(
    readMigratedSyncValue(storage, key) === "canonical-wins",
    "canonical data must never be overwritten by an old key",
  );
});

Deno.test("canonical and transition launch URLs resolve to the same connection", () => {
  const canonical = parseConnectionLaunchUrl(
    "mitsuro://connect?url=https%3A%2F%2Fdevice.example&token=secret",
  );
  const legacy = parseConnectionLaunchUrl(
    "krusty://connect?url=https%3A%2F%2Fdevice.example&token=secret",
  );
  const legacyHash = parseConnectionLaunchUrl(
    "https://device.example/#krusty-remote-token=secret",
  );
  assert(canonical?.serverUrl === "https://device.example", "canonical scheme must parse");
  assert(JSON.stringify(legacy) === JSON.stringify(canonical), "old scheme must map to canonical connection data");
  assert(JSON.stringify(legacyHash) === JSON.stringify(canonical), "old hash token must remain readable");
});

Deno.test("desktop globals support both shells without mixing generations", () => {
  const canonical = connectionFromInjectedGlobals({
    __MITSURO_SERVER_URL: "http://canonical.example",
    __MITSURO_SERVER_TOKEN: "canonical-token",
    __KRUSTY_SERVER_URL: "http://legacy.example",
    __KRUSTY_SERVER_TOKEN: "legacy-token",
  });
  assert(canonical?.serverUrl === "http://canonical.example", "canonical globals must win as a pair");

  const legacy = connectionFromInjectedGlobals({
    __MITSURO_SERVER_URL: "http://incomplete.example",
    __KRUSTY_SERVER_URL: "http://legacy.example",
    __KRUSTY_SERVER_TOKEN: "legacy-token",
  });
  assert(
    legacy?.serverUrl === "http://legacy.example" && legacy.token === "legacy-token",
    "an incomplete canonical pair must fall back to one complete old generation",
  );
});

Deno.test("old Hive push actions keep their session and open canonical Hive focus", () => {
  const action = canonicalNotificationAction("OPEN_MAKO");
  const data = canonicalNotificationData({
    type: "mako_update",
    focus: "mako",
    sessionId: "session-42",
  });
  assert(action === HIVE_NOTIFICATION_ACTION, "old action must normalize to OPEN_HIVE");
  assert(data.type === "hive_update", "old payload type must normalize");
  assert(data.focus === "hive", "old focus must navigate to Hive");
  assert(data.sessionId === "session-42", "the target session must be preserved");
});

Deno.test("native diagnostics supports both module generations and imports old files", async () => {
  const moduleSource = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-diagnostics/src/MitsuroDiagnosticsModule.ts",
      import.meta.url,
    ).pathname,
  );
  const configSource = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-diagnostics/expo-module.config.json",
      import.meta.url,
    ).pathname,
  );
  const nativeSource = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-diagnostics/ios/MitsuroDiagnosticsModule.swift",
      import.meta.url,
    ).pathname,
  );
  assert(
    moduleSource.includes("?? requireOptionalNativeModule") &&
      configSource.includes("KrustyDiagnosticsCompatibilityModule"),
    "new JS and new native builds must each bridge the other generation",
  );
  assert(
    nativeSource.includes("migrateLegacyPayloadsIfNeeded()") &&
      nativeSource.includes("legacyMetricKitPayloadDirectory()"),
    "MetricKit payloads must be imported before canonical reads and writes",
  );
  const migrationStart = nativeSource.indexOf(
    "private func migrateLegacyPayloadsIfNeeded()",
  );
  const migrationEnd = nativeSource.indexOf(
    "private func payloadDirectory()",
    migrationStart,
  );
  const migrationBody = nativeSource.slice(migrationStart, migrationEnd);
  assert(
    migrationStart >= 0 && migrationEnd > migrationStart &&
      migrationBody.includes("copyItem(at: source, to: destination)") &&
      !migrationBody.includes("moveItem(at: source") &&
      !migrationBody.includes("removeItem(at: source") &&
      nativeSource.includes("var seenIDs = Set<String>()") &&
      nativeSource.includes("if records.count == maxStoredPayloads"),
    "legacy MetricKit imports must retain rollback files while canonical reads stay deduplicated and bounded",
  );
});

Deno.test("APNs registration uses the installed binary bundle identifier", async () => {
  const notifications = await Deno.readTextFile(
    new URL("../hooks/useNotifications.tsx", import.meta.url).pathname,
  );
  assert(
    notifications.includes('Application = require("expo-application")') &&
      notifications.includes("Application?.applicationId?.trim()") &&
      notifications.includes("nativeDeviceToken,\n              runtimeBundleId,"),
    "direct APNs registration must send the runtime application ID instead of a renamed default",
  );
});

Deno.test("profiling build flags accept the prior environment prefix during the bridge", async () => {
  const layout = await Deno.readTextFile(
    new URL("../app/_layout.tsx", import.meta.url).pathname,
  );
  const probe = await Deno.readTextFile(
    new URL("../diagnostics/jsHotPathProbe.ts", import.meta.url).pathname,
  );
  assert(
    layout.includes("EXPO_PUBLIC_MITSURO_PERFORMANCE") &&
      layout.includes("EXPO_PUBLIC_KRUSTY_PERFORMANCE") &&
      probe.includes("EXPO_PUBLIC_MITSURO_JS_HOTPATH_PROBE") &&
      probe.includes("EXPO_PUBLIC_KRUSTY_JS_HOTPATH_PROBE"),
    "existing build automation must keep its profiling toggles for the bridge release",
  );
});

Deno.test("Hive widget keeps the prior native kind through OTA skew", async () => {
  const config = JSON.parse(
    await Deno.readTextFile(new URL("../app.json", import.meta.url).pathname),
  );
  const compatibilityWidget = await Deno.readTextFile(
    new URL("../widgets/MakoWidget.tsx", import.meta.url).pathname,
  );
  const sync = await Deno.readTextFile(
    new URL("../hooks/useWidgetSync.ts", import.meta.url).pathname,
  );
  const names = config.expo.plugins
    .find((plugin: unknown) => Array.isArray(plugin) && plugin[0] === "expo-widgets")?.[1]
    ?.widgets?.map((widget: { name: string }) => widget.name) ?? [];
  assert(
    names.includes("HiveWidget") && names.includes("MakoWidget"),
    "the native build must contain both widget kinds during the transition",
  );
  assert(
    compatibilityWidget.includes('createWidget("MakoWidget", HiveWidgetView)') &&
      sync.includes('require("../widgets/HiveWidget")') &&
      sync.includes('require("../widgets/MakoWidget")'),
    "new JS must update both canonical and prior installed widget kinds",
  );
});
