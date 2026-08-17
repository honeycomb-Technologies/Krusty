import { createSessionsStore, type SessionListItem } from "../src/sessions.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(
      `${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`,
    );
  }
}

function session(id: string, extra: Partial<SessionListItem> = {}): SessionListItem {
  return {
    id,
    title: id,
    updated_at: "2026-08-16T12:00:00Z",
    session_type: "chat",
    ...extra,
  };
}

function createWorkspace() {
  let sessionId: string | null = null;
  return {
    getState: () => ({
      sessionId,
      directory: null,
      mode: "neutral" as const,
      targetBranch: null,
      clear: () => {
        sessionId = null;
      },
      setWorkspace: (
        _directory: string | null,
        nextSessionId: string | null,
      ) => {
        sessionId = nextSessionId;
      },
    }),
  };
}

function createClient(options: {
  sessions: SessionListItem[];
  getSessions?: () => Promise<SessionListItem[]>;
  deleteSession?: (id: string) => Promise<void>;
}) {
  return {
    getSessions: options.getSessions ??
      (async () => options.sessions.slice()),
    deleteSession: options.deleteSession ?? (async () => {}),
    getDirectories: async () => [],
    createSession: async () => {
      throw new Error("unused");
    },
    getSession: async () => {
      throw new Error("unused");
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

Deno.test("delete removes the row before the server answers", async () => {
  const alpha = session("alpha");
  const beta = session("beta");
  const deleteGate = deferred<void>();
  const client = createClient({
    sessions: [alpha, beta],
    deleteSession: () => deleteGate.promise,
  });
  const store = createSessionsStore(client as never, createWorkspace() as never);
  store.setState({ sessions: [alpha, beta] });

  const pending = store.getState().deleteSession("alpha");
  assertEquals(
    store.getState().sessions.map((item) => item.id).join(","),
    "beta",
    "the active list must drop the row immediately",
  );

  deleteGate.resolve();
  assertEquals(await pending, true, "delete must succeed after the server ack");
});

Deno.test("a stale list fetch cannot restore a deleted row", async () => {
  const alpha = session("alpha");
  const beta = session("beta");
  const loadGate = deferred<SessionListItem[]>();
  const client = createClient({
    sessions: [alpha, beta],
    getSessions: () => loadGate.promise,
    deleteSession: async () => {},
  });
  const store = createSessionsStore(client as never, createWorkspace() as never);
  store.setState({ sessions: [alpha, beta] });

  const loading = store.getState().loadSessions();
  const deleted = store.getState().deleteSession("alpha");
  assertEquals(
    store.getState().sessions.map((item) => item.id).join(","),
    "beta",
    "optimistic delete must win while a poll is in flight",
  );

  loadGate.resolve([alpha, beta]);
  await loading;
  await deleted;
  assertEquals(
    store.getState().sessions.map((item) => item.id).join(","),
    "beta",
    "the stale poll must not put the deleted row back",
  );
});

Deno.test("archive marks the row locally before the next poll", () => {
  const alpha = session("alpha");
  const store = createSessionsStore(
    createClient({ sessions: [alpha] }) as never,
    createWorkspace() as never,
  );
  store.setState({ sessions: [alpha] });
  store.getState().setSessionArchived("alpha", true);
  assert(
    Boolean(store.getState().sessions[0]?.archived_at),
    "archive must set archived_at immediately so the active list can hide it",
  );
});

Deno.test("a failed delete restores the previous row", async () => {
  const alpha = session("alpha");
  const store = createSessionsStore(
    createClient({
      sessions: [alpha],
      deleteSession: async () => {
        throw new Error("nope");
      },
    }) as never,
    createWorkspace() as never,
  );
  store.setState({ sessions: [alpha] });
  const deleted = await store.getState().deleteSession("alpha");
  assertEquals(deleted, false, "failed delete must report false");
  assertEquals(
    store.getState().sessions[0]?.id,
    "alpha",
    "the row must come back when the server rejects the delete",
  );
});
