import { MitsuroApiError, type StreamCallbacks } from "@mitsuro/api";
import { createSessionStore } from "../src/session/store.ts";
import { WorkerInputIdempotency } from "../src/session/workerInputIdempotency.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals<T>(actual: T, expected: T, message: string): void {
  if (!Object.is(actual, expected)) {
    throw new Error(
      `${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`,
    );
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((next, fail) => {
    resolve = next;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function createStorage() {
  const data = new Map<string, string>();
  return {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => {
      data.set(key, value);
    },
    delete: (key: string) => {
      data.delete(key);
    },
  };
}

function createWorkspace() {
  return {
    getState: () => ({
      directory: null,
      mode: "neutral" as const,
      targetBranch: null,
      initFromSession: () => {},
      setWorkspace: () => {},
      clear: () => {},
    }),
  };
}

function createSessionsStore() {
  return {
    getState: () => ({
      sessions: [],
      loadSessions: () => {},
    }),
  };
}

function createPlanStore() {
  return {
    getState: () => ({
      setVisible: () => {},
      setItems: () => {},
      setWorkflow: () => {},
    }),
  };
}

function sessionState(agentState: string, sessionId = "worker-dm") {
  return {
    id: sessionId,
    agent_state: agentState,
    started_at: null,
    last_event_at: null,
    mode: "build",
    permission_mode: "autonomous",
    recovery: null,
    live_partial_assistant: null,
    pending_interactions: [],
    delegated_tools: [],
    recent_delegated_runs: [],
    last_event_sequence: null,
  };
}

function sessionResponse(sessionId = "worker-dm") {
  return {
    session: {
      id: sessionId,
      title: "Worker DM",
      token_count: 0,
      working_dir: null,
      project_dir: null,
      workspace_mode: "neutral",
      session_type: "hive",
      parent_session_id: null,
      mode: "build",
      permission_mode: "autonomous",
      updated_at: "2026-08-25T00:00:00Z",
      model: null,
      target_branch: null,
    },
    messages: [],
  };
}

function workerBinding(sessionId: string) {
  return {
    kind: "worker_dm",
    session_id: sessionId,
    worker: {
      id: "worker-1",
      revision: 1,
      slug: "tester",
      display_name: "Tester",
      permission_mode: "autonomous",
      autonomy: "continuous",
      status: "active",
      created_at: "2026-08-25T00:00:00Z",
      updated_at: "2026-08-25T00:00:00Z",
      attention: [],
    },
  };
}

function acceptWorkerInput(
  callbacks: StreamCallbacks,
  sessionId: string,
  runId: string,
): void {
  callbacks.onWorkerResponsePending?.({
    type: "worker_response_pending",
    worker_id: "worker-1",
    session_id: sessionId,
    run_id: runId,
  });
}

Deno.test("uncertain Worker identities survive intervening same-session intents", () => {
  const identities = new WorkerInputIdempotency();
  const first = identities.keyFor("worker-a", "chat", "fingerprint-a");
  const intervening = identities.keyFor(
    "worker-a",
    "chat",
    "fingerprint-b",
  );
  assert(
    first !== intervening,
    "different intents require distinct identities",
  );
  assertEquals(
    identities.keyFor("worker-a", "chat", "fingerprint-a"),
    first,
    "an intervening turn cannot rotate an uncertain earlier identity",
  );
  identities.accept("worker-a", "chat", intervening);
  assertEquals(
    identities.keyFor("worker-a", "chat", "fingerprint-a"),
    first,
    "accepting the intervening turn must not clear the earlier identity",
  );
  identities.accept("worker-a", "chat", first);
  assert(
    identities.keyFor("worker-a", "chat", "fingerprint-a") !== first,
    "exact acceptance rotates only the accepted identity",
  );
});

Deno.test("a seventeenth unresolved Worker identity fails closed", () => {
  const identities = new WorkerInputIdempotency();
  const first = identities.keyFor("worker-0", "chat", "fingerprint-0");
  for (let index = 1; index < 16; index += 1) {
    identities.keyFor(
      `worker-${index}`,
      "chat",
      `fingerprint-${index}`,
    );
  }
  let rejected = false;
  try {
    identities.keyFor("worker-16", "chat", "fingerprint-16");
  } catch {
    rejected = true;
  }
  assert(rejected, "capacity cannot evict an unresolved exact key");
  assertEquals(
    identities.keyFor("worker-0", "chat", "fingerprint-0"),
    first,
    "the oldest unresolved identity remains stable",
  );
});

Deno.test("distinct identical queued Worker turns reserve distinct server keys", async () => {
  const firstAttached = deferred<StreamCallbacks>();
  const firstTransport = deferred<void>();
  const requests: Array<{ message: string; key: string }> = [];
  let attempt = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { message: string; session_id?: string },
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      attempt += 1;
      requests.push({
        message: request.message,
        key: options?.idempotencyKey ?? "",
      });
      if (attempt === 1) {
        firstAttached.resolve(callbacks);
        await firstTransport.promise;
        return;
      }
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-1",
        session_id: request.session_id ?? "worker-dm",
        run_id: `identical-run-${attempt}`,
      });
      callbacks.onWorkerResponseCommitted?.({
        type: "worker_response_committed",
        worker_id: "worker-1",
        session_id: request.session_id ?? "worker-dm",
        run_id: `identical-run-${attempt}`,
      });
      callbacks.onFinish(request.session_id ?? "worker-dm", "completed");
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-dm", "Worker DM", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "active seed",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const firstCallbacks = await firstAttached.promise;
  const identicalAttachment = [{
    name: "same.txt",
    type: "file" as const,
    mimeType: "text/plain",
    text: "same attachment",
  }];
  await store.getState().sendMessage(
    "identical queued turn",
    identicalAttachment,
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await store.getState().sendMessage(
    "identical queued turn",
    identicalAttachment,
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );

  firstCallbacks.onWorkerResponsePending?.({
    type: "worker_response_pending",
    worker_id: "worker-1",
    session_id: "worker-dm",
    run_id: "seed-run",
  });
  firstCallbacks.onWorkerResponseCommitted?.({
    type: "worker_response_committed",
    worker_id: "worker-1",
    session_id: "worker-dm",
    run_id: "seed-run",
  });
  firstCallbacks.onFinish("worker-dm", "completed");
  firstTransport.resolve();
  await firstSend;
  for (let index = 0; index < 50 && requests.length < 3; index += 1) {
    await new Promise((resolve) => setTimeout(resolve, 2));
  }

  const identicalRequests = requests.filter((request) =>
    request.message === "identical queued turn"
  );
  assertEquals(
    identicalRequests.length,
    2,
    "both distinct identical turns reach the Worker exactly once",
  );
  assert(
    identicalRequests[0]?.key !== identicalRequests[1]?.key,
    "distinct unresolved turns must never share a server idempotency key",
  );
  store.getState().cleanup();
});

Deno.test("Worker chat reuses an uncertain key and rotates only after acceptance", async () => {
  const keys: string[] = [];
  let attempt = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      const key = options?.idempotencyKey;
      assert(key, "direct Worker chat must carry an idempotency key");
      keys.push(key);
      attempt += 1;
      if (attempt <= 2) {
        callbacks.onError(`transport failed ${attempt}`);
        return;
      }
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-1",
        session_id: "worker-dm",
        run_id: `run-${attempt}`,
      });
      callbacks.onWorkerResponseCommitted?.({
        type: "worker_response_committed",
        worker_id: "worker-1",
        session_id: "worker-dm",
        run_id: `run-${attempt}`,
      });
      callbacks.onFinish("worker-dm", "completed");
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
    updateSession: async () => ({}),
    setCurrentModel: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("worker-dm", "Worker DM", undefined, "hive");

  for (let index = 0; index < 2; index += 1) {
    await store.getState().sendMessage("inspect the failure").catch(() => {});
  }
  assertEquals(keys[1], keys[0], "uncertain retries must reuse the exact key");

  await store.getState().sendMessage("inspect the failure");
  assertEquals(
    keys[2],
    keys[0],
    "the accepted retry must keep the pending key",
  );
  await store.getState().sendMessage("inspect the failure");
  assert(
    keys[3] !== keys[2],
    "a new send after exact Worker acceptance must receive a new key",
  );
  store.getState().cleanup();
});

Deno.test("rejected queued Worker successor retries once with its original identity", async () => {
  const firstTransport = deferred<void>();
  const firstAttached = deferred<StreamCallbacks>();
  const queuedRejected = deferred<void>();
  const queuedAccepted = deferred<void>();
  const keys: string[] = [];
  let attempt = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { session_id?: string },
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      attempt += 1;
      keys.push(options?.idempotencyKey ?? "");
      if (attempt === 1) {
        firstAttached.resolve(callbacks);
        await firstTransport.promise;
        return;
      }
      if (attempt === 2) {
        callbacks.onError("queued successor rejected");
        queuedRejected.resolve();
        return;
      }
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-1",
        session_id: request.session_id ?? "worker-dm",
        run_id: "queued-retry-run",
      });
      callbacks.onFinish(request.session_id ?? "worker-dm", "completed");
      queuedAccepted.resolve();
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-dm", "Worker DM", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const firstCallbacks = await firstAttached.promise;
  await store.getState().sendMessage(
    "queued private input",
    [{
      name: "note.txt",
      type: "file",
      mimeType: "text/plain",
      text: "queued attachment",
    }],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  acceptWorkerInput(firstCallbacks, "worker-dm", "queued-seed-run");
  firstCallbacks.onFinish("worker-dm", "completed");
  firstTransport.resolve();
  await firstSend;
  await queuedRejected.promise;
  for (
    let index = 0;
    index < 20 && store.getState().queuedMessages.length === 0;
    index += 1
  ) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }

  assertEquals(
    store.getState().queuedMessages.length,
    1,
    "a definite successor rejection restores its exact pending payload",
  );
  assertEquals(
    store.getState().messages.filter((message) =>
      message.content.includes("queued private input")
    ).length,
    1,
    "the original optimistic row is retained once without a false sent duplicate",
  );
  assert(
    store.getState().messages.some((message) =>
      message.content.includes("queued private input") && message.isQueued
    ),
    "the rejected row remains visibly unsent",
  );

  await store.getState().sendMessage(
    "intervening accepted turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  ).catch((error) => {
    assert(
      String(error).includes("older queued message"),
      "a newer draft must wait behind the recovered exact queue",
    );
  });
  await queuedAccepted.promise;

  assertEquals(attempt, 3, "the older queue retries before a newer draft");
  assertEquals(
    keys[2],
    keys[1],
    "the uncertain queued successor retains its original idempotency identity",
  );
  assertEquals(
    store.getState().queuedMessages.length,
    0,
    "exact successor acceptance clears the pending queue",
  );
  assertEquals(
    store.getState().messages.filter((message) =>
      message.content.includes("queued private input")
    ).length,
    1,
    "acceptance commits the same optimistic row without duplication",
  );
  assert(
    store.getState().messages.some((message) =>
      message.content.includes("queued private input") && !message.isQueued
    ),
    "the exact accepted row becomes durable-looking only after acceptance",
  );
  store.getState().cleanup();
});

Deno.test("rejected queued Worker payload and identity survive a fresh store graph", async () => {
  const storage = createStorage();
  const firstAttached = deferred<StreamCallbacks>();
  const firstTransport = deferred<void>();
  const firstRejected = deferred<void>();
  const firstKeys: string[] = [];
  let firstAttempt = 0;
  const firstClient = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      firstAttempt += 1;
      firstKeys.push(options?.idempotencyKey ?? "");
      if (firstAttempt === 1) {
        firstAttached.resolve(callbacks);
        await firstTransport.promise;
        return;
      }
      callbacks.onError("definite queued failure");
      firstRejected.resolve();
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const firstStore = createSessionStore(
    firstClient as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  firstStore.getState().initSession(
    "worker-dm",
    "Worker DM",
    undefined,
    "hive",
  );
  const initialSend = firstStore.getState().sendMessage(
    "initial turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const callbacks = await firstAttached.promise;
  await firstStore.getState().sendMessage(
    "restart-safe queued input",
    [{
      name: "note.txt",
      type: "file",
      mimeType: "text/plain",
      text: "durable queued attachment",
    }],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  acceptWorkerInput(callbacks, "worker-dm", "restart-seed-run");
  callbacks.onFinish("worker-dm", "completed");
  firstTransport.resolve();
  await initialSend;
  await firstRejected.promise;
  for (
    let index = 0;
    index < 30 && firstStore.getState().queuedMessages.length === 0;
    index += 1
  ) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assertEquals(
    firstStore.getState().queuedMessages.length,
    1,
    "the first graph persists and restores the rejected batch",
  );
  const originalQueuedKey = firstKeys[1];
  assert(
    originalQueuedKey,
    "the first queued Worker attempt owns an exact key",
  );
  firstStore.getState().cleanup();

  const retried = deferred<void>();
  const secondKeys: string[] = [];
  let secondAttempt = 0;
  const secondClient = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { session_id?: string },
      nextCallbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      secondAttempt += 1;
      secondKeys.push(options?.idempotencyKey ?? "");
      nextCallbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-1",
        session_id: request.session_id ?? "worker-dm",
        run_id: `restart-run-${secondAttempt}`,
      });
      nextCallbacks.onFinish(request.session_id ?? "worker-dm", "completed");
      if (secondAttempt === 2) retried.resolve();
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const secondStore = createSessionStore(
    secondClient as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  secondStore.getState().initSession(
    "worker-dm",
    "Worker DM",
    undefined,
    "hive",
  );
  await secondStore.getState().loadSession("worker-dm", true);
  for (let index = 0; index < 30 && secondAttempt === 0; index += 1) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assertEquals(
    secondStore.getState().queuedMessages.length,
    0,
    "a fresh idle Worker graph claims its older queue before newer input",
  );
  assert(
    secondStore.getState().messages.some((message) =>
      message.content.includes("restart-safe queued input")
    ),
    "the fresh graph restores the original unsent row",
  );

  await secondStore.getState().sendMessage(
    "accepted turn that releases recovery",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await retried.promise;
  assertEquals(
    secondAttempt,
    2,
    "the recovered queue dispatches exactly once before the new turn",
  );
  assertEquals(
    secondKeys[0],
    originalQueuedKey,
    "restart retry preserves the original Worker idempotency identity",
  );
  assertEquals(
    secondStore.getState().messages.filter((message) =>
      message.content.includes("restart-safe queued input")
    ).length,
    1,
    "acceptance commits one exact optimistic row without duplication",
  );
  secondStore.getState().cleanup();
});

Deno.test("a detached queued Worker successor restores only when its exact session reopens", async () => {
  const firstTransport = deferred<void>();
  const firstAttached = deferred<StreamCallbacks>();
  const successorTransport = deferred<void>();
  const successorAttached = deferred<StreamCallbacks>();
  let attempt = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
    ) => {
      attempt += 1;
      if (attempt === 1) {
        firstAttached.resolve(callbacks);
        await firstTransport.promise;
        return;
      }
      if (attempt >= 3) {
        callbacks.onWorkerResponsePending?.({
          type: "worker_response_pending",
          worker_id: "worker-1",
          session_id: "worker-a",
          run_id: `reopened-run-${attempt}`,
        });
        callbacks.onFinish("worker-a", "completed");
        return;
      }
      successorAttached.resolve(callbacks);
      await successorTransport.promise;
    },
    getSessionState: async (sessionId: string) =>
      sessionState("idle", sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first A turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const firstCallbacks = await firstAttached.promise;
  await store.getState().sendMessage(
    "private queued A input",
    [{
      name: "a.txt",
      type: "file",
      mimeType: "text/plain",
      text: "A only",
    }],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  acceptWorkerInput(firstCallbacks, "worker-a", "detached-seed-run");
  firstCallbacks.onFinish("worker-a", "completed");
  firstTransport.resolve();
  await firstSend;
  const staleSuccessorCallbacks = await successorAttached.promise;

  store.getState().initSession("worker-b", "Worker B", undefined, "hive");
  staleSuccessorCallbacks.onFinish("worker-a", "completed");
  successorTransport.resolve();
  await new Promise((resolve) => setTimeout(resolve, 10));

  assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
  assertEquals(
    store.getState().messages.length,
    0,
    "B never receives A's queued row",
  );
  assertEquals(
    store.getState().queuedMessages.length,
    0,
    "B never receives A's payload",
  );

  await store.getState().loadSession("worker-a", true);
  for (let index = 0; index < 30 && attempt < 3; index += 1) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assertEquals(store.getState().sessionId, "worker-a", "A reopens explicitly");
  assertEquals(
    store.getState().queuedMessages.length,
    0,
    "the detached payload is reclaimed only after A reopens",
  );
  assertEquals(
    store.getState().messages.filter((message) =>
      message.content.includes("private queued A input") && !message.isQueued
    ).length,
    1,
    "A's original optimistic row is reclaimed once on its exact session",
  );
  store.getState().cleanup();
});

Deno.test("Worker steering retains rejected intent and rotates for changed or accepted input", async () => {
  const requests: Array<{
    sessionId: string;
    message: string;
    key: string;
  }> = [];
  let attempt = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    steerSession: async (
      request: { session_id: string; message: string },
      options?: { idempotencyKey?: string },
    ) => {
      const key = options?.idempotencyKey;
      assert(key, "direct Worker steering must carry an idempotency key");
      requests.push({
        sessionId: request.session_id,
        message: request.message,
        key,
      });
      attempt += 1;
      if (attempt === 1) throw new Error("connection reset after send");
      if (attempt === 2) {
        throw new MitsuroApiError(422, "steering rejected", "rejected");
      }
      return { status: "accepted", pending_id: `pending-${attempt}` };
    },
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("worker-dm", "Worker DM", undefined, "hive");
  store.setState({ isStreaming: true });

  await store.getState().sendMessage("same direction").catch(() => {});
  await store.getState().sendMessage("same direction").catch(() => {});
  assertEquals(
    requests[1]?.key,
    requests[0]?.key,
    "transport uncertainty and a definitive rejection retain the same intent key",
  );

  await store.getState().sendMessage("changed direction");
  assert(
    requests[2]?.key !== requests[1]?.key,
    "changed content must receive a distinct key",
  );
  await store.getState().sendMessage("changed direction");
  assert(
    requests[3]?.key !== requests[2]?.key,
    "success clears the accepted key before the same content is sent again",
  );

  store.getState().initSession(
    "worker-dm-2",
    "Other Worker",
    undefined,
    "hive",
  );
  store.setState({ isStreaming: true });
  await store.getState().sendMessage("changed direction");
  assert(
    requests[4]?.key !== requests[3]?.key,
    "a changed session must receive a distinct key",
  );
  store.getState().cleanup();
});

Deno.test("active Worker steer replays its persisted operation and key after restart", async () => {
  const storage = createStorage();
  let originalKey = "";
  const firstClient = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    steerSession: async (
      _request: unknown,
      options?: { idempotencyKey?: string },
    ) => {
      originalKey = options?.idempotencyKey ?? "";
      throw new Error("connection lost after steer write");
    },
  };
  const firstStore = createSessionStore(
    firstClient as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  firstStore.getState().initSession("worker-dm", "Worker", undefined, "hive");
  firstStore.setState({ isStreaming: true });
  await firstStore.getState().sendMessage(
    "persist this steer",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  ).catch(() => {});
  assert(originalKey, "the original steer carries a durable exact key");
  firstStore.getState().cleanup();

  let replayKey = "";
  let replayOperation = "";
  const secondClient = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    steerSession: async (
      request: { message: string },
      options?: { idempotencyKey?: string },
    ) => {
      replayOperation = request.message;
      replayKey = options?.idempotencyKey ?? "";
      return { status: "accepted", pending_id: "replayed-steer" };
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const secondStore = createSessionStore(
    secondClient as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  secondStore.getState().initSession(
    "worker-dm",
    "Worker",
    undefined,
    "hive",
  );
  await secondStore.getState().loadSession("worker-dm", true);
  for (let index = 0; index < 30 && !replayKey; index += 1) {
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
  assertEquals(replayOperation, "persist this steer", "restart retains steer");
  assertEquals(replayKey, originalKey, "restart reuses the exact steer key");
  secondStore.getState().cleanup();
});

Deno.test("late Worker steer 404/409 cannot queue or resend A input into B", async () => {
  for (
    const [status, workerBStreaming] of [
      [404, false],
      [409, true],
    ] as const
  ) {
    const response = deferred<{
      status: "accepted";
      pending_id: string;
    }>();
    const steerStarted = deferred<void>();
    let streamCalls = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) =>
        workerBinding(sessionId),
      steerSession: () => {
        steerStarted.resolve();
        return response.promise;
      },
      streamChat: async () => {
        streamCalls += 1;
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );
    store.getState().initSession("worker-a", "Worker A", undefined, "hive");
    store.setState({ isStreaming: true });

    const pending = store.getState().sendMessage(`private A ${status}`);
    await steerStarted.promise;
    store.getState().initSession("worker-b", "Worker B", undefined, "hive");
    store.setState({ isStreaming: workerBStreaming });
    response.reject(new MitsuroApiError(status, "late steer race", "conflict"));
    await pending.catch(() => {});

    assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
    assertEquals(
      store.getState().queuedMessages.length,
      0,
      `late ${status} must not queue Worker A input under Worker B`,
    );
    assert(
      !store.getState().messages.some((message) =>
        message.content.includes("private A")
      ),
      `late ${status} must not render Worker A input under Worker B`,
    );
    assertEquals(
      streamCalls,
      0,
      `late ${status} must not start a replacement Worker B turn`,
    );
    assertEquals(store.getState().error, null, "B must not inherit A's race");
    store.getState().cleanup();
  }
});

Deno.test("late Worker A steer error cannot overwrite Worker B state", async () => {
  const response = deferred<{
    status: "accepted";
    pending_id: string;
  }>();
  const steerStarted = deferred<void>();
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    steerSession: () => {
      steerStarted.resolve();
      return response.promise;
    },
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  store.setState({ isStreaming: true });

  const pending = store.getState().sendMessage("private A failure");
  await steerStarted.promise;
  store.getState().initSession("worker-b", "Worker B", undefined, "hive");
  response.reject(new Error("Worker A transport failed"));
  await pending.catch(() => {});

  assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
  assertEquals(store.getState().error, null, "B must not inherit A's error");
  assertEquals(
    store.getState().messages.length,
    0,
    "B transcript must not be mutated by A's late error",
  );
  store.getState().cleanup();
});

Deno.test("non-conversation lane conflict rejects Worker input without claiming it was queued", async () => {
  const keys: string[] = [];
  const conflict =
    "Worker direct message is blocked by non-conversation run workflow-1 (worker_workflow)";
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    steerSession: async (
      _request: { session_id: string; message: string },
      options?: { idempotencyKey?: string },
    ) => {
      const key = options?.idempotencyKey;
      assert(key, "direct Worker steering must carry an idempotency key");
      keys.push(key);
      throw new MitsuroApiError(409, conflict, conflict);
    },
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("worker-dm", "Worker DM", undefined, "hive");
  store.setState({ isStreaming: true });

  for (let attempt = 0; attempt < 2; attempt += 1) {
    let rejected = false;
    try {
      await store.getState().sendMessage("retain this exact draft");
    } catch (error) {
      rejected = error instanceof MitsuroApiError && error.status === 409;
    }
    assert(
      rejected,
      "the definitive busy conflict must reject to the composer",
    );
    assertEquals(
      store.getState().queuedMessages.length,
      0,
      "the rejected input must not enter the local queue",
    );
    assert(
      !store.getState().messages.some((message) =>
        message.content.includes("retain this exact draft")
      ),
      "the rejected input must not remain as a claimed staged message",
    );
    assert(
      store.getState().error?.includes(
        `Message was not sent. API 409: ${conflict}`,
      ),
      "the client must truthfully report that the Worker input was not sent",
    );
  }
  assertEquals(
    keys[1],
    keys[0],
    "a later retry of the same unsent intent must retain its exact key",
  );
  store.getState().cleanup();
});

Deno.test("non-streaming non-conversation conflict restores the exact Worker draft", async () => {
  const keys: string[] = [];
  const conflict =
    "Worker direct message is blocked by non-conversation run workflow-2 (worker_workflow)";
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      const key = options?.idempotencyKey;
      assert(key, "direct Worker chat must carry an idempotency key");
      keys.push(key);
      callbacks.onError(`API 409: ${conflict}`);
    },
    getSessionState: async () => sessionState("idle"),
    getSession: async () => sessionResponse(),
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("worker-dm", "Worker DM", undefined, "hive");

  for (let attempt = 0; attempt < 2; attempt += 1) {
    let rejected = false;
    try {
      await store.getState().sendMessage("restore this exact draft");
    } catch (error) {
      rejected = error instanceof Error && error.message.includes(conflict);
    }
    assert(rejected, "the stream conflict must reject to the Worker composer");
    assertEquals(
      store.getState().messages.length,
      0,
      "neither optimistic user text nor an empty assistant may survive",
    );
    assertEquals(
      store.getState().isStreaming,
      false,
      "the definitively rejected request must not remain streaming",
    );
    assert(
      store.getState().error?.includes(
        `Message was not sent. API 409: ${conflict}`,
      ),
      "the direct chat must report that the draft was not sent",
    );
  }
  assertEquals(
    keys[1],
    keys[0],
    "retrying the same unsent draft must preserve its idempotency identity",
  );
  store.getState().cleanup();
});

Deno.test("late non-streaming Worker A busy conflict cannot overwrite Worker B", async () => {
  const response = deferred<void>();
  const attached = deferred<StreamCallbacks>();
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      nextCallbacks: StreamCallbacks,
    ) => {
      attached.resolve(nextCallbacks);
      return response.promise;
    },
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const pending = store.getState().sendMessage("private A busy input");
  const callbacks = await attached.promise;

  store.getState().initSession("worker-b", "Worker B", undefined, "hive");
  callbacks.onError(
    "API 409: Worker direct message is blocked by non-conversation run workflow-a (worker_workflow)",
  );
  response.resolve();
  let rejected = false;
  try {
    await pending;
  } catch {
    rejected = true;
  }

  assert(rejected, "A's unaccepted draft must reject after detaching from B");
  assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
  assertEquals(
    store.getState().messages.length,
    0,
    "B transcript is untouched",
  );
  assertEquals(store.getState().error, null, "B must not inherit A's conflict");
  assertEquals(
    store.getState().isStreaming,
    false,
    "B stream state is untouched",
  );
  store.getState().cleanup();
});

Deno.test("ordinary Chat keeps the pre-idempotency stream contract", async () => {
  const optionsSeen: unknown[] = [];
  let classifierCalls = 0;
  const client = {
    getHiveWorkerBySession: async () => {
      classifierCalls += 1;
      return workerBinding("ordinary-chat");
    },
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      optionsSeen.push(options);
      callbacks.onFinish("ordinary-chat", "completed");
    },
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );
  store.getState().initSession("ordinary-chat", "Chat", undefined, "chat");

  await store.getState().sendMessage("hello");
  assertEquals(
    classifierCalls,
    0,
    "ordinary Chat must not invoke Hive classification",
  );
  assertEquals(optionsSeen[0], undefined, "ordinary Chat must not send a key");
  store.getState().cleanup();
});

Deno.test("typed primary Hive keeps the ordinary stream contract", async () => {
  const optionsSeen: unknown[] = [];
  let classifierCalls = 0;
  const client = {
    ensureHiveMain: async () => ({ session_id: "primary-hive" }),
    getHiveWorkerBySession: async () => {
      classifierCalls += 1;
      return { kind: "primary_hive", session_id: "primary-hive" };
    },
    getSession: async () => sessionResponse("primary-hive"),
    getSessionState: async () => sessionState("idle", "primary-hive"),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      optionsSeen.push(options);
      callbacks.onFinish("primary-hive", "completed");
    },
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
    updateSession: async () => ({}),
    setCurrentModel: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
  );

  await store.getState().ensureHiveMainSession();
  await store.getState().sendMessage("hello");
  assertEquals(
    classifierCalls,
    0,
    "the typed main-session result should classify primary Hive without a probe",
  );
  assertEquals(
    optionsSeen[0],
    undefined,
    "typed primary Hive must keep the ordinary unkeyed stream contract",
  );
  store.getState().cleanup();
});
