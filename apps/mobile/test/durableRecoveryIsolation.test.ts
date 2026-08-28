import {
  deriveRecoveryConnectionScope,
  durableRecoveryEpochKey,
  durableRecoveryStorageKey,
  sha256Hex,
} from "../platform/recovery-connection-scope.ts";
import {
  DurableRecoveryConflictError,
  DurableRecoveryLockUnavailableError,
  DurableRecoveryPeerTabError,
  DurableRecoverySnapshotError,
  LinearizableWebDurableRecovery,
  type OriginLockManager,
  type StorageInvalidationSource,
  type SyncStringStorage,
} from "../platform/web-durable-recovery.ts";
import { guardRecoveryTransport } from "../platform/recovery-transport-guard.ts";
import type { MitsuroStorage } from "../../../packages/state/src/storage.ts";
import { QueuedSuccessorRecovery } from "../../../packages/state/src/session/queuedSuccessorRecovery.ts";
import type {
  ChatMessage,
  QueuedMessage,
} from "../../../packages/state/src/session/types.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function rejectedError(
  operation: () => Promise<unknown>,
): Promise<Error> {
  try {
    await operation();
  } catch (error) {
    if (error instanceof Error) return error;
    throw new Error("operation rejected with a non-Error value");
  }
  throw new Error("operation unexpectedly resolved");
}

class FakeOriginLocks implements OriginLockManager {
  private readonly held = new Set<string>();

  async request<T>(
    name: string,
    _options: { mode: "exclusive"; ifAvailable: true },
    callback: (lock: { name: string } | null) => T | Promise<T>,
  ): Promise<T> {
    if (this.held.has(name)) return await callback(null);
    this.held.add(name);
    try {
      return await callback({ name });
    } finally {
      this.held.delete(name);
    }
  }
}

class FakeOrigin {
  readonly values = new Map<string, string>();
  private readonly listeners = new Map<
    string,
    Set<(key: string | null) => void>
  >();

  view(tabId: string): {
    storage: SyncStringStorage;
    invalidations: StorageInvalidationSource;
  } {
    return {
      storage: {
        getItem: (key) => this.values.get(key) ?? null,
        setItem: (key, value) => {
          this.values.set(key, value);
          this.emit(tabId, key);
        },
        removeItem: (key) => {
          this.values.delete(key);
          this.emit(tabId, key);
        },
      },
      invalidations: (listener) => {
        const listeners = this.listeners.get(tabId) ?? new Set();
        listeners.add(listener);
        this.listeners.set(tabId, listeners);
        return () => listeners.delete(listener);
      },
    };
  }

  private emit(sourceTabId: string, key: string): void {
    for (const [tabId, listeners] of this.listeners) {
      if (tabId === sourceTabId) continue;
      for (const listener of listeners) listener(key);
    }
  }
}

function acknowledge(
  recovery: LinearizableWebDurableRecovery,
  logicalKey = "mitsuro-queued-successor-recovery-v1:hive",
): string | null {
  recovery.beginSnapshot();
  const value = recovery.get(logicalKey);
  recovery.acknowledgeSnapshot();
  return value;
}

function queueStorage(
  durable: LinearizableWebDurableRecovery,
): MitsuroStorage {
  const regular = new Map<string, string>();
  return {
    get: (key) => regular.get(key) ?? null,
    set: (key, value) => regular.set(key, value),
    delete: (key) => {
      regular.delete(key);
    },
    getDurableSync: (key) => durable.get(key),
    getDurable: (key) => Promise.resolve(durable.get(key)),
    setDurable: (key, value) => durable.set(key, value),
    deleteDurable: (key) => durable.delete(key),
  };
}

async function releaseWebLock(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

Deno.test("recovery scope is stable, opaque, and isolated by server principal", () => {
  assert(
    sha256Hex("abc") ===
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    "the dependency-free SHA-256 must match the standard vector",
  );
  const token = "principal-capability-do-not-store";
  const first = deriveRecoveryConnectionScope(
    "https://Example.COM:443/api/",
    token,
  );
  const restart = deriveRecoveryConnectionScope(
    "https://example.com/api",
    token,
  );
  const switchedDeployment = deriveRecoveryConnectionScope(
    "https://example.com/another-path",
    token,
  );
  const switchedPrincipal = deriveRecoveryConnectionScope(
    "https://example.com/api",
    "different-principal-capability",
  );
  const switchedServer = deriveRecoveryConnectionScope(
    "https://other.example.com/api",
    token,
  );

  assert(
    first === restart,
    "the same origin and principal must survive restart",
  );
  assert(
    first !== switchedDeployment,
    "different server deployment paths on one origin must remain isolated",
  );
  assert(
    first !== switchedPrincipal,
    "a principal switch must use a separate recovery namespace",
  );
  assert(
    first !== switchedServer,
    "a server switch must use a separate recovery namespace",
  );
  assert(
    /^connection-v1-[0-9a-f]{64}$/.test(first) &&
      !first.includes("example.com") && !first.includes(token),
    "the public scope must contain neither the server URL nor credential",
  );
  const physicalKey = durableRecoveryStorageKey(first, "hive");
  const switchedKey = durableRecoveryStorageKey(switchedPrincipal, "hive");
  assert(
    !physicalKey.includes("example.com") && !physicalKey.includes(token),
    "durable keys must remain privacy-safe",
  );
  assert(
    physicalKey !== switchedKey,
    "a switched connection must not address the prior principal's record",
  );
  let rejectedQuery = false;
  try {
    deriveRecoveryConnectionScope("https://example.com/api?tenant=a", token);
  } catch {
    rejectedQuery = true;
  }
  assert(
    rejectedQuery,
    "unsupported base-URL query semantics must fail instead of aliasing a deployment",
  );
});

Deno.test("chat transport cannot begin before durable authority", async () => {
  const calls = { authority: 0, transport: 0 };
  let allowTransport = false;
  const client = {
    value: "client-state",
    streamChat(): Promise<string> {
      calls.transport += 1;
      return Promise.resolve("streamed");
    },
    steerSession(): Promise<string> {
      calls.transport += 1;
      return Promise.resolve("steered");
    },
    deleteSession(): Promise<string> {
      calls.transport += 1;
      return Promise.resolve("deleted");
    },
    getSession(): Promise<string> {
      return Promise.resolve(this.value);
    },
  };
  const guarded = guardRecoveryTransport(client, () => {
    calls.authority += 1;
    return allowTransport
      ? Promise.resolve()
      : Promise.reject(new DurableRecoveryPeerTabError());
  });

  const blocked = await rejectedError(() => guarded.streamChat());
  assert(
    blocked instanceof DurableRecoveryPeerTabError && calls.transport === 0,
    "the network method must not run when another tab owns recovery",
  );
  const blockedDelete = await rejectedError(() => guarded.deleteSession());
  assert(
    blockedDelete instanceof DurableRecoveryPeerTabError &&
      Number(calls.transport) === 0,
    "session deletion must not reach the server before recovery authority",
  );
  assert(
    await guarded.getSession() === "client-state" &&
      Number(calls.authority) === 2,
    "read-only client methods must stay usable and preserve their receiver",
  );
  allowTransport = true;
  assert(
    await guarded.steerSession() === "steered" &&
      Number(calls.transport) === 1 && Number(calls.authority) === 3,
    "an authorized chat transport may run after the ownership check",
  );
});

Deno.test("one web tab owns the complete recovery attempt lifecycle", async () => {
  const scope = deriveRecoveryConnectionScope(
    "https://server.example",
    "principal-a",
  );
  const origin = new FakeOrigin();
  const locks = new FakeOriginLocks();
  const firstView = origin.view("first");
  const secondView = origin.view("second");
  const first = new LinearizableWebDurableRecovery(
    scope,
    firstView.storage,
    locks,
    firstView.invalidations,
  );
  const second = new LinearizableWebDurableRecovery(
    scope,
    secondView.storage,
    locks,
    secondView.invalidations,
  );

  assert(
    await first.activate(),
    "the first tab should acquire connection ownership",
  );
  acknowledge(first);
  assert(
    !(await second.activate()),
    "the second tab must observe active peer ownership without queueing behind it",
  );
  acknowledge(second);

  first.get("mitsuro-queued-successor-recovery-v1:hive");
  await first.set(
    "mitsuro-queued-successor-recovery-v1:hive",
    '{"phase":"claiming"}',
  );
  const peerMutation = await rejectedError(() =>
    second.set(
      "mitsuro-queued-successor-recovery-v1:hive",
      '{"phase":"in_flight"}',
    )
  );
  assert(
    peerMutation instanceof DurableRecoveryPeerTabError,
    "a peer tab must fail before it can claim or mark an attempt in flight",
  );
  const peerTransport = await rejectedError(() => second.ensureAuthority());
  assert(
    peerTransport instanceof DurableRecoveryPeerTabError,
    "the same ownership check must fence transport",
  );
  assert(
    first.get("mitsuro-queued-successor-recovery-v1:hive") ===
      '{"phase":"claiming"}',
    "the losing tab must not overwrite the owner's attempt",
  );

  first.dispose();
  await releaseWebLock();
  const takeover = await rejectedError(() => second.ensureAuthority());
  assert(
    takeover instanceof DurableRecoverySnapshotError,
    "a new owner must rebuild from the authoritative snapshot before transport",
  );
  assert(
    acknowledge(second) === '{"phase":"claiming"}',
    "takeover must refresh the prior owner's durable state",
  );
  await second.ensureAuthority();
  await second.set(
    "mitsuro-queued-successor-recovery-v1:hive",
    '{"phase":"in_flight"}',
  );
  assert(
    second.get("mitsuro-queued-successor-recovery-v1:hive") ===
      '{"phase":"in_flight"}',
    "the rebuilt successor may continue after acquiring ownership",
  );
  second.dispose();
  await releaseWebLock();

  const restartedView = origin.view("restarted");
  const restarted = new LinearizableWebDurableRecovery(
    scope,
    restartedView.storage,
    locks,
    restartedView.invalidations,
  );
  assert(
    await restarted.activate(),
    "the same connection must reacquire ownership after restart",
  );
  assert(
    acknowledge(restarted) === '{"phase":"in_flight"}',
    "the same connection must recover its prior durable attempt",
  );

  const isolatedScope = deriveRecoveryConnectionScope(
    "https://server.example",
    "principal-a-switched",
  );
  const isolatedView = origin.view("isolated");
  const isolated = new LinearizableWebDurableRecovery(
    isolatedScope,
    isolatedView.storage,
    locks,
    isolatedView.invalidations,
  );
  assert(
    await isolated.activate() && acknowledge(isolated) === null,
    "a switched principal must own a separate empty namespace",
  );
  restarted.dispose();
  isolated.dispose();
});

Deno.test("a peer queue rolls back claim before any transport authority exists", async () => {
  const scope = deriveRecoveryConnectionScope(
    "https://server.example/deployment",
    "principal-claim",
  );
  const origin = new FakeOrigin();
  const locks = new FakeOriginLocks();
  const ownerView = origin.view("claim-owner");
  const peerView = origin.view("claim-peer");
  const ownerDurable = new LinearizableWebDurableRecovery(
    scope,
    ownerView.storage,
    locks,
    ownerView.invalidations,
  );
  const peerDurable = new LinearizableWebDurableRecovery(
    scope,
    peerView.storage,
    locks,
    peerView.invalidations,
  );
  assert(await ownerDurable.activate(), "the owner tab must acquire the lock");
  assert(
    !(await peerDurable.activate()),
    "the peer tab must remain read-only",
  );

  ownerDurable.beginSnapshot();
  const ownerQueue = new QueuedSuccessorRecovery(
    queueStorage(ownerDurable),
    "hive",
  );
  ownerDurable.acknowledgeSnapshot();
  peerDurable.beginSnapshot();
  const peerQueue = new QueuedSuccessorRecovery(
    queueStorage(peerDurable),
    "hive",
  );
  peerDurable.acknowledgeSnapshot();
  await Promise.all([ownerQueue.ready(), peerQueue.ready()]);

  const message: QueuedMessage = {
    id: "queued-claim",
    content: "private queued task",
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  const row: ChatMessage = {
    id: message.id,
    role: "user",
    content: message.content,
    isQueued: true,
  };
  await ownerQueue.appendPending("worker-session", message, row);

  const peerClaim = await rejectedError(() =>
    peerQueue.claim(
      {
        id: "peer-claim",
        sessionId: "worker-session",
        queuedMessages: [message],
      },
      [row],
    )
  );
  assert(
    peerClaim instanceof DurableRecoveryPeerTabError &&
      peerQueue.get("worker-session")?.phase === "pending",
    "a losing tab must roll its tentative claiming state back without an attempt token",
  );

  const ownerClaim = await ownerQueue.claim(
    {
      id: "owner-claim",
      sessionId: "worker-session",
      queuedMessages: [message],
    },
    [row],
  );
  assert(
    await ownerQueue.markInFlight(
      "worker-session",
      ownerClaim.id,
      ownerClaim.attemptToken,
    ),
    "only the durable owner may advance the exact claim to in-flight",
  );
  let ownerTransportCalls = 0;
  const ownerClient = guardRecoveryTransport(
    {
      streamChat(): Promise<void> {
        ownerTransportCalls += 1;
        return Promise.resolve();
      },
    },
    () => ownerDurable.ensureAuthority(),
  );
  const peerClient = guardRecoveryTransport(
    {
      streamChat(): Promise<void> {
        return Promise.reject(
          new Error("peer transport must never be reached"),
        );
      },
    },
    () => peerDurable.ensureAuthority(),
  );
  await ownerClient.streamChat();
  assert(
    await rejectedError(() => peerClient.streamChat()) instanceof
        DurableRecoveryPeerTabError && ownerTransportCalls === 1,
    "the peer cannot bypass the failed durable claim through stream transport",
  );

  ownerQueue.dispose();
  peerQueue.dispose();
  ownerDurable.dispose();
  peerDurable.dispose();
});

Deno.test("web recovery invalidates peers and rejects stale compare-and-swap", async () => {
  const scope = deriveRecoveryConnectionScope(
    "https://server.example",
    "principal-b",
  );
  const origin = new FakeOrigin();
  const locks = new FakeOriginLocks();
  const ownerView = origin.view("owner");
  const peerView = origin.view("peer");
  const owner = new LinearizableWebDurableRecovery(
    scope,
    ownerView.storage,
    locks,
    ownerView.invalidations,
  );
  const peer = new LinearizableWebDurableRecovery(
    scope,
    peerView.storage,
    locks,
    peerView.invalidations,
  );
  let peerInvalidations = 0;
  peer.subscribe(() => peerInvalidations += 1);

  await owner.activate();
  acknowledge(owner, "queue");
  acknowledge(peer, "queue");
  owner.get("queue");
  await owner.set("queue", "claimed");
  assert(
    peerInvalidations >= 1,
    "epoch and record writes must invalidate another tab's cached graph",
  );

  owner.get("queue");
  origin.values.set(durableRecoveryStorageKey(scope, "queue"), "bypassed");
  const conflict = await rejectedError(() => owner.set("queue", "stale"));
  assert(
    conflict instanceof DurableRecoveryConflictError,
    "a stale read must never become a last-writer-wins overwrite",
  );
  assert(
    origin.values.get(durableRecoveryStorageKey(scope, "queue")) === "bypassed",
    "CAS failure must retain the newer authoritative value",
  );

  acknowledge(owner, "queue");
  origin.values.set(durableRecoveryEpochKey(scope), "999");
  const staleTransport = await rejectedError(() => owner.ensureAuthority());
  assert(
    staleTransport instanceof DurableRecoverySnapshotError,
    "transport must compare the current origin epoch even if an event was missed",
  );
  owner.dispose();
  peer.dispose();
});

Deno.test("unsupported web locking fails closed before mutation or transport", async () => {
  const scope = deriveRecoveryConnectionScope(
    "https://server.example",
    "principal-c",
  );
  const origin = new FakeOrigin();
  const view = origin.view("unsupported");
  const recovery = new LinearizableWebDurableRecovery(
    scope,
    view.storage,
    null,
    view.invalidations,
  );
  acknowledge(recovery, "queue");
  assert(
    await rejectedError(() => recovery.ensureAuthority()) instanceof
      DurableRecoveryLockUnavailableError,
    "transport must fail closed without origin-wide locking",
  );
  assert(
    await rejectedError(() => recovery.set("queue", "payload")) instanceof
      DurableRecoveryLockUnavailableError,
    "durable mutation must fail closed without origin-wide locking",
  );
});

Deno.test("discard scrubs recovery payload before session deletion transport", async () => {
  const scope = deriveRecoveryConnectionScope(
    "https://server.example/deletion",
    "principal-delete",
  );
  const origin = new FakeOrigin();
  const locks = new FakeOriginLocks();
  const view = origin.view("delete-owner");
  const durable = new LinearizableWebDurableRecovery(
    scope,
    view.storage,
    locks,
    view.invalidations,
  );
  assert(await durable.activate(), "the deleting tab must own recovery");
  durable.beginSnapshot();
  const queue = new QueuedSuccessorRecovery(queueStorage(durable), "hive");
  durable.acknowledgeSnapshot();
  await queue.ready();

  const privatePrompt = "private task that deletion must scrub";
  const message: QueuedMessage = {
    id: "queued-delete",
    content: privatePrompt,
    attachments: [],
    sendOptions: { sessionType: "hive" },
  };
  await queue.appendPending("session-delete", message, {
    id: message.id,
    role: "user",
    content: privatePrompt,
    isQueued: true,
  });

  let deleteTransportCalls = 0;
  const guarded = guardRecoveryTransport(
    {
      deleteSession(): Promise<void> {
        deleteTransportCalls += 1;
        for (const value of origin.values.values()) {
          assert(
            !value.includes(privatePrompt),
            "server DELETE must not begin while recovery retains private payload",
          );
        }
        return Promise.resolve();
      },
    },
    () => durable.ensureAuthority(),
  );

  await queue.delete("session-delete");
  await guarded.deleteSession();
  assert(
    deleteTransportCalls === 1,
    "the delete transport may run after the payload-free tombstone is durable",
  );
  queue.dispose();
  durable.dispose();
});

Deno.test("connection intents and every deletion surface retain source-order fences", async () => {
  const [
    connection,
    sessionsRoute,
    sessionActions,
    deletionAdmission,
  ] = await Promise.all([
    Deno.readTextFile(new URL("../hooks/useConnection.tsx", import.meta.url)),
    Deno.readTextFile(
      new URL("../app/(tabs)/sessions.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL(
        "../components/chat-screen/useSessionActions.ts",
        import.meta.url,
      ),
    ),
    Deno.readTextFile(
      new URL(
        "../components/chat-screen/sessionDeletionAdmission.ts",
        import.meta.url,
      ),
    ),
  ]);

  const connectBoundary = connection.slice(
    connection.indexOf("const doConnect"),
    connection.indexOf("// Load saved connection"),
  );
  const healthAwait = connectBoundary.indexOf(
    "await newClient.checkHealth()",
  );
  const authAwait = connectBoundary.indexOf(
    "await newClient.bootstrapAuth()",
  );
  const recoveryPublish = connectBoundary.indexOf(
    "setRecoveryConnectionScope(nextRecoveryScope)",
  );
  assert(
    connection.includes("createConnectionIntentCoordinator") &&
      connection.includes("runCurrentCredentialOperation") &&
      connection.includes("runCredentialOperation") &&
      connection.indexOf("initialConnectionIntentRef.current =") <
        connection.indexOf("// Load saved connection") &&
      healthAwait >= 0 &&
      connectBoundary.indexOf(
          "if (!connectionIntents.isCurrent(intent)) return false;",
          healthAwait,
        ) > healthAwait &&
      authAwait > healthAwait &&
      connectBoundary.indexOf(
          "if (!connectionIntents.isCurrent(intent)) return false;",
          authAwait,
        ) > authAwait &&
      recoveryPublish > authAwait &&
      connectBoundary.indexOf("setClient(null)") < healthAwait &&
      connectBoundary.indexOf("setRecoveryConnectionScope(null)") <
        healthAwait,
    "health/auth completion must stay fenced and the prior account must be hidden before connection publication",
  );

  const disconnectBoundary = connection.slice(
    connection.indexOf("const disconnect"),
    connection.indexOf("const reconnect"),
  );
  const initialLoadBoundary = connection.slice(
    connection.indexOf("// Load saved connection"),
    connection.indexOf("const connect ="),
  );
  const reconnectBoundary = connection.slice(
    connection.indexOf("const reconnect"),
    connection.indexOf("return (", connection.indexOf("const reconnect")),
  );
  assert(
    disconnectBoundary.indexOf("connectionIntents.begin()") >= 0 &&
      disconnectBoundary.indexOf("connectionIntents.begin()") <
        disconnectBoundary.indexOf("setClient(null)") &&
      disconnectBoundary.indexOf("runCredentialOperation") <
        disconnectBoundary.indexOf("deleteConnectionCredentials") &&
      initialLoadBoundary.indexOf("runCurrentCredentialOperation") <
        initialLoadBoundary.indexOf("readConnectionCredentials") &&
      initialLoadBoundary.indexOf("honorPendingConnectionLogout") >= 0 &&
      initialLoadBoundary.indexOf("honorPendingConnectionLogout") <
        initialLoadBoundary.indexOf("connectionFromInjectedGlobals") &&
      reconnectBoundary.indexOf("runCurrentCredentialOperation") <
        reconnectBoundary.indexOf("readConnectionCredentials") &&
      reconnectBoundary.indexOf("setClient(null)") <
        reconnectBoundary.indexOf("readConnectionCredentials"),
    "Disconnect must invalidate stale completions and every migration-capable credential access must remain serialized",
  );

  const routeDelete = sessionsRoute.slice(
    sessionsRoute.indexOf("onPress: async () =>"),
    sessionsRoute.indexOf("const t = theme.colors"),
  );
  const routeTransport = routeDelete.indexOf(
    "const deleted = await expectedStores.sessions",
  );
  const routeDetach = routeDelete.indexOf(
    "clearDeletedSessionFromModeStoreGraphs",
  );
  assert(
    routeDelete.indexOf("await beginAllModeSessionDeletionAdmission") >= 0 &&
      routeDelete.indexOf("await beginAllModeSessionDeletionAdmission") <
        routeTransport &&
      routeDetach > routeTransport &&
      routeDelete.indexOf("admission.commit()") > routeDetach &&
      routeDelete.includes("await admission.rollback()") &&
      sessionsRoute.includes(
        "activeRecoveryScopeRef.current = recoveryConnectionScope",
      ) &&
      sessionsRoute.includes("activeRecoveryScopeRef.current = null") &&
      sessionsRoute.includes("activeStoresRef.current === expectedStores") &&
      sessionsRoute.includes("activeStoresRef.current = null") &&
      routeDelete.includes("activeStoresRef.current?.modes"),
    "the legacy session route must scrub and retain an exact-scope lifecycle fence before DELETE",
  );

  const genericDelete = sessionActions.slice(
    sessionActions.indexOf("const handleDeleteSession"),
    sessionActions.indexOf("const handleSetSessionPinned"),
  );
  const genericTransport = genericDelete.indexOf(
    "deleted = await sessionsStore.getState().deleteSession",
  );
  const genericAdmissionFailure = genericDelete.slice(
    genericDelete.indexOf("catch (scrubError)"),
    genericDelete.indexOf(
      "if (!isCurrentDeletionBoundary()) {",
      genericDelete.indexOf("catch (scrubError)"),
    ),
  );
  const genericDetach = genericDelete.indexOf(
    "clearDeletedSessionFromModeStoreGraphs",
  );
  const projectDelete = sessionActions.slice(
    sessionActions.indexOf("const handleDeleteProjectSessions"),
    sessionActions.indexOf(
      "return {",
      sessionActions.indexOf(
        "const handleDeleteProjectSessions",
      ),
    ),
  );
  const deletionBatch = deletionAdmission.slice(
    deletionAdmission.indexOf("export async function runSessionDeletionBatch"),
  );
  const projectAdmissionFailure = projectDelete.slice(
    projectDelete.indexOf(
      'admissionResults.some((result) => result.status === "rejected")',
    ),
    projectDelete.indexOf(
      "if (!isCurrentDeletionBoundary())",
      projectDelete.indexOf(
        'admissionResults.some((result) => result.status === "rejected")',
      ),
    ),
  );
  assert(
    genericDelete.indexOf("await beginAllModeSessionDeletionAdmission") >= 0 &&
      genericDelete.indexOf("await beginAllModeSessionDeletionAdmission") <
        genericDelete.indexOf(
          "sessionsStore.getState().deleteSession",
        ) &&
      projectDelete.indexOf("Promise.allSettled") >= 0 &&
      projectDelete.indexOf("beginAllModeSessionDeletionAdmission") <
        projectDelete.indexOf(
          "sessionsStore.getState().deleteSession",
        ) &&
      projectDelete.indexOf("No conversations were deleted") <
        projectDelete.indexOf("onDeleted?.()") &&
      genericDelete.indexOf("onDeleted?.()") >
        genericDelete.indexOf("await beginAllModeSessionDeletionAdmission") &&
      genericDelete.indexOf("onDeleted?.()") < genericTransport &&
      !genericAdmissionFailure.includes("onFailed?.()") &&
      genericDetach > genericTransport &&
      genericDelete.indexOf("admission.commit()") > genericDetach &&
      genericDelete.indexOf(
          "await admission.rollback()",
          genericTransport,
        ) > genericTransport &&
      projectDelete.indexOf("onDeleted?.()") >
        projectDelete.indexOf("Promise.allSettled") &&
      projectDelete.includes("rollbackSessionDeletionAdmissions") &&
      !projectAdmissionFailure.includes("onFailed?.(deletionIds)") &&
      projectDelete.indexOf("runSessionDeletionBatch") >
        projectDelete.indexOf("beginAllModeSessionDeletionAdmission") &&
      projectDelete.includes("onFailed?.(result.remainingIds)") &&
      projectDelete.includes(
        "clearDeletedSessionFromModeStoreGraphs",
      ) &&
      projectDelete.includes("deletionModeStoresRef.current") &&
      projectDelete.includes("onFailed?.(deletionIds)") &&
      projectDelete.includes(
        "if (!isCurrentDeletionBoundary()) {\n            // The batch completed against the captured graph",
      ) &&
      genericDelete.includes(
        "const cleared = clearDeletedSessionFromModeStoreGraphs",
      ) &&
      genericDelete.includes("deletionModeStoresRef.current") &&
      genericDelete.includes("onFailed?.()") &&
      deletionBatch.indexOf("deleted = await deleteSession(sessionId)") <
        deletionBatch.indexOf("beforeCommit(sessionId)") &&
      deletionBatch.indexOf("beforeCommit(sessionId)") <
        deletionBatch.indexOf("admission.commit()") &&
      deletionBatch.indexOf(
          "await rollbackOutstanding()",
          deletionBatch.indexOf("deleted = await deleteSession(sessionId)"),
        ) > deletionBatch.indexOf("deleted = await deleteSession(sessionId)") &&
      deletionBatch.includes("remainingIds: ids.slice(index)") &&
      sessionActions.includes("deletionBoundaryActiveRef.current = true") &&
      sessionActions.includes("deletionBoundaryActiveRef.current = false") &&
      sessionActions.includes("deletionModeStoresRef.current === modeStores") &&
      sessionActions.includes(
        "deletionSessionsStoreRef.current === sessionsStore",
      ),
    "single and project deletion must hold producer admission across transport, roll back on failure, and stop after graph replacement or disposal",
  );
});

Deno.test("Disconnect stays neutral but does not report durable success early", async () => {
  const [connection, settings, sections, identityStorage] = await Promise.all([
    Deno.readTextFile(new URL("../hooks/useConnection.tsx", import.meta.url)),
    Deno.readTextFile(
      new URL("../components/settings/SettingsPanel.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../components/settings/sections.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../platform/identity-storage.ts", import.meta.url),
    ),
  ]);
  const disconnect = connection.slice(
    connection.indexOf("const disconnect"),
    connection.indexOf("const reconnect"),
  );
  const settingsDisconnect = settings.slice(
    settings.indexOf("const handleDisconnect"),
    settings.indexOf("const updateProviderForm"),
  );
  const deleteAwait = disconnect.indexOf(
    "await connectionIntents.runCredentialOperation",
  );
  const initialLoad = connection.slice(
    connection.indexOf("// Load saved connection"),
    connection.indexOf("const connect ="),
  );
  assert(
    disconnect.indexOf('setStatus("disconnecting")') >= 0 &&
      disconnect.indexOf("setClient(null)") < deleteAwait &&
      disconnect.indexOf("setRecoveryConnectionScope(null)") < deleteAwait &&
      disconnect.indexOf("setIsConfigured(false)") > deleteAwait &&
      disconnect.indexOf('setStatus("error")') > deleteAwait &&
      disconnect.includes("throw new Error(message)") &&
      settingsDisconnect.includes("await disconnect()") &&
      settingsDisconnect.includes("catch (logoutError)") &&
      !settingsDisconnect.includes("await Haptics.impactAsync") &&
      sections.includes("<MessageBanner text={connectError} />") &&
      identityStorage.includes("serverLogoutIntent") &&
      identityStorage.includes("honorPendingConnectionLogout") &&
      initialLoad.indexOf("honorPendingConnectionLogout") >= 0 &&
      initialLoad.indexOf("honorPendingConnectionLogout") <
        initialLoad.indexOf("connectionFromInjectedGlobals") &&
      identityStorage.indexOf("Promise.allSettled") >
        identityStorage.indexOf("deleteConnectionCredentials"),
    "Disconnect must neutralize immediately, await every credential deletion, retain configured state on failure, and expose the failure",
  );
});

Deno.test("mobile wiring scopes adapters, transport, deletion, and account handoff", async () => {
  const [
    stores,
    transportGuard,
    sessionsRoute,
    connection,
    layout,
    nativeStorage,
    webStorage,
    stateStorage,
  ] = await Promise.all([
    Deno.readTextFile(new URL("../hooks/useStores.tsx", import.meta.url)),
    Deno.readTextFile(
      new URL("../platform/recovery-transport-guard.ts", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../app/(tabs)/sessions.tsx", import.meta.url),
    ),
    Deno.readTextFile(new URL("../hooks/useConnection.tsx", import.meta.url)),
    Deno.readTextFile(new URL("../app/_layout.tsx", import.meta.url)),
    Deno.readTextFile(
      new URL("../platform/mitsuro-storage.native.ts", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../platform/mitsuro-storage.web.ts", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../../../packages/state/src/storage.ts", import.meta.url),
    ),
  ]);
  assert(
    connection.includes("deriveRecoveryConnectionScope(url, token)") &&
      layout.includes("recoveryConnectionScope={recoveryConnectionScope}"),
    "the authenticated connection must own the opaque recovery scope",
  );
  assert(
    stores.includes("guardRecoveryTransport(") &&
      stores.includes("ensureDurableRecoveryAuthority") &&
      stores.includes("subscribeDurableRecoveryInvalidation") &&
      stores.includes(
        "scopedStoreGraph?.recoveryConnectionScope === recoveryConnectionScope",
      ) &&
      stores.includes("isConnectionHandoff ? null : children") &&
      transportGuard.includes('"streamChat"') &&
      transportGuard.includes('"steerSession"') &&
      transportGuard.includes('"deleteSession"') &&
      stores.includes("createSessionsStore(sessionClient") &&
      !sessionsRoute.includes("client.deleteSession(") &&
      sessionsRoute.includes("beginAllModeSessionDeletionAdmission(") &&
      sessionsRoute.includes(
        "sessionSnapshot?.recoveryConnectionScope === recoveryConnectionScope",
      ),
    "session sends must share the owner fence and account switches must synchronously hide stale stores",
  );
  assert(
    stateStorage.includes("readonly durableRecoveryNamespace?: string") &&
      nativeStorage.includes(
        "constructor(readonly durableRecoveryNamespace: string)",
      ) &&
      nativeStorage.includes(
        "durableRecoveryStorageKey(this.durableRecoveryNamespace",
      ) &&
      webStorage.includes("LinearizableWebDurableRecovery") &&
      webStorage.includes(
        "constructor(readonly durableRecoveryNamespace: string)",
      ) &&
      webStorage.includes("createStorage(connectionScope: string)"),
    "native and web durable records and their shared coordinator must use one opaque connection namespace",
  );
});
