import { createSessionStore } from "../src/session/store.ts";
import type { StreamCallbacks } from "@mitsuro/api";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
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
  const values = new Map<string, string>();
  return {
    get: (key: string) => values.get(key) ?? null,
    set: (key: string, value: string) => values.set(key, value),
    delete: (key: string) => values.delete(key),
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

function streamingState(sessionId: string) {
  return {
    id: sessionId,
    agent_state: "streaming",
    started_at: null,
    last_event_at: null,
    mode: "build",
    permission_mode: "autonomous",
    recovery: null,
    live_partial_assistant: {
      text: "private response from Worker A",
      thinking: "",
      tool_calls: [],
    },
    pending_interactions: [],
    delegated_tools: [],
    recent_delegated_runs: [],
    last_event_sequence: 7,
  };
}

function idleState(sessionId: string) {
  return {
    ...streamingState(sessionId),
    agent_state: "idle",
    live_partial_assistant: null,
  };
}

function sessionResponse(sessionId: string, sessionType = "chat") {
  return {
    session: {
      id: sessionId,
      title: sessionId,
      token_count: 0,
      working_dir: null,
      project_dir: null,
      workspace_mode: "neutral",
      session_type: sessionType,
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

Deno.test(
  "late tool approval completion cannot attach Worker A polling to Worker B",
  async () => {
    const approval = deferred<void>();
    const client = {
      submitToolApproval: () => approval.promise,
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );

    store.getState().initSession("worker-a", "Worker A", undefined, "hive");
    const pending = store.getState().submitToolApproval("tool-a", true);
    store.getState().initSession("worker-b", "Worker B", undefined, "hive");

    approval.resolve();
    await pending;

    assertEquals(store.getState().sessionId, "worker-b", "B stays selected");
    assertEquals(
      [store.getState().isStreaming, store.getState().isLoading],
      [false, false],
      "A's approval completion must not mark B as running",
    );
    assertEquals(
      store.getState().messages,
      [],
      "A's approval completion must not change B's transcript",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "late stream recovery snapshot from Worker A cannot overwrite Worker B",
  async () => {
    const recovery = deferred<ReturnType<typeof streamingState>>();
    const recoveryStarted = deferred<void>();
    const client = {
      streamChat: async () => {
        throw new Error("stream disconnected");
      },
      getSessionState: (sessionId: string) => {
        assertEquals(sessionId, "worker-a", "recovery must target Worker A");
        recoveryStarted.resolve();
        return recovery.promise;
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );

    store.getState().initSession("worker-a", "Worker A", undefined, "chat");
    const pending = store.getState().sendMessage("private A input");
    await recoveryStarted.promise;
    store.getState().initSession("worker-b", "Worker B", undefined, "chat");

    recovery.resolve(streamingState("worker-a"));
    await pending;

    assertEquals(store.getState().sessionId, "worker-b", "B stays selected");
    assertEquals(
      [store.getState().isStreaming, store.getState().isLoading],
      [false, false],
      "A's recovery must not attach stream state to B",
    );
    assertEquals(
      store.getState().messages,
      [],
      "A's recovered partial response must not enter B's transcript",
    );
    assertEquals(
      store.getState().error,
      null,
      "B must not inherit A's recovery error",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "terminal recovery reload from Worker A cannot surface its stream error on Worker B",
  async () => {
    const reloadSnapshot = deferred<ReturnType<typeof idleState>>();
    const reloadStarted = deferred<void>();
    let stateRequestCount = 0;
    const client = {
      streamChat: async () => {
        throw new Error("private Worker A stream error");
      },
      getSessionState: (sessionId: string) => {
        assertEquals(sessionId, "worker-a", "recovery must target Worker A");
        stateRequestCount += 1;
        if (stateRequestCount === 1) {
          return Promise.resolve(idleState(sessionId));
        }
        reloadStarted.resolve();
        return reloadSnapshot.promise;
      },
      getSession: async (sessionId: string) => sessionResponse(sessionId),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );

    store.getState().initSession("worker-a", "Worker A", undefined, "chat");
    const pending = store.getState().sendMessage("private A input");
    await reloadStarted.promise;
    store.getState().initSession("worker-b", "Worker B", undefined, "chat");

    reloadSnapshot.resolve(idleState("worker-a"));
    await pending;

    assertEquals(store.getState().sessionId, "worker-b", "B stays selected");
    assertEquals(
      store.getState().error,
      null,
      "A's terminal stream error must not surface on B",
    );
    assertEquals(
      [store.getState().isStreaming, store.getState().isLoading],
      [false, false],
      "A's terminal recovery must not change B's run state",
    );
    assertEquals(
      store.getState().messages,
      [],
      "A's terminal recovery must not reload its transcript into B",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "an existing optimistic Worker DM honors its explicit Hive send contract without mutating session type",
  async () => {
    const capturedRequests: Array<Record<string, unknown>> = [];
    let capturedIdempotencyKey: string | undefined;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async (
        request: Record<string, unknown>,
        callbacks: StreamCallbacks,
        _signal: AbortSignal,
        options?: { idempotencyKey?: string },
      ) => {
        capturedRequests.push(request);
        capturedIdempotencyKey = options?.idempotencyKey;
        acceptWorkerInput(callbacks, "worker-a-dm", "optimistic-worker-run");
        callbacks.onFinish("worker-a-dm", "completed");
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );

    store.getState().initSession("worker-a-dm", "Worker A");
    store.setState({
      model: "grok-4.6",
      modelKey: {
        provider: "grok",
        model_id: "grok-4.6",
        api_format: "open_ai_responses",
      },
      permissionMode: "autonomous",
      mode: "build",
    });
    await store.getState().sendMessage(
      "send before generic session hydration finishes",
      [],
      {
        sessionType: "hive",
        hiveConversationKind: "worker_dm",
      },
    );

    const capturedRequest = capturedRequests[0];
    assertEquals(
      store.getState().sessionType,
      null,
      "the send hint must not mutate the optimistic shell's durable type",
    );
    assertEquals(
      {
        session_id: capturedRequest?.session_id,
        session_type: capturedRequest?.session_type,
        model: capturedRequest?.model,
        model_key: capturedRequest?.model_key,
        permission_mode: capturedRequest?.permission_mode,
        mode: capturedRequest?.mode,
        hiveConversationKind: capturedRequest?.hiveConversationKind,
      },
      {
        session_id: "worker-a-dm",
        session_type: undefined,
        model: undefined,
        model_key: undefined,
        permission_mode: undefined,
        mode: undefined,
        hiveConversationKind: undefined,
      },
      "an existing Worker DM must omit creation metadata, generic overrides, and the client-only ownership hint",
    );
    if (!capturedIdempotencyKey) {
      throw new Error(
        "an optimistic Worker DM must retain its idempotency key",
      );
    }
    store.getState().cleanup();
  },
);

Deno.test(
  "an optimistic Worker steer honors the explicit Hive idempotency contract",
  async () => {
    let steeringIdempotencyKey: string | undefined;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      steerSession: async (
        _request: unknown,
        options?: { idempotencyKey?: string },
      ) => {
        steeringIdempotencyKey = options?.idempotencyKey;
        return {
          pending_id: "pending-a",
          status: "queued",
          staged_input_id: null,
          successor_run_id: null,
        };
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );

    store.getState().initSession("worker-a-dm", "Worker A");
    store.setState({ isStreaming: true });
    await store.getState().sendMessage(
      "steer the optimistic Worker shell",
      [],
      { sessionType: "hive" },
    );

    if (!steeringIdempotencyKey) {
      throw new Error("the Worker steer must carry an idempotency key");
    }
    store.getState().cleanup();
  },
);

Deno.test(
  "late workflow completion rejects without publishing Worker A into Worker B",
  async () => {
    const commandResult = deferred<any>();
    const workflowPublications: unknown[] = [];
    const client = {
      executeWorkflowCommand: () => commandResult.promise,
    };
    const planStore = {
      getState: () => ({
        setVisible: () => {},
        setItems: () => {},
        setWorkflow: (workflow: unknown) => workflowPublications.push(workflow),
      }),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      planStore as never,
    );

    store.getState().initSession("worker-a", "Worker A", undefined, "hive");
    const pending = store.getState().executeWorkflowCommand({
      action: "approve_plan",
      goal_id: "goal-a",
      expected_revision: 1,
    } as never);
    store.getState().initSession("worker-b", "Worker B", undefined, "hive");
    commandResult.resolve({
      snapshot: {
        goal: { id: "goal-a", status: "active" },
      },
    });

    let rejected = false;
    try {
      await pending;
    } catch {
      rejected = true;
    }
    assertEquals(
      rejected,
      true,
      "the stale command must terminate caller chains",
    );
    assertEquals(
      workflowPublications.filter((workflow) => workflow !== null),
      [],
      "A's workflow must not enter B's plan store",
    );
    assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
    assertEquals(store.getState().mode, "build", "A cannot change B's mode");
    store.getState().cleanup();
  },
);

Deno.test(
  "late Hive companion ensure cannot replace a newly selected Worker",
  async () => {
    const ensured = deferred<{ session_id: string }>();
    let sessionLoads = 0;
    const client = {
      ensureHiveMain: () => ensured.promise,
      getSession: async (sessionId: string) => {
        sessionLoads += 1;
        return sessionResponse(sessionId);
      },
      getSessionState: async (sessionId: string) => idleState(sessionId),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );

    const pending = store.getState().ensureHiveMainSession();
    store.getState().initSession("worker-b", "Worker B", undefined, "hive");
    ensured.resolve({ session_id: "primary-hive" });

    assertEquals(
      await pending,
      null,
      "stale ensure must report no adopted session",
    );
    assertEquals(store.getState().sessionId, "worker-b", "B remains selected");
    assertEquals(sessionLoads, 0, "the stale main session is never hydrated");
    store.getState().cleanup();
  },
);

Deno.test(
  "a definite unrecovered Worker stream failure rejects for draft restoration",
  async () => {
    let stateRequestCount = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async () => {
        throw new Error("provider rejected Worker turn");
      },
      getSessionState: async (sessionId: string) => {
        stateRequestCount += 1;
        return idleState(sessionId);
      },
      getSession: async (sessionId: string) => sessionResponse(sessionId),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");

    let rejection = "";
    try {
      await store.getState().sendMessage(
        "restore this exact draft",
        [],
        { sessionType: "hive" },
      );
    } catch (error) {
      rejection = error instanceof Error ? error.message : String(error);
    }

    assertEquals(
      rejection,
      "provider rejected Worker turn",
      "the dedicated composer must observe the unrecovered failure",
    );
    assertEquals(
      stateRequestCount >= 1,
      true,
      "canonical recovery is attempted before rejecting the draft",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "a production-style Worker SSE error that resolves still rejects an unaccepted draft",
  async () => {
    let stateRequestCount = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async (
        _request: unknown,
        callbacks: StreamCallbacks,
      ) => {
        callbacks.onError("provider rejected callback-only Worker turn");
        // The real API client reports SSE failures through callbacks and then
        // resolves its transport promise.
      },
      getSessionState: async (sessionId: string) => {
        stateRequestCount += 1;
        return idleState(sessionId);
      },
      getSession: async (sessionId: string) => sessionResponse(sessionId),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");

    let rejection = "";
    try {
      await store.getState().sendMessage(
        "restore callback-rejected draft",
        [],
        { sessionType: "hive", hiveConversationKind: "worker_dm" },
      );
    } catch (error) {
      rejection = error instanceof Error ? error.message : String(error);
    }

    assertEquals(
      rejection,
      "provider rejected callback-only Worker turn",
      "callback-only provider failure must reject to the exact Worker composer",
    );
    assertEquals(stateRequestCount >= 1, true, "recovery is checked first");
    store.getState().cleanup();
  },
);

Deno.test(
  "a silently resolved Worker A transport after navigation cannot accept its draft into B",
  async () => {
    const transport = deferred<void>();
    const attached = deferred<void>();
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: sessionId },
      }),
      streamChat: async () => {
        attached.resolve();
        await transport.promise;
      },
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
    const pending = store.getState().sendMessage(
      "private A draft",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
    await attached.promise;
    store.getState().initSession("worker-b", "Worker B", undefined, "hive");
    transport.resolve();

    let rejection = "";
    try {
      await pending;
    } catch (error) {
      rejection = error instanceof Error ? error.message : String(error);
    }
    assertEquals(
      rejection.includes("detached before remote acceptance"),
      true,
      "unconfirmed stale Worker input must reject for exact-session draft restoration",
    );
    assertEquals(store.getState().sessionId, "worker-b", "B stays selected");
    assertEquals(store.getState().messages, [], "B transcript stays untouched");
    store.getState().cleanup();
  },
);

Deno.test(
  "terminal stream callbacks ignore provider records after finish",
  async () => {
    const client = {
      streamChat: async (
        request: { session_id?: string },
        callbacks: StreamCallbacks,
      ) => {
        callbacks.onFinish(request.session_id ?? "chat-a", "completed");
        callbacks.onTextDelta("late provider text");
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );
    store.getState().initSession("chat-a", "Chat A", undefined, "chat");
    await store.getState().sendMessage("hello");
    await new Promise((resolve) => setTimeout(resolve, 20));
    assertEquals(
      store.getState().messages.some((message) =>
        message.content.includes("late provider text")
      ),
      false,
      "records after the first terminal event must be ignored",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "a production-style tool-result callback error rejects after idle recovery",
  async () => {
    let stateRequestCount = 0;
    const client = {
      streamToolResult: async (
        _request: unknown,
        callbacks: StreamCallbacks,
      ) => {
        callbacks.onError("tool result was not accepted");
        // The production SSE client reports the terminal record through the
        // callback and then resolves its transport promise.
      },
      getSessionState: async (sessionId: string) => {
        stateRequestCount += 1;
        return idleState(sessionId);
      },
      getSession: async (sessionId: string) => sessionResponse(sessionId),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );
    store.getState().initSession("chat-a", "Chat A", undefined, "chat");
    store.setState({
      messages: [{
        id: "assistant-tool",
        role: "assistant",
        content: "",
        toolCalls: [{
          id: "tool-a",
          name: "AskUserQuestion",
          arguments: {},
          status: "pending",
        }],
      }],
    });

    let rejection = "";
    try {
      await store.getState().submitToolResult("tool-a", "answer");
    } catch (error) {
      rejection = error instanceof Error ? error.message : String(error);
    }

    assertEquals(
      rejection,
      "tool result was not accepted",
      "the exact-session wrapper must be able to reload an actionable tool after failure",
    );
    assertEquals(stateRequestCount >= 1, true, "recovery is checked first");
    store.getState().cleanup();
  },
);

Deno.test(
  "pre-boundary Worker Stop awaits cancellation failure and restores canonical state",
  async () => {
    const cancelGate = deferred<void>();
    const streamStarted = deferred<void>();
    let cancelCalls = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async (
        _request: unknown,
        _callbacks: unknown,
        signal: AbortSignal,
      ) => {
        streamStarted.resolve();
        await new Promise<void>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("aborted")), {
            once: true,
          });
        });
      },
      cancelSession: async () => {
        cancelCalls += 1;
        await cancelGate.promise;
        return { ok: true };
      },
      getSessionState: async (sessionId: string) => idleState(sessionId),
      getSession: async (sessionId: string) =>
        sessionResponse(sessionId, "hive"),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");
    const send = store.getState().sendMessage(
      "stop before the response boundary",
      [],
      {
        sessionType: "hive",
        hiveConversationKind: "worker_dm",
      },
    );
    await streamStarted.promise;
    store.getState().stopStreaming();
    await Promise.resolve();
    assertEquals(
      cancelCalls,
      1,
      "Stop must await the exact Hive cancel request",
    );

    cancelGate.reject(new Error("daemon refused cancellation"));
    await send;
    for (let index = 0; index < 20 && !store.getState().error; index += 1) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }

    assertEquals(
      store.getState().error?.includes("daemon refused cancellation"),
      true,
      "a failed pre-boundary Stop must remain visible after canonical reload",
    );
    assertEquals(
      store.getState().isStreaming,
      false,
      "the local stream stays detached",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "late Worker Stop status cannot reload Worker A over Worker B",
  async () => {
    const status = deferred<any>();
    const statusStarted = deferred<void>();
    const streamStarted = deferred<void>();
    let sessionLoads = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async (
        _request: unknown,
        callbacks: {
          onWorkerResponsePending(event: unknown): void;
        },
        signal: AbortSignal,
      ) => {
        callbacks.onWorkerResponsePending({
          type: "worker_response_pending",
          worker_id: "worker-a",
          session_id: "worker-a-dm",
          run_id: "run-a",
        });
        streamStarted.resolve();
        await new Promise<void>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("aborted")), {
            once: true,
          });
        });
      },
      cancelSession: async () => ({ ok: true }),
      getHiveSessionStatus: () => {
        statusStarted.resolve();
        return status.promise;
      },
      getSessionState: async (sessionId: string) => idleState(sessionId),
      getSession: async (sessionId: string) => {
        sessionLoads += 1;
        return sessionResponse(sessionId, "hive");
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");
    const send = store.getState().sendMessage("response to stop", [], {
      sessionType: "hive",
    });
    await streamStarted.promise;
    store.getState().stopStreaming();
    await statusStarted.promise;
    const loadsBeforeNavigation = sessionLoads;
    store.getState().initSession("worker-b-dm", "Worker B", undefined, "hive");

    status.resolve({
      session_id: "worker-a-dm",
      session_type: "hive",
      title: "Worker A",
      tasks: [],
      agent_state: "idle",
      runtime: {
        session_id: "worker-a-dm",
        status: "cancelled",
        current_run_id: "run-a",
      },
      cadence: {},
    });
    await send;
    for (let index = 0; index < 4; index += 1) await Promise.resolve();

    assertEquals(
      store.getState().sessionId,
      "worker-b-dm",
      "B remains selected",
    );
    assertEquals(
      sessionLoads,
      loadsBeforeNavigation,
      "the stale status continuation cannot reload A after B is selected",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "a recovered Worker stream uses the exact UI binding for pre-boundary Stop semantics",
  async () => {
    let cancelCalls = 0;
    let sessionLoads = 0;
    const client = {
      cancelSession: async () => {
        cancelCalls += 1;
      },
      getSessionState: async (sessionId: string) => idleState(sessionId),
      getSession: async (sessionId: string) => {
        sessionLoads += 1;
        return sessionResponse(sessionId, "hive");
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");
    store.setState({
      isStreaming: true,
      messages: [{
        id: "recovered-worker-partial",
        role: "assistant",
        content: "untrusted recovered partial",
        kind: "live_partial",
      }],
    });

    store.getState().stopStreaming({
      expectedSessionId: "worker-b-dm",
      hiveConversationKind: "worker_dm",
    });
    assertEquals(
      [store.getState().isStreaming, cancelCalls],
      [true, 0],
      "a stale Worker Stop target must not stop the current session",
    );

    store.getState().stopStreaming({
      expectedSessionId: "worker-a-dm",
      hiveConversationKind: "worker_dm",
    });
    for (let index = 0; index < 20 && sessionLoads === 0; index += 1) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }

    assertEquals(cancelCalls, 1, "the exact Worker session must be cancelled");
    assertEquals(
      sessionLoads >= 1,
      true,
      "an accepted pre-boundary Worker Stop must rehydrate canonical state",
    );
    assertEquals(
      store.getState().messages.some((message) =>
        message.id === "recovered-worker-partial"
      ),
      false,
      "the recovered Worker partial must never be finalized as trusted text",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "an accepted pre-boundary Worker Stop stays fenced until Hive runtime is terminal",
  async () => {
    const status = deferred<any>();
    const statusStarted = deferred<void>();
    const streamStarted = deferred<void>();
    let runtimeTerminal = false;
    let streamCalls = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async (
        request: { session_id?: string },
        callbacks: StreamCallbacks,
        signal: AbortSignal,
      ) => {
        streamCalls += 1;
        if (streamCalls === 1) {
          streamStarted.resolve();
          await new Promise<void>((_resolve, reject) => {
            signal.addEventListener(
              "abort",
              () => reject(new Error("aborted")),
              { once: true },
            );
          });
          return;
        }
        const sessionId = request.session_id ?? "worker-a-dm";
        acceptWorkerInput(callbacks, sessionId, "post-stop-run");
        callbacks.onFinish(sessionId, "completed");
      },
      cancelSession: async () => ({ ok: true }),
      getHiveSessionStatus: async () => {
        statusStarted.resolve();
        return status.promise;
      },
      getSessionState: async (sessionId: string) =>
        runtimeTerminal ? idleState(sessionId) : streamingState(sessionId),
      getSession: async (sessionId: string) =>
        sessionResponse(sessionId, "hive"),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");
    const firstSend = store.getState().sendMessage(
      "stop before boundary",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
    await streamStarted.promise;
    store.getState().stopStreaming({
      expectedSessionId: "worker-a-dm",
      hiveConversationKind: "worker_dm",
    });
    await firstSend;
    await statusStarted.promise;

    let earlyError = "";
    try {
      await store.getState().sendMessage(
        "must wait",
        [],
        { sessionType: "hive", hiveConversationKind: "worker_dm" },
      );
    } catch (error) {
      earlyError = error instanceof Error ? error.message : String(error);
    }
    assertEquals(
      earlyError.includes("still stopping"),
      true,
      "the cancel receipt alone must not unblock a successor Worker turn",
    );
    assertEquals(
      store.getState().messages.some((message) =>
        message.content.includes("private response from Worker A")
      ),
      false,
      "the stopped live partial stays suppressed during settlement",
    );

    runtimeTerminal = true;
    status.resolve({
      session_id: "worker-a-dm",
      session_type: "hive",
      title: "Worker A",
      tasks: [],
      agent_state: "idle",
      runtime: {
        session_id: "worker-a-dm",
        status: "cancelled",
        current_run_id: "run-a",
      },
      cadence: {},
    });
    let successorAccepted = false;
    for (let index = 0; index < 20 && !successorAccepted; index += 1) {
      try {
        await store.getState().sendMessage(
          "after settlement",
          [],
          { sessionType: "hive", hiveConversationKind: "worker_dm" },
        );
        successorAccepted = true;
      } catch (error) {
        if (
          !(error instanceof Error) || !error.message.includes("still stopping")
        ) {
          throw error;
        }
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    }
    assertEquals(
      successorAccepted,
      true,
      "terminal status releases the exact fence",
    );
    assertEquals(
      streamCalls,
      2,
      "only the original and post-settlement turns run",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "a boundary-backed Worker Stop blocks same-session sends until its run settles",
  async () => {
    const status = deferred<any>();
    const statusStarted = deferred<void>();
    const streamStarted = deferred<void>();
    let streamCalls = 0;
    const client = {
      getHiveWorkerBySession: async (sessionId: string) => ({
        kind: "worker_dm",
        session_id: sessionId,
        worker: { id: "worker-a" },
      }),
      streamChat: async (
        request: { session_id?: string },
        callbacks: StreamCallbacks,
        signal: AbortSignal,
      ) => {
        streamCalls += 1;
        if (streamCalls === 1) {
          callbacks.onWorkerResponsePending?.({
            type: "worker_response_pending",
            worker_id: "worker-a",
            session_id: "worker-a-dm",
            run_id: "run-a",
          });
          streamStarted.resolve();
          await new Promise<void>((_resolve, reject) => {
            signal.addEventListener(
              "abort",
              () => reject(new Error("aborted")),
              {
                once: true,
              },
            );
          });
          return;
        }
        const sessionId = request.session_id ?? "worker-a-dm";
        acceptWorkerInput(callbacks, sessionId, "post-boundary-stop-run");
        callbacks.onFinish(sessionId, "completed");
      },
      cancelSession: async () => ({ ok: true }),
      getHiveSessionStatus: async () => {
        statusStarted.resolve();
        return status.promise;
      },
      getSessionState: async (sessionId: string) => idleState(sessionId),
      getSession: async (sessionId: string) =>
        sessionResponse(sessionId, "hive"),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");
    const firstSend = store.getState().sendMessage(
      "stop after boundary",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
    await streamStarted.promise;
    store.getState().stopStreaming({
      expectedSessionId: "worker-a-dm",
      hiveConversationKind: "worker_dm",
    });
    await firstSend;
    await statusStarted.promise;

    let earlyError = "";
    try {
      await store.getState().sendMessage(
        "must still wait",
        [],
        { sessionType: "hive", hiveConversationKind: "worker_dm" },
      );
    } catch (error) {
      earlyError = error instanceof Error ? error.message : String(error);
    }
    assertEquals(
      earlyError.includes("still stopping"),
      true,
      "run settlement owns the fence",
    );

    status.resolve({
      session_id: "worker-a-dm",
      session_type: "hive",
      title: "Worker A",
      tasks: [],
      agent_state: "idle",
      runtime: {
        session_id: "worker-a-dm",
        status: "cancelled",
        current_run_id: "run-a",
      },
      cadence: {},
    });
    let successorAccepted = false;
    for (let index = 0; index < 20 && !successorAccepted; index += 1) {
      try {
        await store.getState().sendMessage(
          "after exact run settlement",
          [],
          { sessionType: "hive", hiveConversationKind: "worker_dm" },
        );
        successorAccepted = true;
      } catch (error) {
        if (
          !(error instanceof Error) || !error.message.includes("still stopping")
        ) {
          throw error;
        }
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    }
    assertEquals(
      successorAccepted,
      true,
      "terminal run status releases the fence",
    );
    assertEquals(
      streamCalls,
      2,
      "no same-session turn started during settlement",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "terminal Worker runtime cannot resurrect a stale session partial or poll",
  async () => {
    const streamStarted = deferred<void>();
    let streamCalls = 0;
    let stateRequests = 0;
    const client = {
      streamChat: async (
        request: { session_id?: string },
        callbacks: StreamCallbacks,
        signal: AbortSignal,
      ) => {
        streamCalls += 1;
        if (streamCalls === 1) {
          streamStarted.resolve();
          await new Promise<void>((_resolve, reject) => {
            signal.addEventListener(
              "abort",
              () => reject(new Error("aborted")),
              {
                once: true,
              },
            );
          });
          return;
        }
        const sessionId = request.session_id ?? "worker-a-dm";
        acceptWorkerInput(callbacks, sessionId, "post-terminal-run");
        callbacks.onFinish(sessionId, "completed");
      },
      cancelSession: async () => ({ ok: true }),
      getHiveSessionStatus: async () => ({
        session_id: "worker-a-dm",
        session_type: "hive",
        title: "Worker A",
        tasks: [],
        // Forced execution-host cancellation can leave this generic projection
        // stale even after durable Hive runtime is terminal.
        agent_state: "streaming",
        runtime: {
          session_id: "worker-a-dm",
          status: "cancelled",
          current_run_id: "run-a",
        },
        cadence: {},
      }),
      getSessionState: async (sessionId: string) => {
        stateRequests += 1;
        return streamingState(sessionId);
      },
      getSession: async (sessionId: string) =>
        sessionResponse(sessionId, "hive"),
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
      "hive",
    );
    let pollingActive = false;
    const startStatePolling = store.getState().startStatePolling;
    const stopStatePolling = store.getState().stopStatePolling;
    store.setState({
      startStatePolling: (sessionId: string) => {
        startStatePolling(sessionId);
        pollingActive = true;
      },
      stopStatePolling: () => {
        stopStatePolling();
        pollingActive = false;
      },
    });
    store.getState().initSession("worker-a-dm", "Worker A", undefined, "hive");

    const firstSend = store.getState().sendMessage(
      "stop before the response boundary",
      [],
      { sessionType: "hive", hiveConversationKind: "worker_dm" },
    );
    await streamStarted.promise;
    store.getState().stopStreaming({
      expectedSessionId: "worker-a-dm",
      hiveConversationKind: "worker_dm",
    });
    await firstSend;

    let successorAccepted = false;
    for (let index = 0; index < 40 && !successorAccepted; index += 1) {
      try {
        await store.getState().sendMessage(
          "after forced-stop settlement",
          [],
          { sessionType: "hive", hiveConversationKind: "worker_dm" },
        );
        successorAccepted = true;
      } catch (error) {
        if (
          !(error instanceof Error) || !error.message.includes("still stopping")
        ) {
          throw error;
        }
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    }

    assertEquals(
      stateRequests >= 2,
      true,
      "settlement performs its final canonical reload",
    );
    assertEquals(
      store.getState().messages.some((message) =>
        message.content.includes("private response from Worker A")
      ),
      false,
      "the stale stopped response remains filtered through that reload",
    );
    assertEquals(
      store.getState().isStreaming,
      false,
      "stale agent state cannot restore streaming",
    );
    assertEquals(
      pollingActive,
      false,
      "settlement tears down polling started by stale state",
    );
    assertEquals(
      successorAccepted,
      true,
      "the exact Stop settlement still releases sends",
    );
    assertEquals(
      streamCalls,
      2,
      "only the stopped turn and its successor execute",
    );
    store.getState().cleanup();
  },
);

Deno.test(
  "a detached stream finalizer cannot clear the newer session's live ownership",
  async () => {
    const activeStreams = new Map<string, { reject(error: unknown): void }>();
    let fullSessionLoads = 0;
    const client = {
      streamChat: (
        request: { session_id?: string },
        _callbacks: unknown,
        signal: AbortSignal,
      ) =>
        new Promise<void>((_resolve, reject) => {
          const sessionId = request.session_id ?? "new";
          activeStreams.set(sessionId, { reject });
          signal.addEventListener(
            "abort",
            () => reject(new Error(`detached ${sessionId}`)),
            { once: true },
          );
        }),
      getSessionState: async (sessionId: string) => streamingState(sessionId),
      getSession: async (sessionId: string) => {
        fullSessionLoads += 1;
        return sessionResponse(sessionId);
      },
    };
    const store = createSessionStore(
      client as never,
      createStorage(),
      createWorkspace() as never,
      createSessionsStore() as never,
      createPlanStore() as never,
    );

    store.getState().initSession("session-a", "A", undefined, "chat");
    const sendA = store.getState().sendMessage("A request");
    await Promise.resolve();
    store.getState().initSession("session-b", "B", undefined, "chat");
    const sendB = store.getState().sendMessage("B request");
    await Promise.resolve();
    await Promise.resolve();

    await store.getState().loadSession("session-b", true);
    assertEquals(
      fullSessionLoads,
      0,
      "B's live SSE ownership must keep refresh metadata-only after A finalizes",
    );

    store.getState().cleanup();
    activeStreams.get("session-b")?.reject(new Error("cleanup"));
    await Promise.all([sendA, sendB]);
  },
);
