import {
  beginAllModeSessionDeletionAdmission,
  clearDeletedSessionFromModeStoreGraphs,
  clearDeletedSessionFromModeStores,
  runSessionDeletionBatch,
  type SessionDeletionAdmission,
  type SessionDeletionModeStores,
  type SessionDeletionProjectionModeStores,
} from "../components/chat-screen/sessionDeletionAdmission.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function modeStores(
  begin: (mode: "chat" | "code" | "hive") => Promise<SessionDeletionAdmission>,
): SessionDeletionModeStores {
  return Object.fromEntries(
    (["chat", "code", "hive"] as const).map((mode) => [
      mode,
      {
        session: {
          getState: () => ({
            beginSessionDeletionAdmission: () => begin(mode),
          }),
        },
      },
    ]),
  ) as unknown as SessionDeletionModeStores;
}

Deno.test("all recovery producers are fenced before admission waits", async () => {
  const scrubMayFinish = deferred<void>();
  const events: string[] = [];
  const fenced = new Set<string>();
  let durablePayload = "private queued prompt";
  const stores = modeStores((mode) => {
    fenced.add(mode);
    events.push(`fence:${mode}`);
    return (async () => {
      await scrubMayFinish.promise;
      durablePayload = "";
      events.push(`scrub:${mode}`);
      return {
        commit: () => events.push(`commit:${mode}`),
        rollback: () => Promise.resolve(),
      };
    })();
  });

  const admissionPromise = beginAllModeSessionDeletionAdmission(
    stores,
    "session-delete",
  );
  assert(
    fenced.size === 3 &&
      events.slice(0, 3).every((event) => event.startsWith("fence:")),
    "every mode must install its synchronous fence before the first await",
  );
  const racingAppendAccepted = !fenced.has("hive");
  scrubMayFinish.resolve();
  const admission = await admissionPromise;
  assert(
    !racingAppendAccepted && durablePayload === "",
    "a racing producer must be rejected and its prompt absent before DELETE",
  );
  admission.commit();
  assert(
    events.filter((event) => event.startsWith("commit:")).length === 3,
    "server acceptance must commit every mode lease",
  );
});

Deno.test("partial admission failure rolls back every acquired mode", async () => {
  const rolledBack: string[] = [];
  const attempted: string[] = [];
  const stores = modeStores((mode) => {
    attempted.push(mode);
    if (mode === "code") throw new Error("scrub failed");
    return Promise.resolve({
      commit: () => {},
      rollback: async () => {
        rolledBack.push(mode);
      },
    });
  });

  let rejected = false;
  try {
    await beginAllModeSessionDeletionAdmission(stores, "session-failure");
  } catch {
    rejected = true;
  }
  assert(
    rejected && attempted.join(",") === "chat,code,hive" &&
      rolledBack.sort().join(",") === "chat,hive",
    "a synchronous scrub failure must still fence every mode, abort deletion, and release every successful admission",
  );
});

Deno.test("duplicate admission rejection preserves the original deletion owner", async () => {
  const held = new Set<string>();
  let originalCommits = 0;
  const stores = modeStores((mode) => {
    if (held.has(mode)) {
      throw new Error(`duplicate:${mode}`);
    }
    held.add(mode);
    return Promise.resolve({
      commit: () => {
        originalCommits += 1;
        held.delete(mode);
      },
      rollback: async () => {
        held.delete(mode);
      },
    });
  });

  const original = await beginAllModeSessionDeletionAdmission(
    stores,
    "session-duplicate",
  );
  let duplicateRejected = false;
  try {
    await beginAllModeSessionDeletionAdmission(stores, "session-duplicate");
  } catch {
    duplicateRejected = true;
  }
  assert(
    duplicateRejected && held.size === 3 && originalCommits === 0,
    "a concurrent caller must not acquire, settle, or release the original deletion owner's admission",
  );

  original.commit();
  assert(
    Number(held.size) === 0 && Number(originalCommits) === 3,
    "only the original owner may settle its three mode admissions",
  );
});

Deno.test("batch deletion stops at first failure and restores every unattempted session", async () => {
  const events: string[] = [];
  const admissions = new Map<string, SessionDeletionAdmission>(
    ["one", "two", "three"].map(
      (sessionId): [string, SessionDeletionAdmission] => [
        sessionId,
        {
          commit: () => events.push(`commit:${sessionId}`),
          rollback: async () => {
            events.push(`rollback:${sessionId}`);
          },
        },
      ],
    ),
  );

  const result = await runSessionDeletionBatch(
    ["one", "two", "three"],
    admissions,
    async (sessionId) => {
      events.push(`delete:${sessionId}`);
      return sessionId === "one";
    },
    () => true,
    (sessionId) => events.push(`detach:${sessionId}`),
  );

  assert(
    events.join(",") ===
      "delete:one,detach:one,commit:one,delete:two,rollback:two,rollback:three",
    "an accepted DELETE must detach its producer before commit, then the first failed DELETE must roll back every uncommitted admission",
  );
  assert(
    result.deletedIds.join(",") === "one" &&
      result.remainingIds.join(",") === "two,three" &&
      admissions.size === 0,
    "the caller must receive the exact committed and restored UI partitions",
  );
});

Deno.test("batch boundary replacement commits accepted deletes and restores the rest", async () => {
  const events: string[] = [];
  let current = true;
  const admissions = new Map<string, SessionDeletionAdmission>(
    ["one", "two"].map(
      (sessionId): [string, SessionDeletionAdmission] => [
        sessionId,
        {
          commit: () => events.push(`commit:${sessionId}`),
          rollback: async () => {
            events.push(`rollback:${sessionId}`);
          },
        },
      ],
    ),
  );

  const result = await runSessionDeletionBatch(
    ["one", "two"],
    admissions,
    async (sessionId) => {
      events.push(`delete:${sessionId}`);
      current = false;
      return true;
    },
    () => current,
    (sessionId) => events.push(`detach:${sessionId}`),
  );

  assert(
    result.boundaryChanged &&
      result.deletedIds.join(",") === "one" &&
      result.remainingIds.join(",") === "two" &&
      events.join(",") === "delete:one,detach:one,commit:one,rollback:two",
    "a replaced graph must settle the completed transport and release every untouched fence",
  );
});

Deno.test("deleted-session detach is exact across every mode", () => {
  const active = new Map([
    ["chat", "deleted"],
    ["code", "new-selection"],
    ["hive", "deleted"],
  ]);
  const cleared: string[] = [];
  const stores = Object.fromEntries(
    (["chat", "code", "hive"] as const).map((mode) => [
      mode,
      {
        session: {
          getState: () => ({
            sessionId: active.get(mode) ?? null,
            clearSession: () => {
              cleared.push(mode);
              active.set(mode, "");
            },
          }),
        },
      },
    ]),
  ) as unknown as SessionDeletionProjectionModeStores;

  assert(
    clearDeletedSessionFromModeStores(stores, "deleted") &&
      cleared.join(",") === "chat,hive" &&
      active.get("code") === "new-selection",
    "deletion must detach every exact producer without clearing a newer selection",
  );
});

Deno.test("deleted-session detach reaches a replacement graph before commit", () => {
  const graph = (codeSessionId: string) => {
    const active = new Map([
      ["chat", "deleted"],
      ["code", codeSessionId],
      ["hive", "deleted"],
    ]);
    const cleared: string[] = [];
    const stores = Object.fromEntries(
      (["chat", "code", "hive"] as const).map((mode) => [
        mode,
        {
          session: {
            getState: () => ({
              sessionId: active.get(mode) ?? null,
              clearSession: () => {
                cleared.push(mode);
                active.set(mode, "");
              },
            }),
          },
        },
      ]),
    ) as unknown as SessionDeletionProjectionModeStores;
    return { stores, active, cleared };
  };
  const captured = graph("deleted");
  const replacement = graph("new-selection");

  assert(
    clearDeletedSessionFromModeStoreGraphs(
      captured.stores,
      replacement.stores,
      "deleted",
    ) &&
      captured.cleared.join(",") === "chat,code,hive" &&
      replacement.cleared.join(",") === "chat,hive" &&
      replacement.active.get("code") === "new-selection",
    "a same-lifecycle replacement graph must detach the deleted ID without clearing its newer selection",
  );
});
