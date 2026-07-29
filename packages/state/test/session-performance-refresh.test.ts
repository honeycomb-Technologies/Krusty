import { createSessionStore } from "../src/session/store.ts";
import {
  processStoredMessages,
  processStoredMessagesCooperatively,
} from "../src/session/messages.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`);
  }
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
  let directory: string | null = null;
  let sessionId: string | null = null;
  let mode: "neutral" | "selected" | "created" = "neutral";
  let targetBranch: string | null = null;
  return {
    getState: () => ({
      directory,
      mode,
      sessionId,
      targetBranch,
      initFromSession: (
        nextSessionId: string,
        nextDirectory: string | null,
        nextMode: "neutral" | "selected" | "created" = "neutral",
        nextTargetBranch: string | null = null,
      ) => {
        sessionId = nextSessionId;
        directory = nextDirectory;
        mode = nextMode;
        targetBranch = nextTargetBranch;
      },
      setSession: (nextSessionId: string | null) => {
        sessionId = nextSessionId;
      },
      clear: () => {
        sessionId = null;
        directory = null;
        mode = "neutral";
        targetBranch = null;
      },
    }),
  };
}

function createSessionsStore(sessions: Array<Record<string, unknown>> = []) {
  return {
    getState: () => ({
      sessions,
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

Deno.test("processStoredMessages reuses stable IDs from previous messages", () => {
  const previous = processStoredMessages([
    { role: "user", content: [{ type: "text", text: "hello" }] },
    { role: "assistant", content: [{ type: "text", text: "world" }] },
  ]);
  const next = processStoredMessages(
    [
      { role: "user", content: [{ type: "text", text: "hello" }] },
      { role: "assistant", content: [{ type: "text", text: "world" }] },
    ],
    previous,
  );
  assertEquals(next[0]?.id, previous[0]?.id, "user id should remain stable");
  assertEquals(next[1]?.id, previous[1]?.id, "assistant id should remain stable");
});

Deno.test("cooperative stored-message parsing is byte-equivalent and time-sliced", async () => {
  const rawMessages = [
    {
      role: "user",
      content: [
        { type: "text", text: "hello" },
        { type: "text", text: " world" },
        {
          type: "image",
          source: {
            type: "base64",
            media_type: "image/png",
            data: "aGVsbG8=",
          },
        },
      ],
    },
    {
      role: "assistant",
      content: [
        { type: "thinking", thinking: "checking" },
        { type: "tool_use", id: "tool-1", name: "Bash", input: { command: "pwd" } },
        { type: "text", text: "done" },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "tool_result",
          tool_use_id: "tool-1",
          output: "/work",
          is_error: false,
        },
      ],
    },
  ];
  const previous = processStoredMessages(rawMessages);
  const expected = processStoredMessages(rawMessages, previous);
  let clock = 0;
  let hostYields = 0;

  const actual = await processStoredMessagesCooperatively(
    rawMessages,
    previous,
    {
      timeSliceMs: 4,
      now: () => {
        clock += 2;
        return clock;
      },
      yieldToHost: async () => {
        hostYields += 1;
      },
    },
  );

  assert(!actual.cancelled, "the complete transform should not be cancelled");
  assert(actual.yieldCount > 0, "content-block work should yield between slices");
  assertEquals(actual.yieldCount, hostYields, "yield telemetry should match host turns");
  assert(
    actual.maxSliceDurationMs <= 4,
    `synthetic slices should honor the budget, got ${actual.maxSliceDurationMs}`,
  );
  assertEquals(
    JSON.stringify(actual.messages),
    JSON.stringify(expected),
    "cooperative output must exactly match the synchronous compatibility path",
  );
});

Deno.test("cooperative stored-message parsing discards partial work after invalidation", async () => {
  const rawMessages = Array.from({ length: 20 }, (_, index) => ({
    role: index % 2 === 0 ? "user" : "assistant",
    content: [{ type: "text", text: `message-${index}` }],
  }));
  let clock = 0;
  let active = true;

  const result = await processStoredMessagesCooperatively(rawMessages, [], {
    timeSliceMs: 4,
    now: () => {
      clock += 5;
      return clock;
    },
    yieldToHost: async () => {
      active = false;
    },
    shouldContinue: () => active,
  });

  assert(result.cancelled, "a superseded selection must cancel at the first host yield");
  assertEquals(result.yieldCount, 1, "cancellation should stop after one bounded slice");
  assertEquals(result.messages.length, 0, "partial transcripts must never be publishable");
});

Deno.test("warm reload preserves message IDs across full session hydrate", async () => {
  const client = {
    getSession: async () => ({
      session: {
        id: "session-1",
        title: "Perf",
        token_count: 1,
        working_dir: "/work",
        project_dir: "/work",
        workspace_mode: "selected",
        session_type: "chat",
        mode: "build",
        permission_mode: "autonomous",
        model: "gpt",
      },
      messages: [
        { role: "user", content: [{ type: "text", text: "hello" }] },
        { role: "assistant", content: [{ type: "text", text: "cached answer" }] },
      ],
    }),
    getSessionState: async () => ({
      agent_state: "idle",
      mode: "build",
      permission_mode: "autonomous",
      last_event_sequence: 1,
      workflow: null,
    }),
    heartbeatSessionPresence: async () => ({}),
    removeSessionPresence: async () => ({}),
    updateSession: async () => ({}),
    setCurrentModel: async () => ({}),
  };

  const store = createSessionStore(
    client as never,
    createStorage(),
    createWorkspace() as never,
    createSessionsStore([
      { id: "session-1", session_type: "chat", title: "Perf" },
    ]) as never,
    createPlanStore() as never,
  );

  await store.getState().loadSession("session-1");
  const firstIds = store.getState().messages.map((message) => message.id);
  assert(firstIds.length === 2, "expected two messages");

  await store.getState().loadSession("session-1", true);
  const secondIds = store.getState().messages.map((message) => message.id);
  assertEquals(
    secondIds.join("|"),
    firstIds.join("|"),
    "full hydrate should preserve stable message identities",
  );
});
