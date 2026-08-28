import { MemoryStorage } from "../src/storage.ts";
import {
  QueuedSuccessorRecovery,
  type QueuedSuccessorRecoveryRecord,
} from "../src/session/queuedSuccessorRecovery.ts";
import type {
  ChatMessage,
  QueuedMessage,
  QueuedSuccessorClaimInput,
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

function queued(id: string, content = id): QueuedMessage {
  return {
    id,
    content,
    attachments: [],
    sendOptions: {
      sessionType: "hive",
      hiveConversationKind: "worker_dm",
    },
  };
}

function row(message: QueuedMessage): ChatMessage {
  return {
    id: message.id,
    role: "user",
    content: message.content,
    isQueued: true,
  };
}

function claim(
  id: string,
  sessionId: string,
  queuedMessages: QueuedMessage[],
): QueuedSuccessorClaimInput {
  return { id, sessionId, queuedMessages };
}

function namespacedDurableStorage(
  data: Map<string, string>,
  durableRecoveryNamespace: string,
  unavailable: () => boolean = () => false,
  durableWrites: string[] = [],
) {
  return {
    durableRecoveryNamespace,
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurableSync: (key: string) => data.get(key) ?? null,
    getDurable: async (key: string) => data.get(key) ?? null,
    setDurable: async (key: string, value: string) => {
      if (unavailable()) throw new Error("durable write unavailable");
      durableWrites.push(value);
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      if (unavailable()) throw new Error("durable delete unavailable");
      data.delete(key);
    },
  };
}

Deno.test("one durable Worker queue record survives restart with its exact identity", async () => {
  const storage = new MemoryStorage();
  const first = new QueuedSuccessorRecovery(storage, "hive-test");
  const original = queued("queued-a", "private queued input");
  await first.appendPending("worker-a", original, row(original));
  const initial = await first.claim(
    claim("claim-a", "worker-a", [original]),
    [row(original)],
  );
  await first.setWorkerIdentity("worker-a", initial.id, initial.attemptToken, {
    fingerprint: "request-fingerprint-a",
    key: "worker-input-fixed-a",
  });
  first.dispose();

  const restarted = new QueuedSuccessorRecovery(storage, "hive-test");
  await restarted.ready();
  const recovered = restarted.get("worker-a");
  assert(recovered, "the pending batch must survive a fresh store graph");
  assertEquals(
    recovered.phase,
    "uncertain",
    "a process death cannot claim an in-flight transport was rejected",
  );
  assertEquals(recovered.queuedMessages.length, 1, "payload remains exact");
  assertEquals(recovered.rows.length, 1, "optimistic row remains exact");
  assertEquals(
    recovered.workerInput?.key,
    "worker-input-fixed-a",
    "the Worker retry retains its server idempotency identity",
  );
  const retry = await restarted.claim(
    claim("ignored-retry-id", "worker-a", [original]),
    [row(original)],
  );
  assertEquals(
    retry.workerInput?.key,
    "worker-input-fixed-a",
    "the first mutation after restart cannot resurrect in_flight state",
  );
  restarted.dispose();
});

Deno.test("retry supersedes the same per-session record without layering a newer tail", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "hive-test");
  const original = queued("queued-a", "original uncertain batch");
  await recovery.appendPending("worker-a", original, row(original));
  const first = await recovery.claim(
    claim("claim-a", "worker-a", [original]),
    [row(original)],
  );
  await recovery.setWorkerIdentity("worker-a", first.id, first.attemptToken, {
    fingerprint: "fingerprint-a",
    key: "stable-key-a",
  });
  const rejected = await recovery.reject(
    "worker-a",
    first.id,
    first.attemptToken,
  );
  assertEquals(
    rejected?.phase,
    "rejected",
    "keyed Worker failure is retryable",
  );

  const newerTail = queued("queued-b", "newer tail waits its turn");
  await recovery.appendPending("worker-a", newerTail, row(newerTail));
  const retry = await recovery.claim(
    claim("claim-b", "worker-a", [original, newerTail]),
    [row(original), row(newerTail)],
  );
  assertEquals(retry.id, first.id, "retry reuses the one durable record");
  assertEquals(
    retry.queuedMessages.length,
    1,
    "a newer tail is not merged into an uncertain remote request",
  );
  assertEquals(
    retry.queuedMessages[0]?.id,
    original.id,
    "the exact original payload is retried first",
  );
  assertEquals(
    retry.workerInput?.key,
    "stable-key-a",
    "supersession preserves the exact Worker identity",
  );

  await recovery.reject("worker-a", retry.id, retry.attemptToken);
  const restarted = new QueuedSuccessorRecovery(storage, "hive-test");
  await restarted.ready();
  const records = [restarted.get("worker-a")].filter(
    (value): value is QueuedSuccessorRecoveryRecord => value !== null,
  );
  assertEquals(
    records.length,
    1,
    "restart hydrates one record, never a claim stack",
  );
  assertEquals(
    records[0].queuedMessages.length,
    2,
    "one rejected batch plus its newer tail survive without duplication",
  );
});

Deno.test("ordinary uncertain queue is retained without inventing a retry identity", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "chat-test");
  const message = queued("queued-chat", "ordinary chat payload");
  await recovery.appendPending("chat-a", message, row(message));
  const pending = await recovery.claim(
    claim("claim-chat", "chat-a", [message]),
    [row(message)],
  );
  const uncertain = await recovery.reject(
    "chat-a",
    pending.id,
    pending.attemptToken,
  );
  assertEquals(
    uncertain?.phase,
    "uncertain",
    "non-idempotent transport failure must not become an automatic retry",
  );
  assertEquals(
    uncertain?.workerInput,
    undefined,
    "ordinary Chat never fabricates Worker idempotency authority",
  );
  let replayBlocked = false;
  try {
    const newer = queued("queued-chat-new", "newer ordinary payload");
    await recovery.claim(
      claim("claim-chat-new", "chat-a", [newer]),
      [row(newer)],
    );
  } catch {
    replayBlocked = true;
  }
  assert(
    replayBlocked,
    "a later turn cannot silently replay an uncertain non-idempotent request",
  );

  const authorized = await recovery.retryOrdinaryUncertain("chat-a");
  assertEquals(
    authorized?.phase,
    "pending",
    "only explicit user authorization makes an ordinary batch retryable",
  );
  assertEquals(
    recovery.claimable("chat-a")[0]?.id,
    message.id,
    "explicit retry retains the exact original payload",
  );
  const discarded = await recovery.delete("chat-a");
  assertEquals(
    discarded?.queuedMessages[0]?.id,
    message.id,
    "discard is exact",
  );
  assertEquals(recovery.get("chat-a"), null, "discard removes private payload");
});

Deno.test("durable recovery fails closed instead of evicting a ninth pending session", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "bounded-test");
  for (let index = 0; index < 8; index += 1) {
    const message = queued(`queued-${index}`);
    await recovery.appendPending(`session-${index}`, message, row(message));
  }
  let rejected = false;
  try {
    const overflow = queued("queued-overflow");
    await recovery.appendPending(
      "session-overflow",
      overflow,
      row(overflow),
    );
  } catch {
    rejected = true;
  }
  assert(
    rejected,
    "a ninth pending session must fail instead of evicting a draft",
  );

  const restarted = new QueuedSuccessorRecovery(storage, "bounded-test");
  await restarted.ready();
  for (let index = 0; index < 8; index += 1) {
    assert(
      restarted.get(`session-${index}`),
      `bounded failure must preserve session-${index}`,
    );
  }
});

Deno.test("oversized rich queue remains live rather than entering partial durable state", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "size-test");
  const oversized = queued("queued-large");
  oversized.attachments = [{
    name: "large.txt",
    type: "file",
    mimeType: "text/plain",
    text: "x".repeat(400 * 1024),
  }];
  let rejected = false;
  try {
    await recovery.appendPending(
      "worker-large",
      oversized,
      row(oversized),
    );
  } catch {
    rejected = true;
  }
  assert(rejected, "oversized recovery must fail before dispatch");
  assertEquals(
    recovery.get("worker-large"),
    null,
    "failed persistence cannot leave a partial in-memory claim",
  );
});

Deno.test("pending input and a prepared-but-undispatched claim both restart as safely retryable", async () => {
  const storage = new MemoryStorage();
  const pendingRecovery = new QueuedSuccessorRecovery(storage, "pending-test");
  const pending = queued("queued-pending", "persist before predecessor finish");
  await pendingRecovery.appendPending("worker-a", pending, row(pending));

  let restarted = new QueuedSuccessorRecovery(storage, "pending-test");
  await restarted.ready();
  assertEquals(
    restarted.get("worker-a")?.phase,
    "pending",
    "a local tail is durably retryable before any transport claim",
  );

  await restarted.claim(
    claim("ignored-claim-id", "worker-a", [pending]),
    [row(pending)],
  );
  restarted.dispose();
  restarted = new QueuedSuccessorRecovery(storage, "pending-test");
  await restarted.ready();
  assertEquals(
    restarted.get("worker-a")?.phase,
    "pending",
    "a crash during claim preparation cannot become uncertain delivery",
  );
  assertEquals(
    restarted.claimable("worker-a")[0]?.id,
    pending.id,
    "the exact undispatched payload remains claimable",
  );
  const retry = await restarted.claim(
    claim("ignored-restart-id", "worker-a", [pending]),
    [row(pending)],
  );
  assertEquals(
    retry.phase,
    "claiming",
    "the first mutation after restart can claim the normalized pending phase",
  );
  restarted.dispose();
});

Deno.test("accepting one claimed batch atomically promotes its newer tail", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "promotion-test");
  const first = queued("queued-first", "first");
  const second = queued("queued-second", "second");
  await recovery.appendPending("worker-a", first, row(first));
  const active = await recovery.claim(
    claim("ignored", "worker-a", [first]),
    [row(first)],
  );
  await recovery.markInFlight("worker-a", active.id, active.attemptToken, {
    fingerprint: "fingerprint-first",
    key: "key-first",
  });
  await recovery.appendPending("worker-a", second, row(second));
  await recovery.accept("worker-a", active.id, active.attemptToken);

  const promoted = recovery.get("worker-a");
  assertEquals(promoted?.phase, "pending", "the later tail becomes pending");
  assertEquals(
    promoted?.queuedMessages.length,
    1,
    "only the later tail remains",
  );
  assertEquals(promoted?.queuedMessages[0]?.id, second.id, "order is exact");
  assertEquals(
    promoted?.workerInput,
    undefined,
    "the accepted key is not reused",
  );
});

Deno.test("reconnected recovery instances serialize the shared envelope", async () => {
  const storage = new MemoryStorage();
  const oldGraph = new QueuedSuccessorRecovery(storage, "reconnect-test");
  const newGraph = new QueuedSuccessorRecovery(storage, "reconnect-test");
  const first = queued("queued-a", "from old graph");
  const second = queued("queued-b", "from new graph");
  await oldGraph.appendPending("worker-a", first, row(first));
  await newGraph.appendPending("worker-b", second, row(second));

  const active = await oldGraph.claim(
    claim("ignored", "worker-a", [first]),
    [row(first)],
  );
  await oldGraph.markInFlight("worker-a", active.id, active.attemptToken, {
    fingerprint: "fingerprint-a",
    key: "key-a",
  });
  await oldGraph.accept("worker-a", active.id, active.attemptToken);

  const finalGraph = new QueuedSuccessorRecovery(storage, "reconnect-test");
  await finalGraph.ready();
  assertEquals(finalGraph.get("worker-a"), null, "accepted A is removed");
  assertEquals(
    finalGraph.get("worker-b")?.queuedMessages[0]?.id,
    second.id,
    "an old continuation cannot erase the new graph's B record",
  );
});

Deno.test("a disposed delivery attempt cannot mutate a newer retry", async () => {
  const storage = new MemoryStorage();
  const oldGraph = new QueuedSuccessorRecovery(storage, "attempt-owner-test");
  const message = queued("queued-a", "exact retry payload");
  await oldGraph.appendPending("worker-a", message, row(message));
  const oldAttempt = await oldGraph.claim(
    claim("ignored-old", "worker-a", [message]),
    [row(message)],
  );
  await oldGraph.markInFlight(
    "worker-a",
    oldAttempt.id,
    oldAttempt.attemptToken,
    { fingerprint: "fingerprint-old", key: "key-old" },
  );
  oldGraph.dispose();

  const newGraph = new QueuedSuccessorRecovery(storage, "attempt-owner-test");
  await newGraph.ready();
  const newAttempt = await newGraph.claim(
    claim("ignored-new", "worker-a", [message]),
    [row(message)],
  );

  assertEquals(
    await oldGraph.releaseUndispatched(
      "worker-a",
      oldAttempt.id,
      oldAttempt.attemptToken,
    ),
    null,
    "a disposed owner cannot reset the new claim to pending",
  );
  assertEquals(
    await oldGraph.reject(
      "worker-a",
      oldAttempt.id,
      oldAttempt.attemptToken,
    ),
    null,
    "a disposed owner cannot reject the new claim",
  );
  assertEquals(
    await oldGraph.accept(
      "worker-a",
      oldAttempt.id,
      oldAttempt.attemptToken,
    ),
    false,
    "a disposed owner cannot accept or delete the new claim",
  );

  const marked = await newGraph.markInFlight(
    "worker-a",
    newAttempt.id,
    newAttempt.attemptToken,
    { fingerprint: "fingerprint-new", key: "key-new" },
  );
  assert(marked, "the current attempt retains exclusive transport authority");
  assertEquals(
    newGraph.get("worker-a")?.workerInput?.key,
    "key-new",
    "the stale continuation cannot clear the current Worker identity",
  );
  assert(
    await newGraph.accept(
      "worker-a",
      newAttempt.id,
      newAttempt.attemptToken,
    ),
    "the current attempt can settle normally",
  );
  assertEquals(newGraph.get("worker-a"), null, "accepted retry is removed");
});

Deno.test("an unavailable recovery read cannot overwrite an existing envelope", async () => {
  const data = new Map<string, string>();
  let failedReads = 0;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurable: async (key: string) => {
      if (failedReads > 0) {
        failedReads -= 1;
        throw new Error("temporary recovery read failure");
      }
      return data.get(key) ?? null;
    },
    setDurable: async (key: string, value: string) => {
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      data.delete(key);
    },
  };
  const seeded = new QueuedSuccessorRecovery(storage, "read-failure-test");
  const original = queued("queued-original", "must survive read failure");
  await seeded.appendPending("worker-a", original, row(original));
  seeded.dispose();

  // Constructor hydration, explicit ready retry, and the mutation's mandatory
  // reload all fail independently. The write must never proceed from an empty
  // in-memory fallback after any of them.
  failedReads = 3;
  const unavailable = new QueuedSuccessorRecovery(
    storage,
    "read-failure-test",
  );
  await unavailable.ready().catch(() => {});
  let rejected = false;
  try {
    const newer = queued("queued-newer", "must not overwrite A");
    await unavailable.appendPending("worker-b", newer, row(newer));
  } catch {
    rejected = true;
  }
  assert(
    rejected,
    "mutation fails closed while the durable envelope is unreadable",
  );
  unavailable.dispose();

  const recovered = new QueuedSuccessorRecovery(storage, "read-failure-test");
  await recovered.ready();
  assertEquals(
    recovered.get("worker-a")?.queuedMessages[0]?.id,
    original.id,
    "the existing private payload survives the failed writer",
  );
  assertEquals(
    recovered.get("worker-b"),
    null,
    "no empty overwrite is persisted",
  );
});

Deno.test("accepted cleanup failure retains only a payload-free tombstone", async () => {
  const data = new Map<string, string>();
  let failDeletion = false;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurableSync: (key: string) => data.get(key) ?? null,
    getDurable: async (key: string) => data.get(key) ?? null,
    setDurable: async (key: string, value: string) => {
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      if (failDeletion) throw new Error("storage deletion failed");
      data.delete(key);
    },
  };
  const recovery = new QueuedSuccessorRecovery(storage, "accepted-tombstone");
  const privateMessage = queued(
    "private-accepted",
    "private content must not survive acceptance",
  );
  await recovery.appendPending(
    "worker-private",
    privateMessage,
    row(privateMessage),
  );
  const active = await recovery.claim(
    claim("ignored", "worker-private", [privateMessage]),
    [row(privateMessage)],
  );
  await recovery.markInFlight(
    "worker-private",
    active.id,
    active.attemptToken,
    {
      fingerprint: "private-fingerprint",
      key: "private-key",
    },
  );

  failDeletion = true;
  assert(
    await recovery.accept(
      "worker-private",
      active.id,
      active.attemptToken,
    ),
    "physical tombstone cleanup cannot turn accepted delivery into failure",
  );
  const raw = [...data.values()].join("\n");
  assert(
    raw.includes('"phase":"accepted"'),
    "the failed deletion leaves an inert acceptance tombstone",
  );
  assert(
    !raw.includes(privateMessage.content) &&
      !raw.includes("private-fingerprint") && !raw.includes("private-key"),
    "the tombstone retains no prompt or Worker identity material",
  );

  const restarted = new QueuedSuccessorRecovery(storage, "accepted-tombstone");
  await restarted.ready();
  assertEquals(
    restarted.get("worker-private"),
    null,
    "an accepted tombstone is never replayable after restart",
  );
});

Deno.test("a pre-dispose mutation is ordered before replacement hydration", async () => {
  const data = new Map<string, string>();
  let releaseFirstRead!: () => void;
  const firstRead = new Promise<void>((resolve) => {
    releaseFirstRead = resolve;
  });
  let readCount = 0;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurable: async (key: string) => {
      readCount += 1;
      if (readCount === 1) await firstRead;
      return data.get(key) ?? null;
    },
    setDurable: async (key: string, value: string) => {
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      data.delete(key);
    },
  };
  const oldGraph = new QueuedSuccessorRecovery(storage, "handoff-order");
  const message = queued("queued-before-dispose", "must reach replacement");
  const pendingAppend = oldGraph.appendPending(
    "worker-a",
    message,
    row(message),
  );
  oldGraph.dispose();
  const replacement = new QueuedSuccessorRecovery(storage, "handoff-order");
  releaseFirstRead();
  await pendingAppend;
  await replacement.ready();
  assertEquals(
    replacement.get("worker-a")?.queuedMessages[0]?.id,
    message.id,
    "replacement hydration waits behind the operation begun before disposal",
  );
});

Deno.test("replacement reads normalize an owner disposed after hydration", async () => {
  const storage = new MemoryStorage();
  const oldGraph = new QueuedSuccessorRecovery(storage, "overlap-normalize");
  const message = queued("queued-overlap", "ordinary uncertain payload");
  await oldGraph.appendPending("chat-a", message, row(message));
  const active = await oldGraph.claim(
    claim("ignored", "chat-a", [message]),
    [row(message)],
  );
  await oldGraph.markInFlight("chat-a", active.id, active.attemptToken);

  const replacement = new QueuedSuccessorRecovery(
    storage,
    "overlap-normalize",
  );
  await replacement.ready();
  assertEquals(
    replacement.get("chat-a")?.phase,
    "in_flight",
    "a genuinely live owner is preserved during overlap",
  );
  oldGraph.dispose();
  assertEquals(
    replacement.get("chat-a")?.phase,
    "uncertain",
    "the replacement lazily normalizes after the old owner disposes",
  );
  assert(
    replacement.isOrdinaryUncertain("chat-a"),
    "ordinary uncertain delivery becomes explicitly actionable",
  );
});

Deno.test("discard cleanup failure retains only a payload-free tombstone", async () => {
  const data = new Map<string, string>();
  let failDeletion = false;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurableSync: (key: string) => data.get(key) ?? null,
    getDurable: async (key: string) => data.get(key) ?? null,
    setDurable: async (key: string, value: string) => {
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      if (failDeletion) throw new Error("storage deletion failed");
      data.delete(key);
    },
  };
  const recovery = new QueuedSuccessorRecovery(storage, "discard-tombstone");
  const privateMessage = queued(
    "private-discarded",
    "private discarded content",
  );
  await recovery.appendPending(
    "deleted-session",
    privateMessage,
    row(privateMessage),
  );

  failDeletion = true;
  const discarded = await recovery.delete("deleted-session");
  assertEquals(
    discarded?.queuedMessages[0]?.id,
    privateMessage.id,
    "discard returns the exact removed record",
  );
  const raw = [...data.values()].join("\n");
  assert(
    raw.includes('"phase":"accepted"') &&
      !raw.includes(privateMessage.content),
    "failed physical deletion keeps only an inert payload-free tombstone",
  );
  const restarted = new QueuedSuccessorRecovery(storage, "discard-tombstone");
  await restarted.ready();
  assertEquals(
    restarted.get("deleted-session"),
    null,
    "discarded payload is never reachable after restart",
  );
});

Deno.test("remote acceptance survives the first local tombstone write failure", async () => {
  const data = new Map<string, string>();
  let storageUnavailable = false;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurableSync: (key: string) => data.get(key) ?? null,
    getDurable: async (key: string) => data.get(key) ?? null,
    setDurable: async (key: string, value: string) => {
      if (storageUnavailable) throw new Error("durable write unavailable");
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      if (storageUnavailable) throw new Error("durable delete unavailable");
      data.delete(key);
    },
  };
  const recovery = new QueuedSuccessorRecovery(storage, "remote-accept-fail");
  const message: QueuedMessage = {
    ...queued("remote-accepted", "accepted at Worker boundary"),
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "policy-a",
      key: "stable-remote-key",
    },
  };
  await recovery.appendPending("worker-a", message, row(message));
  const active = await recovery.claim(
    claim("ignored", "worker-a", [message]),
    [row(message)],
  );
  await recovery.markInFlight(
    "worker-a",
    active.id,
    active.attemptToken,
    message.workerInput,
  );

  storageUnavailable = true;
  assert(
    await recovery.acceptRemote(
      "worker-a",
      active.id,
      active.attemptToken,
    ),
    "the authoritative remote boundary wins over local tombstone failure",
  );
  assertEquals(
    recovery.get("worker-a"),
    null,
    "the live graph relinquishes replay authority immediately",
  );
  assertEquals(
    recovery.isDelivering("worker-a"),
    false,
    "failed local cleanup cannot retain a delivery lock",
  );

  const restarted = new QueuedSuccessorRecovery(
    storage,
    "remote-accept-fail",
  );
  await restarted.ready();
  assertEquals(
    restarted.get("worker-a")?.workerInput?.key,
    "stable-remote-key",
    "a restart retains the original key while cleanup remains unavailable",
  );
  restarted.dispose();

  storageUnavailable = false;
  const next = queued("next-input", "later input can secure a new slot");
  await recovery.appendPending("worker-a", next, row(next));
  assertEquals(
    recovery.get("worker-a")?.queuedMessages[0]?.id,
    next.id,
    "the accepted local mask permits a later durable input",
  );
  recovery.dispose();
});

Deno.test("remote acceptance write failure preserves a newer Worker tail", async () => {
  const data = new Map<string, string>();
  let storageUnavailable = false;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurableSync: (key: string) => data.get(key) ?? null,
    getDurable: async (key: string) => data.get(key) ?? null,
    setDurable: async (key: string, value: string) => {
      if (storageUnavailable) throw new Error("durable write unavailable");
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      if (storageUnavailable) throw new Error("durable delete unavailable");
      data.delete(key);
    },
  };
  const recovery = new QueuedSuccessorRecovery(
    storage,
    "remote-accept-tail-fail",
  );
  const first: QueuedMessage = {
    ...queued("remote-first", "accepted first Worker turn"),
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "policy-first",
      key: "stable-first-key",
    },
  };
  const second: QueuedMessage = {
    ...queued("remote-second", "newer Worker tail must survive"),
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "policy-second",
      key: "stable-second-key",
    },
  };
  await recovery.appendPending("worker-a", first, row(first));
  await recovery.appendPending("worker-a", second, row(second));
  const active = await recovery.claim(
    claim("ignored", "worker-a", [first, second]),
    [row(first), row(second)],
  );
  await recovery.markInFlight(
    "worker-a",
    active.id,
    active.attemptToken,
    first.workerInput,
  );

  storageUnavailable = true;
  assert(
    await recovery.acceptRemote(
      "worker-a",
      active.id,
      active.attemptToken,
    ),
    "the exact first Worker turn is remotely accepted",
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  const liveTail = recovery.get("worker-a");
  assertEquals(liveTail?.phase, "pending", "the tail becomes claimable");
  assertEquals(
    liveTail?.queuedMessages.length,
    1,
    "only the accepted active turn is masked",
  );
  assertEquals(
    liveTail?.queuedMessages[0]?.workerInput?.key,
    "stable-second-key",
    "the newer exact Worker identity remains live",
  );

  const restartedWhileUnavailable = new QueuedSuccessorRecovery(
    storage,
    "remote-accept-tail-fail",
  );
  await restartedWhileUnavailable.ready();
  assertEquals(
    restartedWhileUnavailable.get("worker-a")?.queuedMessages.length,
    2,
    "a restart retains both exact keys until idempotent cleanup can persist",
  );
  assertEquals(
    restartedWhileUnavailable.get("worker-a")?.queuedMessages[0]?.workerInput
      ?.key,
    "stable-first-key",
    "restart cleanup can safely replay the accepted turn under its old key",
  );
  restartedWhileUnavailable.dispose();

  storageUnavailable = false;
  const third = queued("remote-third", "later input after failed cleanup");
  await recovery.appendPending("worker-a", third, row(third));
  assertEquals(
    recovery.get("worker-a")?.queuedMessages.map((message) => message.id).join(
      ",",
    ),
    `${second.id},${third.id}`,
    "the first successful mutation persists the tail without resurrecting Q1",
  );
  const restartedAfterCleanup = new QueuedSuccessorRecovery(
    storage,
    "remote-accept-tail-fail",
  );
  await restartedAfterCleanup.ready();
  assertEquals(
    restartedAfterCleanup.get("worker-a")?.queuedMessages.map((message) =>
      message.id
    ).join(","),
    `${second.id},${third.id}`,
    "durable cleanup never deletes the unsent tail",
  );
  restartedAfterCleanup.dispose();
  recovery.dispose();
});

Deno.test("a pinched queue moves atomically from its original owner", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "pinch-owner");
  const message = queued("queued-a", "follow the pinch");
  await recovery.appendPending("session-a", message, row(message));
  const moved = await recovery.claim(
    {
      id: "ignored",
      sessionId: "session-b",
      sourceSessionId: "session-a",
      queuedMessages: [message],
    },
    [row(message)],
  );
  assertEquals(recovery.get("session-a"), null, "A no longer owns the queue");
  assertEquals(
    moved.sessionId,
    "session-b",
    "the claim is issued only under the validated pinch target",
  );
  await recovery.markInFlight(
    "session-b",
    moved.id,
    moved.attemptToken,
  );
  await recovery.accept("session-b", moved.id, moved.attemptToken);
  assertEquals(recovery.get("session-b"), null, "B settles the queue once");
});

Deno.test("deletion admission scrubs payload and rollback restores its exact identity", async () => {
  const storage = new MemoryStorage();
  const recovery = new QueuedSuccessorRecovery(storage, "delete-admission");
  const message: QueuedMessage = {
    ...queued("queued-delete", "private deletion-race payload"),
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "delete-policy",
      key: "delete-stable-key",
    },
  };
  await recovery.appendPending("worker-delete", message, row(message));

  recovery.acquireDeletionAdmission("worker-delete");
  const snapshot = await recovery.scrubForDeletion("worker-delete");
  assertEquals(
    snapshot?.queuedMessages[0]?.workerInput?.key,
    "delete-stable-key",
    "the lease retains the exact pre-delete retry identity for rollback",
  );
  assertEquals(
    recovery.get("worker-delete"),
    null,
    "the admitted live graph cannot read the scrubbed private payload",
  );
  let appendRejected = false;
  try {
    const raced = queued("queued-race", "must not survive deletion");
    await recovery.appendPending("worker-delete", raced, row(raced));
  } catch {
    appendRejected = true;
  }
  assert(appendRejected, "admission blocks a newer append before persistence");

  await recovery.rollbackDeletionAdmission("worker-delete", snapshot);
  assertEquals(
    recovery.get("worker-delete")?.queuedMessages[0]?.content,
    message.content,
    "failed server deletion restores the exact private payload",
  );
  assertEquals(
    recovery.get("worker-delete")?.queuedMessages[0]?.workerInput?.key,
    "delete-stable-key",
    "rollback never rotates the uncertain Worker identity",
  );
  const newer = queued("queued-after-rollback", "admission released");
  await recovery.appendPending("worker-delete", newer, row(newer));
  assertEquals(
    recovery.tail("worker-delete").length,
    2,
    "successful rollback reopens exact-session admission",
  );
  recovery.dispose();
});

Deno.test("failed deletion rollback stays admitted and retries the same snapshot", async () => {
  const data = new Map<string, string>();
  let storageUnavailable = false;
  const storage = {
    get: (key: string) => data.get(key) ?? null,
    set: (key: string, value: string) => data.set(key, value),
    delete: (key: string) => data.delete(key),
    getDurableSync: (key: string) => data.get(key) ?? null,
    getDurable: async (key: string) => data.get(key) ?? null,
    setDurable: async (key: string, value: string) => {
      if (storageUnavailable) throw new Error("durable write unavailable");
      data.set(key, value);
    },
    deleteDurable: async (key: string) => {
      if (storageUnavailable) throw new Error("durable delete unavailable");
      data.delete(key);
    },
  };
  const recovery = new QueuedSuccessorRecovery(
    storage,
    "delete-rollback-retry",
  );
  const message: QueuedMessage = {
    ...queued("queued-delete-retry", "restore me exactly"),
    workerOperation: "chat",
    workerInput: {
      operation: "chat",
      fingerprint: "delete-retry-policy",
      key: "delete-retry-key",
    },
  };
  await recovery.appendPending("worker-delete", message, row(message));
  recovery.acquireDeletionAdmission("worker-delete");
  const snapshot = await recovery.scrubForDeletion("worker-delete");

  storageUnavailable = true;
  let rollbackRejected = false;
  try {
    await recovery.rollbackDeletionAdmission("worker-delete", snapshot);
  } catch {
    rollbackRejected = true;
  }
  assert(
    rollbackRejected,
    "a failed durable restore is surfaced to the caller",
  );
  assert(
    recovery.isDeletionAdmitted("worker-delete"),
    "failed rollback keeps exact-session admission held",
  );
  let appendRejected = false;
  try {
    const raced = queued("queued-after-failed-rollback");
    await recovery.appendPending("worker-delete", raced, row(raced));
  } catch {
    appendRejected = true;
  }
  assert(
    appendRejected,
    "new input remains fenced while rollback is retryable",
  );

  storageUnavailable = false;
  await recovery.rollbackDeletionAdmission("worker-delete", snapshot);
  assertEquals(
    recovery.get("worker-delete")?.queuedMessages[0]?.workerInput?.key,
    "delete-retry-key",
    "retry restores the same exact Worker identity",
  );
  assert(
    !recovery.isDeletionAdmitted("worker-delete"),
    "only the successful rollback releases admission",
  );
  recovery.dispose();
});

Deno.test("deletion repair authority never crosses durable principals", async () => {
  const principalAData = new Map<string, string>();
  const principalBData = new Map<string, string>();
  const principalBWrites: string[] = [];
  let principalAUnavailable = false;
  const principalAStorage = namespacedDurableStorage(
    principalAData,
    "principal-a",
    () => principalAUnavailable,
  );
  const principalBStorage = namespacedDurableStorage(
    principalBData,
    "principal-b",
    undefined,
    principalBWrites,
  );
  const principalA = new QueuedSuccessorRecovery(
    principalAStorage,
    "principal-isolation",
  );
  const principalB = new QueuedSuccessorRecovery(
    principalBStorage,
    "principal-isolation",
  );
  const principalAMessage = queued(
    "principal-a-private",
    "principal A private deletion snapshot",
  );
  const principalBMessage = queued(
    "principal-b-private",
    "principal B private deletion snapshot",
  );
  await principalA.appendPending(
    "colliding-worker-session",
    principalAMessage,
    row(principalAMessage),
  );
  await principalB.appendPending(
    "colliding-worker-session",
    principalBMessage,
    row(principalBMessage),
  );

  principalA.acquireDeletionAdmission("colliding-worker-session");
  const principalASnapshot = await principalA.scrubForDeletion(
    "colliding-worker-session",
  );
  principalAUnavailable = true;
  let rollbackRejected = false;
  try {
    await principalA.rollbackDeletionAdmission(
      "colliding-worker-session",
      principalASnapshot,
    );
  } catch {
    rollbackRejected = true;
  }
  assert(rollbackRejected, "principal A retains a failed repair snapshot");
  assert(
    !principalB.canRepairFailedDeletionAdmission("colliding-worker-session"),
    "principal B cannot observe principal A repair authority",
  );

  principalB.acquireDeletionAdmission("colliding-worker-session");
  const principalBSnapshot = await principalB.scrubForDeletion(
    "colliding-worker-session",
  );
  assertEquals(
    principalBSnapshot?.queuedMessages[0]?.content,
    principalBMessage.content,
    "principal B scrubs only its own colliding session",
  );
  assert(
    principalBWrites.every((value) =>
      !value.includes(principalAMessage.content)
    ),
    "principal A private payload is never written into principal B storage",
  );
  await principalB.rollbackDeletionAdmission(
    "colliding-worker-session",
    principalBSnapshot,
  );
  principalAUnavailable = false;
  await principalA.rollbackDeletionAdmission(
    "colliding-worker-session",
    principalASnapshot,
  );
  principalA.dispose();
  principalB.dispose();
});

Deno.test("same durable principal replacement repairs the exact deletion", async () => {
  const sharedData = new Map<string, string>();
  let unavailable = false;
  const firstStorage = namespacedDurableStorage(
    sharedData,
    "same-principal",
    () => unavailable,
  );
  const first = new QueuedSuccessorRecovery(
    firstStorage,
    "principal-replacement",
  );
  const message = queued(
    "same-principal-private",
    "same principal replacement payload",
  );
  await first.appendPending("replacement-worker", message, row(message));
  first.acquireDeletionAdmission("replacement-worker");
  const snapshot = await first.scrubForDeletion("replacement-worker");
  unavailable = true;
  let rollbackRejected = false;
  try {
    await first.rollbackDeletionAdmission("replacement-worker", snapshot);
  } catch {
    rollbackRejected = true;
  }
  assert(rollbackRejected, "the original graph leaves a repairable rollback");
  first.dispose();

  unavailable = false;
  const replacementStorage = namespacedDurableStorage(
    sharedData,
    "same-principal",
    () => unavailable,
  );
  const replacement = new QueuedSuccessorRecovery(
    replacementStorage,
    "principal-replacement",
  );
  assert(
    replacement.canRepairFailedDeletionAdmission("replacement-worker"),
    "a new adapter for the same authority inherits exact repair ownership",
  );
  const renewedSnapshot = await replacement.renewFailedDeletionAdmission(
    "replacement-worker",
  );
  assertEquals(
    renewedSnapshot?.queuedMessages[0]?.content,
    message.content,
    "same-principal replacement restores then re-scrubs the exact snapshot",
  );
  replacement.commitDeletionAdmission("replacement-worker");

  const restarted = new QueuedSuccessorRecovery(
    namespacedDurableStorage(sharedData, "same-principal"),
    "principal-replacement",
  );
  await restarted.ready();
  assertEquals(
    restarted.get("replacement-worker"),
    null,
    "the replacement commit retains the durable scrub",
  );
  assert(
    !restarted.isDeletionAdmitted("replacement-worker"),
    "the same-principal replacement releases shared admission",
  );
  replacement.dispose();
  restarted.dispose();
});
