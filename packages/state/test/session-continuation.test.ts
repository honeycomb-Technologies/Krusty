import { createSessionStore } from "../src/session/store.ts";

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

function assertDeepEquals(actual: unknown, expected: unknown, message: string) {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(
      `${message}\nexpected: ${expectedJson}\nactual: ${actualJson}`,
    );
  }
}

type IntervalCallback = () => void | Promise<void>;

function installFakeIntervals() {
  const originalSetInterval = globalThis.setInterval;
  const originalClearInterval = globalThis.clearInterval;
  let nextId = 1;
  const polling = new Map<number, IntervalCallback>();
  const other = new Map<number, IntervalCallback>();

  globalThis.setInterval = ((callback: IntervalCallback, delay?: number) => {
    const id = nextId++;
    if (delay === 3000) {
      polling.set(id, callback);
    } else {
      other.set(id, callback);
    }
    return id as unknown as ReturnType<typeof setInterval>;
  }) as typeof setInterval;

  globalThis.clearInterval = ((id?: ReturnType<typeof setInterval>) => {
    const key = Number(id);
    polling.delete(key);
    other.delete(key);
  }) as typeof clearInterval;

  return {
    activePollingCount: () => polling.size,
    async runLatestPollingTick() {
      const callback = Array.from(polling.values()).at(-1);
      assert(callback, "expected an active session-state polling interval");
      await callback();
    },
    restore() {
      globalThis.setInterval = originalSetInterval;
      globalThis.clearInterval = originalClearInterval;
    },
  };
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
      initFromSession: () => {},
      clear: () => {},
    }),
  };
}

function createSessionsStore() {
  let loadCount = 0;
  return {
    getState: () => ({
      loadSessions: () => {
        loadCount += 1;
      },
    }),
    get loadCount() {
      return loadCount;
    },
  };
}

function createPlanStore() {
  let visible = false;
  return {
    getState: () => ({
      setVisible: (nextVisible: boolean) => {
        visible = nextVisible;
      },
      setItems: () => {},
    }),
    get visible() {
      return visible;
    },
  };
}

function sessionState(
  agentState: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    id: "session-1",
    agent_state: agentState,
    started_at: null,
    last_event_at: null,
    mode: "build",
    recovery: null,
    live_partial_assistant: null,
    pending_interactions: [],
    delegated_tools: [],
    recent_delegated_runs: [],
    last_event_sequence: null,
    ...overrides,
  };
}

function sessionResponse() {
  return {
    session: {
      id: "session-1",
      title: "Recovered session",
      token_count: 0,
      working_dir: null,
      project_dir: null,
      workspace_mode: "neutral",
      session_type: "chat",
      parent_session_id: null,
      mode: "build",
      updated_at: "2026-05-08T00:00:00Z",
      model: null,
      target_branch: null,
    },
    messages: [],
  };
}

Deno.test("streamChat network drop keeps polling, recovers snapshot, and stops at actionable pending approval", async () => {
  const timers = installFakeIntervals();
  try {
    const snapshots = [
      sessionState("streaming", {
        live_partial_assistant: {
          text: "still running after reconnect",
          thinking: "",
          tool_calls: [],
        },
        last_event_sequence: 41,
      }),
      sessionState("awaiting_input", {
        live_partial_assistant: {
          text: "",
          thinking: "",
          tool_calls: [
            {
              id: "tool-approval-1",
              name: "Bash",
              arguments: { value: { command: "npm test -- --watch=false" } },
            },
          ],
        },
        pending_interactions: [
          {
            kind: "tool_approval",
            tool_call: {
              id: "tool-approval-1",
              name: "Bash",
              arguments: { value: { command: "npm test -- --watch=false" } },
            },
          },
        ],
        last_event_sequence: 42,
      }),
      sessionState("awaiting_input", {
        live_partial_assistant: {
          text: "",
          thinking: "",
          tool_calls: [
            {
              id: "tool-approval-1",
              name: "Bash",
              arguments: { value: { command: "npm test -- --watch=false" } },
            },
          ],
        },
        pending_interactions: [
          {
            kind: "tool_approval",
            tool_call: {
              id: "tool-approval-1",
              name: "Bash",
              arguments: { value: { command: "npm test -- --watch=false" } },
            },
          },
        ],
        last_event_sequence: 42,
      }),
    ];

    let getSessionCount = 0;
    const client = {
      streamChat: async (
        _request: unknown,
        callbacks: { onTextDelta: (delta: string) => void },
      ) => {
        callbacks.onTextDelta("partial before drop");
        throw new Error("network connection dropped");
      },
      getSessionState: async () => {
        const snapshot = snapshots.shift();
        assert(snapshot, "expected a queued session-state snapshot");
        return snapshot;
      },
      getSession: async () => {
        getSessionCount += 1;
        return sessionResponse();
      },
      heartbeatSessionPresence: async () => ({}),
      removeSessionPresence: async () => ({}),
      updateSession: async () => ({}),
      setCurrentModel: async () => ({}),
    };

    const sessionsStore = createSessionsStore();
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      sessionsStore as never,
      createPlanStore() as never,
    );

    store.getState().initSession("session-1", "Recovered session");
    await store.getState().sendMessage("keep going");

    assertEquals(
      store.getState().isStreaming,
      true,
      "SSE drop should keep the active session in a tracked streaming state while the server snapshot is still streaming",
    );
    assertEquals(
      store.getState().error,
      null,
      "recoverable SSE drop should not leave a stale user-facing error while the server run is still active",
    );
    assertEquals(
      timers.activePollingCount(),
      1,
      "SSE drop should keep or restart exactly one session-state polling interval",
    );
    assertEquals(
      store.getState().lastEventSequence,
      41,
      "the recovery snapshot should update lastEventSequence",
    );

    await timers.runLatestPollingTick();

    assertEquals(
      timers.activePollingCount(),
      0,
      "polling should stop after reaching an actionable awaiting_input snapshot",
    );
    assertEquals(
      store.getState().isStreaming,
      false,
      "awaiting input is actionable and should not keep the transcript in streaming mode",
    );

    const recoveredTool = store
      .getState()
      .messages.flatMap((message) => message.toolCalls ?? [])
      .find((toolCall) => toolCall.id === "tool-approval-1");

    assert(
      recoveredTool,
      "expected pending approval tool call to be restored from the server snapshot",
    );
    assertEquals(
      recoveredTool.status,
      "awaiting_approval",
      "pending approval should render as an actionable approval widget after recovery",
    );
    assertDeepEquals(
      recoveredTool.arguments,
      { command: "npm test -- --watch=false" },
      "pending approval should expose the recovered tool argument preview to the widget",
    );
    assertEquals(
      getSessionCount > 0,
      true,
      "reaching an actionable state should refresh the persisted transcript once",
    );
  } finally {
    timers.restore();
  }
});
