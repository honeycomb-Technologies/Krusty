import {
  genericSessionDeleteDisposition,
  runGenericSessionDeleteIfAllowed,
} from "../components/chat-screen/hiveSessionDeleteFence.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${
        JSON.stringify(actual)
      }`,
    );
  }
}

Deno.test("generic delete classification fails closed for Hive Worker DMs", () => {
  const knownWorkerDmIds = new Set(["known-worker-dm"]);
  assertEquals(
    genericSessionDeleteDisposition({
      sessionId: "known-worker-dm",
      sessionType: "hive",
      workerDmSessionIds: knownWorkerDmIds,
    }),
    "worker_dm",
    "the authoritative roster must protect a known Worker DM",
  );
  assertEquals(
    genericSessionDeleteDisposition({
      sessionId: "resolved-worker-dm",
      sessionType: "hive",
      bindingKind: "worker_dm",
    }),
    "worker_dm",
    "the exact session binding must protect an archived or unlisted Worker",
  );
  assertEquals(
    genericSessionDeleteDisposition({
      sessionId: "primary-hive",
      sessionType: "hive",
      bindingKind: "primary_hive",
    }),
    "allowed",
    "a proven primary Hive session keeps generic delete",
  );
  assertEquals(
    genericSessionDeleteDisposition({
      sessionId: "unresolved-hive",
      sessionType: "hive",
    }),
    "unresolved",
    "Hive binding uncertainty must fail closed",
  );
  assertEquals(
    genericSessionDeleteDisposition({
      sessionId: "ordinary-chat",
      sessionType: "chat",
    }),
    "allowed",
    "ordinary Chat deletion remains available",
  );
});

Deno.test("Worker DM delete preflight performs no local clear or generic delete", async () => {
  let localClears = 0;
  let genericDeletes = 0;
  const disposition = await runGenericSessionDeleteIfAllowed(
    {
      sessionId: "worker-dm",
      sessionType: "hive",
      resolveHiveBinding: async (sessionId) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: {} as never,
      }),
    },
    () => {
      localClears += 1;
      genericDeletes += 1;
    },
  );

  assertEquals(disposition, "worker_dm", "the Worker binding must be retained");
  assertEquals(localClears, 0, "Worker preflight must not clear local state");
  assertEquals(genericDeletes, 0, "Worker preflight must not issue DELETE");
});

Deno.test("Hive binding lookup uncertainty never reaches the delete boundary", async () => {
  let allowedActions = 0;
  const lookupFailure = await runGenericSessionDeleteIfAllowed(
    {
      sessionId: "lookup-failure",
      sessionType: "hive",
      resolveHiveBinding: () => Promise.reject(new Error("unavailable")),
    },
    () => {
      allowedActions += 1;
    },
  );
  const mismatchedBinding = await runGenericSessionDeleteIfAllowed(
    {
      sessionId: "requested-hive",
      sessionType: "hive",
      resolveHiveBinding: async () => ({
        kind: "primary_hive",
        session_id: "different-hive",
      }),
    },
    () => {
      allowedActions += 1;
    },
  );

  assertEquals(lookupFailure, "unresolved", "lookup errors must fail closed");
  assertEquals(
    mismatchedBinding,
    "unresolved",
    "a mismatched exact binding must fail closed",
  );
  assertEquals(
    allowedActions,
    0,
    "binding uncertainty must not clear local state or issue DELETE",
  );
});

Deno.test("primary Hive delete preflight preserves the allowed action", async () => {
  let allowedActions = 0;
  const disposition = await runGenericSessionDeleteIfAllowed(
    {
      sessionId: "primary-hive",
      sessionType: "hive",
      resolveHiveBinding: async (sessionId) => ({
        kind: "primary_hive",
        session_id: sessionId,
      }),
    },
    () => {
      allowedActions += 1;
    },
  );

  assertEquals(disposition, "allowed", "primary Hive must remain deletable");
  assertEquals(allowedActions, 1, "the allowed delete boundary must run once");
});

Deno.test("typed archived rows preserve Chat, Code, and primary Hive delete parity", async () => {
  const allowedSessionIds: string[] = [];
  for (const sessionType of ["chat", "code"] as const) {
    const sessionId = `archived-${sessionType}`;
    const disposition = await runGenericSessionDeleteIfAllowed(
      { sessionId, sessionType },
      () => {
        allowedSessionIds.push(sessionId);
      },
    );
    assertEquals(
      disposition,
      "allowed",
      `archived ${sessionType} must retain generic delete`,
    );
  }

  const primaryDisposition = await runGenericSessionDeleteIfAllowed(
    {
      sessionId: "archived-primary-hive",
      sessionType: "hive",
      resolveHiveBinding: async (sessionId) => ({
        kind: "primary_hive",
        session_id: sessionId,
      }),
    },
    () => {
      allowedSessionIds.push("archived-primary-hive");
    },
  );
  let workerActions = 0;
  const workerDisposition = await runGenericSessionDeleteIfAllowed(
    {
      sessionId: "archived-worker-dm",
      sessionType: "hive",
      resolveHiveBinding: async (sessionId) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: {} as never,
      }),
    },
    () => {
      workerActions += 1;
    },
  );

  assertEquals(primaryDisposition, "allowed", "primary Hive remains deletable");
  assertEquals(workerDisposition, "worker_dm", "Worker DM remains protected");
  assertEquals(
    allowedSessionIds,
    ["archived-chat", "archived-code", "archived-primary-hive"],
    "all typed archived generic rows must reach the allowed action",
  );
  assertEquals(workerActions, 0, "archived Worker DM must not reach delete");
});

Deno.test("drawer and shared action both consume the Worker delete fence", async () => {
  const [drawer, actions, desktopList, tabScreen] = await Promise.all([
    Deno.readTextFile(
      new URL("../components/chat/SessionDrawer.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL(
        "../components/chat-screen/useSessionActions.ts",
        import.meta.url,
      ),
    ),
    Deno.readTextFile(
      new URL("../components/chat/SessionList.tsx", import.meta.url),
    ),
    Deno.readTextFile(new URL("../app/(tabs)/index.tsx", import.meta.url)),
  ]);
  const compactTabScreen = tabScreen.replace(/\s+/g, " ");

  assertEquals(
    drawer.includes("genericDeleteAllowed\n              ? swipeAction") &&
      drawer.includes("if (!genericDeleteAllowed) return") &&
      drawer.includes("...(genericDeleteAllowed") &&
      drawer.includes(
        'chronologicalSessions(displaySessions, "hive").filter((session)',
      ) &&
      drawer.includes('}) !== "worker_dm"') &&
      drawer.includes("hiveSessionBindingSnapshot.client === client") &&
      drawer.includes(
        "hiveSessionBindingSnapshot.scopeKey === hiveSessionBindingScopeKey",
      ) &&
      drawer.includes("session.id,\n        session.session_type,") &&
      desktopList.includes(
        "onDeleteSession(item.id, item.session_type)",
      ) &&
      compactTabScreen.includes(
        'onDeleteRun={(id) => handleDeleteSession(id, "hive")}',
      ),
    true,
    "all row actions must pass exact type into the shared Worker authority",
  );
  assertEquals(
    actions.indexOf("runGenericSessionDeleteIfAllowed(") <
      actions.indexOf(
        'confirmDestructiveAction(\n            "Delete Session"',
      ),
    true,
    "binding preflight must precede confirmation, local clear, and DELETE",
  );
});
