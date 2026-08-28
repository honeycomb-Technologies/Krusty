import { MitsuroApiError, type StreamCallbacks } from "@mitsuro/api";
import { createSessionStore } from "../src/session/store.ts";
import { QueuedSuccessorRecovery } from "../src/session/queuedSuccessorRecovery.ts";
import type {
  ChatMessage,
  QueuedMessage,
  SessionDeletionAdmission,
} from "../src/session/types.ts";

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

class ControlledStorage {
  private readonly data = new Map<string, string>();
  private writesUnavailable = false;
  private nextReadGate: {
    started: ReturnType<typeof deferred<void>>;
    released: ReturnType<typeof deferred<void>>;
  } | null = null;
  private nextWriteGate: {
    started: ReturnType<typeof deferred<void>>;
    released: ReturnType<typeof deferred<void>>;
  } | null = null;

  get(key: string) {
    return this.data.get(key) ?? null;
  }

  set(key: string, value: string) {
    this.data.set(key, value);
  }

  delete(key: string) {
    this.data.delete(key);
  }

  getDurableSync(key: string) {
    return this.get(key);
  }

  async getDurable(key: string) {
    const gate = this.nextReadGate;
    if (gate) {
      this.nextReadGate = null;
      gate.started.resolve();
      await gate.released.promise;
    }
    return this.get(key);
  }

  async setDurable(key: string, value: string) {
    if (this.writesUnavailable) throw new Error("durable write unavailable");
    const gate = this.nextWriteGate;
    if (gate) {
      this.nextWriteGate = null;
      gate.started.resolve();
      await gate.released.promise;
    }
    this.set(key, value);
  }

  async deleteDurable(key: string) {
    if (this.writesUnavailable) throw new Error("durable delete unavailable");
    this.delete(key);
  }

  setWritesUnavailable(unavailable: boolean) {
    this.writesUnavailable = unavailable;
  }

  delayNextWrite() {
    const gate = {
      started: deferred<void>(),
      released: deferred<void>(),
    };
    this.nextWriteGate = gate;
    return {
      started: gate.started.promise,
      release: () => gate.released.resolve(),
    };
  }

  delayNextRead() {
    const gate = {
      started: deferred<void>(),
      released: deferred<void>(),
    };
    this.nextReadGate = gate;
    return {
      started: gate.started.promise,
      release: () => gate.released.resolve(),
    };
  }
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

function sessionState(sessionId: string) {
  return {
    id: sessionId,
    agent_state: "idle",
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

function sessionResponse(sessionId: string) {
  return {
    session: {
      id: sessionId,
      title: sessionId,
      token_count: 0,
      working_dir: null,
      project_dir: null,
      workspace_mode: "neutral",
      session_type: "hive",
      parent_session_id: null,
      mode: "build",
      permission_mode: "autonomous",
      updated_at: "2026-08-26T00:00:00Z",
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
      id: `worker-${sessionId}`,
      revision: 1,
      slug: sessionId,
      display_name: sessionId,
      permission_mode: "autonomous",
      autonomy: "continuous",
      status: "active",
      created_at: "2026-08-26T00:00:00Z",
      updated_at: "2026-08-26T00:00:00Z",
      attention: [],
    },
  };
}

function richAttachment(name: string) {
  return [{
    name,
    type: "file" as const,
    mimeType: "text/plain",
    text: name,
  }];
}

function recoveryRow(message: QueuedMessage): ChatMessage {
  return {
    id: message.id,
    role: "user",
    content: message.content,
    isQueued: true,
  };
}

function acceptWorkerInput(
  callbacks: StreamCallbacks,
  sessionId: string,
  runId: string,
): void {
  callbacks.onWorkerResponsePending?.({
    type: "worker_response_pending",
    worker_id: `worker-${sessionId}`,
    session_id: sessionId,
    run_id: runId,
  });
}

async function waitUntil(
  condition: () => boolean,
  message: string,
): Promise<void> {
  for (let index = 0; index < 100; index += 1) {
    if (condition()) return;
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
  throw new Error(message);
}

Deno.test("rich follow-up is durable before its predecessor finishes", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const transport = deferred<void>();
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
    ) => {
      attached.resolve(callbacks);
      await transport.promise;
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await attached.promise;
  await store.getState().sendMessage(
    "must survive before finish",
    richAttachment("durable.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );

  const restarted = new QueuedSuccessorRecovery(storage, "hive");
  await restarted.ready();
  const pending = restarted.get("worker-a");
  assert(pending, "the active response tail must already be durable");
  assertEquals(
    pending.phase,
    "in_flight",
    "the direct Worker turn itself has durable transport authority",
  );
  assertEquals(
    restarted.tail("worker-a")[0]?.content,
    "must survive before finish",
    "the exact private payload survives",
  );
  restarted.dispose();

  store.getState().cleanup();
  transport.resolve();
  await firstSend.catch(() => {});
});

Deno.test("navigation during durable claim never dispatches A through B", async () => {
  const storage = new ControlledStorage();
  const firstAttached = deferred<StreamCallbacks>();
  const firstTransport = deferred<void>();
  const requests: string[] = [];
  let attempt = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { message: string; session_id?: string },
      callbacks: StreamCallbacks,
    ) => {
      attempt += 1;
      requests.push(`${request.session_id}:${request.message}`);
      if (attempt === 1) {
        firstAttached.resolve(callbacks);
        await firstTransport.promise;
        return;
      }
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-worker-a",
        session_id: request.session_id ?? "worker-a",
        run_id: `accepted-${attempt}`,
      });
      callbacks.onFinish(request.session_id ?? "worker-a", "completed");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
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
  const callbacks = await firstAttached.promise;
  await store.getState().sendMessage(
    "A only queued input",
    richAttachment("a.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );

  acceptWorkerInput(callbacks, "worker-a", "first-a-run");
  await new Promise((resolve) => setTimeout(resolve, 0));
  const delayedClaim = storage.delayNextWrite();
  callbacks.onFinish("worker-a", "completed");
  await delayedClaim.started;
  store.getState().initSession("worker-b", "Worker B", undefined, "hive");
  delayedClaim.release();
  firstTransport.resolve();
  await firstSend;
  await new Promise((resolve) => setTimeout(resolve, 10));

  assertEquals(
    attempt,
    1,
    "claim preparation cannot start A transport after B wins",
  );
  assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
  assertEquals(store.getState().messages.length, 0, "A never writes into B");

  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    inspection.get("worker-a")?.phase,
    "pending",
    "the undispatched A payload returns to a safe pending phase",
  );
  inspection.dispose();

  await store.getState().loadSession("worker-a", true);
  await waitUntil(() => attempt === 2, "reopening A should resume its payload");
  assertEquals(
    requests[1],
    "worker-a:A only queued input",
    "the exact queued request resumes only through A",
  );
  assert(
    store.getState().messages.some((message) =>
      message.content.includes("A only queued input") && !message.isQueued
    ),
    "accepted A input remains visible exactly once",
  );
  store.getState().cleanup();
});

Deno.test("a newer same-session send stays behind a claim persistence await", async () => {
  const storage = new ControlledStorage();
  const firstAttached = deferred<StreamCallbacks>();
  const firstTransport = deferred<void>();
  const requests: string[] = [];
  let steerCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { message: string; session_id?: string },
      callbacks: StreamCallbacks,
    ) => {
      requests.push(request.message);
      if (requests.length === 1) {
        firstAttached.resolve(callbacks);
        await firstTransport.promise;
        return;
      }
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-worker-a",
        session_id: request.session_id ?? "worker-a",
        run_id: `accepted-${requests.length}`,
      });
      callbacks.onFinish(request.session_id ?? "worker-a", "completed");
    },
    steerSession: async () => {
      steerCalls += 1;
      throw new Error("the durable tail must not steer around the claim");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const callbacks = await firstAttached.promise;
  await store.getState().sendMessage(
    "older queued input",
    richAttachment("older.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );

  acceptWorkerInput(callbacks, "worker-a", "first-turn-run");
  await new Promise((resolve) => setTimeout(resolve, 0));
  const delayedClaim = storage.delayNextWrite();
  callbacks.onFinish("worker-a", "completed");
  await delayedClaim.started;
  const newerSend = store.getState().sendMessage(
    "newer queued input",
    richAttachment("newer.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  delayedClaim.release();
  firstTransport.resolve();
  await Promise.all([firstSend, newerSend]);
  await waitUntil(
    () => requests.length === 3,
    "the durable tail should run second",
  );

  assertEquals(steerCalls, 0, "claim persistence never opens a steering lane");
  assertEquals(requests[1], "older queued input", "the older batch runs first");
  assertEquals(requests[2], "newer queued input", "the newer tail runs second");
  assertEquals(store.getState().queuedMessages.length, 0, "both claims settle");
  for (const content of ["older queued input", "newer queued input"]) {
    assertEquals(
      store.getState().messages.filter((message) =>
        message.content.includes(content) && !message.isQueued
      ).length,
      1,
      `${content} commits exactly once`,
    );
  }
  store.getState().cleanup();
});

Deno.test("rich then plain follow-ups remain in durable chronological order", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const transport = deferred<void>();
  let steerCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (_request: unknown, callbacks: StreamCallbacks) => {
      attached.resolve(callbacks);
      await transport.promise;
    },
    steerSession: async () => {
      steerCalls += 1;
      throw new Error("newer input must not steer around a durable tail");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await attached.promise;
  await store.getState().sendMessage(
    "rich Q1",
    richAttachment("q1.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await store.getState().sendMessage(
    "plain Q2",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );

  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(steerCalls, 0, "Q2 never bypasses Q1 through steering");
  assertEquals(
    inspection.tail("worker-a").map((message) => message.content)
      .join("|"),
    "rich Q1|plain Q2",
    "the durable queue preserves user order",
  );
  inspection.dispose();
  store.getState().cleanup();
  transport.resolve();
  await firstSend.catch(() => {});
});

Deno.test("an append awaiting durable reload fences a newer idle send", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const transport = deferred<void>();
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (_request: unknown, callbacks: StreamCallbacks) => {
      streamCalls += 1;
      attached.resolve(callbacks);
      await transport.promise;
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await attached.promise;

  const appendGate = storage.delayNextRead();
  const olderAppend = store.getState().sendMessage(
    "older Q1",
    richAttachment("q1.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await appendGate.started;
  store.setState({ isStreaming: false, isLoading: false });

  let rejected = false;
  try {
    await store.getState().sendMessage(
      "newer Q2",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
  } catch {
    rejected = true;
  }
  assert(rejected, "Q2 must wait while Q1 is becoming durable");
  assertEquals(streamCalls, 1, "Q2 cannot open an overtaking transport");

  appendGate.release();
  await olderAppend;
  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    inspection.tail("worker-a")[0]?.content,
    "older Q1",
    "the first chronological input remains authoritative",
  );
  inspection.dispose();
  store.getState().cleanup();
  transport.resolve();
  await firstSend.catch(() => {});
});

Deno.test("deletion admission drains an older append and rejects newer input", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const transport = deferred<void>();
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (_request: unknown, callbacks: StreamCallbacks) => {
      streamCalls += 1;
      attached.resolve(callbacks);
      await transport.promise;
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "active response",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await attached.promise;

  const appendGate = storage.delayNextRead();
  const olderAppend = store.getState().sendMessage(
    "append racing deletion",
    richAttachment("delete-race.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await appendGate.started;

  const admissionPromise = store.getState().beginSessionDeletionAdmission(
    "worker-a",
  );
  let duplicateAdmissionRejected = false;
  try {
    await store.getState().beginSessionDeletionAdmission("worker-a");
  } catch {
    duplicateAdmissionRejected = true;
  }
  assert(
    duplicateAdmissionRejected,
    "a second deletion transport cannot share the held exact-session lease",
  );
  let newerRejected = false;
  try {
    await store.getState().sendMessage(
      "input after admission",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
  } catch {
    newerRejected = true;
  }
  assert(newerRejected, "admission rejects every newer input synchronously");
  assertEquals(streamCalls, 1, "no deletion-racing input reaches transport");

  appendGate.release();
  let olderRejected = false;
  try {
    await olderAppend;
  } catch {
    olderRejected = true;
  }
  assert(olderRejected, "the append already draining into admission is fenced");
  const admission = await admissionPromise;
  admission.commit();
  assert(
    !store.getState().messages.some((message) =>
      message.content.includes("append racing deletion")
    ),
    "the fenced optimistic prompt is removed from live presentation state",
  );
  assertEquals(
    store.getState().queuedMessages.length,
    0,
    "no volatile queued payload survives the deletion admission",
  );

  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    inspection.get("worker-a"),
    null,
    "the durable scrub leaves no prompt, attachment, or Worker key",
  );
  inspection.dispose();

  store.getState().cleanup();
  transport.resolve();
  await firstSend.catch(() => {});
});

Deno.test("reverse delete settlement requires a newly admitted transport", async () => {
  const storage = new ControlledStorage();
  const seeded = new QueuedSuccessorRecovery(storage, "hive");
  const privateMessage: QueuedMessage = {
    id: "delete-settlement-secret",
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "delete-settlement-policy",
      key: "delete-settlement-key",
    },
    content: "secret queued prompt",
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  await seeded.appendPending(
    "worker-delete-race",
    privateMessage,
    recoveryRow(privateMessage),
  );
  seeded.dispose();

  const store = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  const admissionPromise = store.getState().beginSessionDeletionAdmission(
    "worker-delete-race",
  );
  let duplicateRejected = false;
  try {
    await store.getState().beginSessionDeletionAdmission("worker-delete-race");
  } catch {
    duplicateRejected = true;
  }
  assert(
    duplicateRejected,
    "a concurrent caller cannot start a second DELETE on the same lease",
  );
  const firstAdmission = await admissionPromise;
  await firstAdmission.rollback();
  const restored = new QueuedSuccessorRecovery(storage, "hive");
  await restored.ready();
  assertEquals(
    restored.get("worker-delete-race")?.queuedMessages[0]?.content,
    privateMessage.content,
    "the sole failed DELETE restores its exact private payload",
  );
  assert(
    !restored.isDeletionAdmitted("worker-delete-race"),
    "the completed rollback releases its one transport lease",
  );
  restored.dispose();

  const secondAdmission = await store.getState().beginSessionDeletionAdmission(
    "worker-delete-race",
  );
  secondAdmission.commit();

  const restarted = new QueuedSuccessorRecovery(storage, "hive");
  await restarted.ready();
  assertEquals(
    restarted.get("worker-delete-race"),
    null,
    "a later successful DELETE uses a new scrubbed lease and cannot resurrect payload",
  );
  restarted.dispose();
  store.getState().cleanup();
});

Deno.test("failed rollback commit stays fenced until a new delete repairs it", async () => {
  const storage = new ControlledStorage();
  const seeded = new QueuedSuccessorRecovery(storage, "hive");
  const privateMessage: QueuedMessage = {
    id: "delete-settlement-retry",
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "delete-settlement-retry-policy",
      key: "delete-settlement-retry-key",
    },
    content: "retryable private prompt",
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  await seeded.appendPending(
    "worker-delete-retry",
    privateMessage,
    recoveryRow(privateMessage),
  );
  seeded.dispose();
  const store = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  const admission = await store.getState().beginSessionDeletionAdmission(
    "worker-delete-retry",
  );

  storage.setWritesUnavailable(true);
  const failedRollback = admission.rollback();
  assertEquals(
    admission.rollback(),
    failedRollback,
    "concurrent rollback callers share the failing attempt",
  );
  let rejected = false;
  try {
    await failedRollback;
  } catch {
    rejected = true;
  }
  assert(rejected, "the durable rollback failure is surfaced");
  const failedObserver = new QueuedSuccessorRecovery(storage, "hive");
  assert(
    failedObserver.isDeletionAdmitted("worker-delete-retry"),
    "failed rollback remains fail-closed",
  );
  await failedObserver.ready();
  failedObserver.dispose();

  storage.setWritesUnavailable(false);
  admission.commit();
  const stillFenced = new QueuedSuccessorRecovery(storage, "hive");
  assert(
    stillFenced.isDeletionAdmitted("worker-delete-retry"),
    "a stale commit cannot release an unresolved rollback",
  );
  await stillFenced.ready();
  stillFenced.dispose();

  const nextAdmission = await store.getState().beginSessionDeletionAdmission(
    "worker-delete-retry",
  );
  nextAdmission.commit();
  const committed = new QueuedSuccessorRecovery(storage, "hive");
  await committed.ready();
  assertEquals(
    committed.get("worker-delete-retry"),
    null,
    "new begin repairs rollback, re-scrubs, then owns the successful commit",
  );
  assert(
    !committed.isDeletionAdmitted("worker-delete-retry"),
    "the new committed lease releases admission",
  );
  committed.dispose();
  store.getState().cleanup();
});

Deno.test("replacement graph repairs a failed rollback before new deletion", async () => {
  const storage = new ControlledStorage();
  const seeded = new QueuedSuccessorRecovery(storage, "hive");
  const privateMessage: QueuedMessage = {
    id: "delete-replacement-repair",
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "delete-replacement-policy",
      key: "delete-replacement-key",
    },
    content: "replacement must repair this private prompt",
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  await seeded.appendPending(
    "worker-delete-replacement",
    privateMessage,
    recoveryRow(privateMessage),
  );
  seeded.dispose();

  const firstStore = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  const firstAdmission = await firstStore.getState()
    .beginSessionDeletionAdmission("worker-delete-replacement");
  storage.setWritesUnavailable(true);
  let rollbackRejected = false;
  try {
    await firstAdmission.rollback();
  } catch {
    rollbackRejected = true;
  }
  assert(rollbackRejected, "the original rollback failure is observable");
  firstStore.getState().cleanup();

  storage.setWritesUnavailable(false);
  const replacementStore = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  const replacementAdmission = await replacementStore.getState()
    .beginSessionDeletionAdmission("worker-delete-replacement");
  const whileRenewed = new QueuedSuccessorRecovery(storage, "hive");
  await whileRenewed.ready();
  assertEquals(
    whileRenewed.get("worker-delete-replacement"),
    null,
    "replacement begin restores then freshly scrubs before transport",
  );
  assert(
    whileRenewed.isDeletionAdmitted("worker-delete-replacement"),
    "the replacement transport receives the still-fenced renewed lease",
  );
  whileRenewed.dispose();

  replacementAdmission.commit();
  const restarted = new QueuedSuccessorRecovery(storage, "hive");
  await restarted.ready();
  assertEquals(
    restarted.get("worker-delete-replacement"),
    null,
    "replacement commit cannot recover the old graph's private payload",
  );
  assert(
    !restarted.isDeletionAdmitted("worker-delete-replacement"),
    "replacement commit releases the renewed admission",
  );
  restarted.dispose();
  replacementStore.getState().cleanup();
});

Deno.test("a replacement repair loser self-heals after the winner settles", async () => {
  const storage = new ControlledStorage();
  const seeded = new QueuedSuccessorRecovery(storage, "hive");
  const privateMessage: QueuedMessage = {
    id: "delete-replacement-loser",
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "delete-replacement-loser-policy",
      key: "delete-replacement-loser-key",
    },
    content: "replacement loser must not strand this private prompt",
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  await seeded.appendPending(
    "worker-delete-replacement-loser",
    privateMessage,
    recoveryRow(privateMessage),
  );
  seeded.dispose();

  const firstStore = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  const firstAdmission = await firstStore.getState()
    .beginSessionDeletionAdmission("worker-delete-replacement-loser");
  storage.setWritesUnavailable(true);
  let rollbackRejected = false;
  try {
    await firstAdmission.rollback();
  } catch {
    rollbackRejected = true;
  }
  assert(rollbackRejected, "the original rollback failure is observable");
  firstStore.getState().cleanup();
  storage.setWritesUnavailable(false);

  const repairLoser = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  const repairWinner = createSessionStore(
    {} as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  repairLoser.getState().initSession(
    "worker-delete-replacement-loser",
    "Worker replacement loser",
    undefined,
    "hive",
  );

  const winnerAdmission = deferred<SessionDeletionAdmission>();
  let reentered = false;
  const unsubscribe = repairLoser.subscribe(() => {
    if (reentered) return;
    reentered = true;
    void repairWinner.getState().beginSessionDeletionAdmission(
      "worker-delete-replacement-loser",
    ).then(winnerAdmission.resolve, winnerAdmission.reject);
  });

  let loserRejected = false;
  try {
    await repairLoser.getState().beginSessionDeletionAdmission(
      "worker-delete-replacement-loser",
    );
  } catch {
    loserRejected = true;
  }
  unsubscribe();
  assert(reentered, "the reentrant store wins shared repair");
  assert(loserRejected, "the store that lost repair authority rejects");
  const winningLease = await winnerAdmission.promise;
  winningLease.commit();

  const freshAdmission = await repairLoser.getState()
    .beginSessionDeletionAdmission("worker-delete-replacement-loser");
  freshAdmission.commit();
  const restarted = new QueuedSuccessorRecovery(storage, "hive");
  await restarted.ready();
  assertEquals(
    restarted.get("worker-delete-replacement-loser"),
    null,
    "the settled loser can acquire a genuinely fresh scrubbed lease",
  );
  assert(
    !restarted.isDeletionAdmitted("worker-delete-replacement-loser"),
    "the fresh successful transport releases the replacement admission",
  );
  restarted.dispose();
  repairLoser.getState().cleanup();
  repairWinner.getState().cleanup();
});

Deno.test("a delayed steer fallback keeps its earlier slot ahead of a rich tail", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const transport = deferred<void>();
  const steer = deferred<never>();
  const steerStarted = deferred<void>();
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (_request: unknown, callbacks: StreamCallbacks) => {
      attached.resolve(callbacks);
      await transport.promise;
    },
    steerSession: async () => {
      steerStarted.resolve();
      return await steer.promise;
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await attached.promise;
  const earlierPlain = store.getState().sendMessage(
    "plain Q1",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await Promise.resolve();
  await store.getState().sendMessage(
    "rich Q2",
    richAttachment("q2.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await steerStarted.promise;
  steer.reject(new MitsuroApiError(409, "late steer race", "conflict"));
  await earlierPlain;

  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    inspection.tail("worker-a").map((message) => message.content)
      .join("|"),
    "plain Q1|rich Q2",
    "the fallback uses its pre-await ordering reservation",
  );
  inspection.dispose();
  store.getState().cleanup();
  transport.resolve();
  await firstSend.catch(() => {});
});

Deno.test("an idle presentation snapshot cannot bypass a durable claim", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const predecessor = deferred<void>();
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (_request: unknown, callbacks: StreamCallbacks) => {
      streamCalls += 1;
      if (streamCalls === 1) {
        attached.resolve(callbacks);
        await predecessor.promise;
      }
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const callbacks = await attached.promise;
  await store.getState().sendMessage(
    "older queued",
    richAttachment("older.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  acceptWorkerInput(callbacks, "worker-a", "idle-snapshot-run");
  await new Promise((resolve) => setTimeout(resolve, 0));
  const delayedClaim = storage.delayNextWrite();
  callbacks.onFinish("worker-a", "completed");
  await delayedClaim.started;
  store.setState({ isStreaming: false, isLoading: false });
  let rejected = false;
  try {
    await store.getState().sendMessage(
      "must not overtake",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
  } catch {
    rejected = true;
  }
  assert(rejected, "presentation idle cannot open an independent transport");
  assertEquals(streamCalls, 1, "only the predecessor transport exists");
  delayedClaim.release();
  predecessor.resolve();
  await firstSend;
  store.getState().cleanup();
});

Deno.test("Stop during claim preparation prevents queued transport", async () => {
  const storage = new ControlledStorage();
  const attached = deferred<StreamCallbacks>();
  const predecessor = deferred<void>();
  const cancel = deferred<void>();
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (_request: unknown, callbacks: StreamCallbacks) => {
      streamCalls += 1;
      if (streamCalls === 1) {
        attached.resolve(callbacks);
        await predecessor.promise;
      }
    },
    cancelSession: async () => await cancel.promise,
    getHiveSessionStatus: async () => ({
      status: "cancelled",
      current_run_id: null,
    }),
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const callbacks = await attached.promise;
  await store.getState().sendMessage(
    "queued before Stop",
    richAttachment("stop.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  acceptWorkerInput(callbacks, "worker-a", "stop-before-claim-run");
  await new Promise((resolve) => setTimeout(resolve, 0));
  const delayedClaim = storage.delayNextWrite();
  callbacks.onFinish("worker-a", "completed");
  await delayedClaim.started;
  store.getState().stopStreaming({
    expectedSessionId: "worker-a",
    hiveConversationKind: "worker_dm",
  });
  delayedClaim.release();
  predecessor.resolve();
  await firstSend;
  await new Promise((resolve) => setTimeout(resolve, 5));
  assertEquals(streamCalls, 1, "Stop wins before queued transport starts");
  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    inspection.get("worker-a")?.phase,
    "pending",
    "undispatched queued input stays safely retryable",
  );
  inspection.dispose();
  store.getState().cleanup();
  cancel.resolve();
});

Deno.test("Stop abandons an already-started queued successor without replay", async () => {
  const storage = new ControlledStorage();
  const firstAttached = deferred<StreamCallbacks>();
  const predecessor = deferred<void>();
  const successorStarted = deferred<void>();
  const cancel = deferred<void>();
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      signal: AbortSignal,
    ) => {
      streamCalls += 1;
      if (streamCalls === 1) {
        firstAttached.resolve(callbacks);
        await predecessor.promise;
        return;
      }
      successorStarted.resolve();
      if (signal.aborted) return;
      await new Promise<void>((resolve) => {
        signal.addEventListener("abort", () => resolve(), { once: true });
      });
    },
    cancelSession: async () => await cancel.promise,
    getHiveSessionStatus: async () => ({
      status: "cancelled",
      current_run_id: null,
    }),
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  const firstSend = store.getState().sendMessage(
    "first turn",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  const callbacks = await firstAttached.promise;
  await store.getState().sendMessage(
    "queued successor to stop",
    richAttachment("stop-successor.txt"),
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  acceptWorkerInput(callbacks, "worker-a", "stop-successor-seed-run");
  callbacks.onFinish("worker-a", "completed");
  predecessor.resolve();
  await firstSend;
  await successorStarted.promise;
  store.getState().stopStreaming({
    expectedSessionId: "worker-a",
    hiveConversationKind: "worker_dm",
  });
  await waitUntil(
    () => !store.getState().messages.some((message) => message.isQueued),
    "stopped successor should leave no replayable local row",
  );
  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    inspection.get("worker-a"),
    null,
    "explicit Stop removes the in-flight queued recovery record",
  );
  assertEquals(streamCalls, 2, "the stopped successor is never replayed");
  inspection.dispose();
  store.getState().cleanup();
  cancel.resolve();
});

Deno.test("accepted replay reconciles one canonical attachment turn", async () => {
  const storage = new ControlledStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "hive");
  const attachment = richAttachment("accepted.txt")[0];
  const message: QueuedMessage = {
    id: "queued-accepted-before-crash",
    canonicalUserCountBefore: 0,
    content: "accepted before crash",
    attachments: [attachment],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  const localRow: ChatMessage = {
    id: message.id,
    role: "user",
    content: "accepted before crash\n\n[Attachments: accepted.txt]",
    attachments: [{
      type: "file",
      name: "accepted.txt",
      mimeType: "text/plain",
    }],
    isQueued: true,
  };
  await recovery.appendPending("worker-a", message, localRow);
  const active = await recovery.claim(
    { id: "ignored", sessionId: "worker-a", queuedMessages: [message] },
    [localRow],
  );
  await recovery.markInFlight("worker-a", active.id, active.attemptToken, {
    fingerprint: '{"session_id":"worker-a","thinking_enabled":"medium"}',
    key: "accepted-before-crash-key",
  });
  recovery.dispose();

  let replayCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { session_id?: string },
      callbacks: StreamCallbacks,
    ) => {
      replayCalls += 1;
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-worker-a",
        session_id: request.session_id ?? "worker-a",
        run_id: "accepted-replay",
      });
      callbacks.onFinish(request.session_id ?? "worker-a", "completed");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => ({
      ...sessionResponse(sessionId),
      messages: [{
        role: "user",
        content: [{
          type: "text",
          text: "accepted before crash\n\n--- accepted.txt ---\naccepted.txt",
        }],
      }],
    }),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");
  await store.getState().loadSession("worker-a", true);
  await waitUntil(() => replayCalls === 1, "the exact keyed replay should run");
  await waitUntil(
    () => store.getState().queuedMessages.length === 0,
    "accepted recovery should settle",
  );
  assertEquals(
    store.getState().messages.filter((entry) => entry.role === "user").length,
    1,
    "canonical history replaces the local attachment row exactly once",
  );
  store.getState().cleanup();
});

Deno.test("ordinary uncertain recovery is actionable immediately after hydration", async () => {
  const storage = new ControlledStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "chat");
  const message: QueuedMessage = {
    id: "ordinary-uncertain",
    content: "possibly delivered ordinary input",
    attachments: [],
    sendOptions: { sessionType: "chat" },
  };
  const row: ChatMessage = {
    id: message.id,
    role: "user",
    content: message.content,
    isQueued: true,
  };
  await recovery.appendPending("chat-a", message, row);
  const claim = await recovery.claim(
    { id: "ignored", sessionId: "chat-a", queuedMessages: [message] },
    [row],
  );
  await recovery.markInFlight("chat-a", claim.id, claim.attemptToken);
  await recovery.reject("chat-a", claim.id, claim.attemptToken);
  recovery.dispose();

  const client = {
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "chat",
  );
  store.getState().initSession("chat-a", "Chat A", undefined, "chat");
  await store.getState().loadSession("chat-a", true);

  assert(store.getState().queuedRecoveryBlocked, "uncertain replay is blocked");
  assert(
    store.getState().error?.includes("may already have been delivered"),
    "Retry and Discard are exposed before another Send is attempted",
  );
  assertEquals(
    store.getState().queuedMessages.length,
    0,
    "ordinary uncertain payload is never auto-replayed",
  );
  store.getState().cleanup();
});

Deno.test("repeated canonical text keeps both distinct user turns", async () => {
  const storage = new ControlledStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "hive");
  const message: QueuedMessage = {
    id: "queued-repeat",
    canonicalUserCountBefore: 1,
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: '{"session_id":"worker-a","thinking_enabled":"medium"}',
      key: "repeat-key",
    },
    content: "repeat",
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  await recovery.appendPending("worker-a", message, recoveryRow(message));
  recovery.dispose();

  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      request: { session_id?: string },
      callbacks: StreamCallbacks,
    ) => {
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-worker-a",
        session_id: request.session_id ?? "worker-a",
        run_id: "repeat-replay",
      });
      callbacks.onFinish(request.session_id ?? "worker-a", "completed");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => ({
      ...sessionResponse(sessionId),
      messages: [
        { role: "user", content: [{ type: "text", text: "repeat" }] },
        { role: "user", content: [{ type: "text", text: "repeat" }] },
      ],
    }),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker", undefined, "hive");
  await store.getState().loadSession("worker-a", true);
  await waitUntil(
    () => store.getState().queuedMessages.length === 0,
    "the replay should settle",
  );
  assertEquals(
    store.getState().messages.filter((entry) => entry.role === "user").length,
    2,
    "identical text is reconciled by canonical turn count, never content",
  );
  store.getState().cleanup();
});

Deno.test("unsupported Worker images never transport and queued claims release", async () => {
  const storage = new ControlledStorage();
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async () => {
      streamCalls += 1;
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const direct = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  direct.getState().initSession("worker-direct", "Worker", undefined, "hive");
  let directRejected = false;
  try {
    await direct.getState().sendMessage("bad image", [{
      name: "photo.heic",
      type: "image",
      mimeType: "image/heic",
      base64: "AAAA",
    }], { sessionType: "hive", hiveConversationKind: "worker_dm" });
  } catch {
    directRejected = true;
  }
  assert(directRejected, "unsupported direct image rejects to the composer");
  assertEquals(streamCalls, 0, "unsupported direct image has zero transport");
  direct.getState().cleanup();

  const recovery = new QueuedSuccessorRecovery(storage, "hive");
  const queuedImage: QueuedMessage = {
    id: "queued-heic",
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: '{"session_id":"worker-queued"}',
      key: "queued-heic-key",
    },
    content: "queued bad image",
    attachments: [{
      name: "queued.heic",
      type: "image",
      mimeType: "image/heic",
      base64: "BBBB",
    }],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
  await recovery.appendPending(
    "worker-queued",
    queuedImage,
    recoveryRow(queuedImage),
  );
  recovery.dispose();
  const queued = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  queued.getState().initSession("worker-queued", "Worker", undefined, "hive");
  await queued.getState().loadSession("worker-queued", true);
  await new Promise((resolve) => setTimeout(resolve, 10));
  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  assertEquals(
    streamCalls,
    0,
    "unsupported recovered image has zero transport",
  );
  assertEquals(
    inspection.get("worker-queued")?.phase,
    "pending",
    "the rejected queued image returns to a safely retryable pending claim",
  );
  assert(
    queued.getState().error?.includes("not supported"),
    "queued rejection remains actionable",
  );
  inspection.dispose();
  queued.getState().cleanup();
});

Deno.test("foreign Worker boundary plus exact finish cannot accept the queue", async () => {
  const storage = new ControlledStorage();
  let sentKey = "";
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    streamChat: async (
      _request: unknown,
      callbacks: StreamCallbacks,
      _signal: AbortSignal,
      options?: { idempotencyKey?: string },
    ) => {
      sentKey = options?.idempotencyKey ?? "";
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-worker-b",
        session_id: "worker-b",
        run_id: "foreign-run",
      });
      callbacks.onWorkerResponseCommitted?.({
        type: "worker_response_committed",
        worker_id: "worker-worker-b",
        session_id: "worker-b",
        run_id: "foreign-run",
      });
      callbacks.onFinish("worker-a", "completed");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker A", undefined, "hive");

  let rejected = false;
  try {
    await store.getState().sendMessage(
      "must await A's exact Worker boundary",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
  } catch {
    rejected = true;
  }
  assert(rejected, "a foreign boundary cannot authenticate Worker A");
  assert(sentKey, "the attempted Worker turn carries an exact key");
  assertEquals(
    store.getState().sessionId,
    "worker-a",
    "a foreign boundary cannot switch the selected conversation",
  );

  const inspection = new QueuedSuccessorRecovery(storage, "hive");
  await inspection.ready();
  const retained = inspection.get("worker-a");
  assert(retained, "the unaccepted Worker A queue remains recoverable");
  assertEquals(
    retained.phase,
    "rejected",
    "the exact keyed turn remains eligible for a safe retry",
  );
  assertEquals(
    retained.queuedMessages[0]?.workerInput?.key,
    sentKey,
    "failure retains the original server idempotency key",
  );
  assertEquals(
    inspection.get("worker-b"),
    null,
    "the foreign boundary never acquires queue ownership",
  );
  assert(
    store.getState().queuedMessages.some((message) =>
      message.content.includes("must await A's exact Worker boundary") &&
      message.workerInput?.key === sentKey
    ),
    "the live queue retains the same Worker draft and key for retry",
  );
  inspection.dispose();
  store.getState().cleanup();
});

Deno.test("idle steer race releases its own lock before one Chat fallback", async () => {
  const storage = new ControlledStorage();
  const steerStarted = deferred<void>();
  const steer = deferred<never>();
  let steerCalls = 0;
  let streamCalls = 0;
  const client = {
    getHiveWorkerBySession: async (sessionId: string) =>
      workerBinding(sessionId),
    steerSession: async () => {
      steerCalls += 1;
      steerStarted.resolve();
      return await steer.promise;
    },
    streamChat: async (
      request: { session_id?: string },
      callbacks: StreamCallbacks,
    ) => {
      streamCalls += 1;
      callbacks.onWorkerResponsePending?.({
        type: "worker_response_pending",
        worker_id: "worker-worker-a",
        session_id: request.session_id ?? "worker-a",
        run_id: "fallback-run",
      });
      callbacks.onWorkerResponseCommitted?.({
        type: "worker_response_committed",
        worker_id: "worker-worker-a",
        session_id: request.session_id ?? "worker-a",
        run_id: "fallback-run",
      });
      callbacks.onFinish(request.session_id ?? "worker-a", "completed");
    },
    getSessionState: async (sessionId: string) => sessionState(sessionId),
    getSession: async (sessionId: string) => sessionResponse(sessionId),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
  };
  const store = createSessionStore(
    client as never,
    storage,
    createWorkspace() as never,
    createSessionsStore() as never,
    createPlanStore() as never,
    "hive",
  );
  store.getState().initSession("worker-a", "Worker", undefined, "hive");
  store.setState({ isStreaming: true });
  const pending = store.getState().sendMessage(
    "fallback once",
    [],
    { sessionType: "hive", hiveConversationKind: "worker_dm" },
  );
  await steerStarted.promise;
  store.setState({ isStreaming: false, isLoading: false });
  steer.reject(new MitsuroApiError(409, "run already ended", "conflict"));
  await pending;
  await waitUntil(() => streamCalls === 1, "Chat fallback should start");
  assertEquals(steerCalls, 1, "the original steer is attempted exactly once");
  assertEquals(streamCalls, 1, "the fallback cannot reject itself on its lock");
  store.getState().cleanup();
});
