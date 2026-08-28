import {
  isCurrentSessionNavigationIntent,
  isCurrentSessionSendIntent,
} from "../components/navigation/sessionNavigationIntentFence.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function source(path: string): Promise<string> {
  return Deno.readTextFile(new URL(path, import.meta.url));
}

Deno.test("session navigation intent requires the same generation and exact session phase", () => {
  assert(
    isCurrentSessionNavigationIntent(4, 4, null, null),
    "an unchanged empty Hive origin should remain current",
  );
  assert(
    isCurrentSessionNavigationIntent(4, 4, "hive-main", "hive-main"),
    "the intended adopted Hive session should remain current",
  );
  assert(
    !isCurrentSessionNavigationIntent(4, 5, null, null),
    "a newer navigation generation must supersede an empty Hive dispatch",
  );
  assert(
    !isCurrentSessionNavigationIntent(4, 4, "hive-main", "worker-b"),
    "a newer exact Hive session must supersede a late continuation",
  );
});

Deno.test("a deferred generic send failure stays with its originating selection", async () => {
  const originatingGeneration = 4;
  const originatingSessionId = "session-a";
  const transportSessionId = originatingSessionId;
  let currentGeneration = originatingGeneration;
  let currentSessionId: string | null = originatingSessionId;
  let rejectSend!: (error: Error) => void;
  const send = new Promise<void>((_resolve, reject) => {
    rejectSend = reject;
  });
  let projectedErrorSession: string | null = null;

  const pending = (async () => {
    try {
      await send;
    } catch {
      if (
        isCurrentSessionSendIntent(
          originatingGeneration,
          currentGeneration,
          originatingSessionId,
          transportSessionId,
          currentSessionId,
        )
      ) {
        projectedErrorSession = currentSessionId;
      }
    }
  })();

  currentGeneration += 1;
  currentSessionId = "session-b";
  rejectSend(new Error("late A failure"));
  await pending;

  assert(
    projectedErrorSession === null,
    "A's deferred rejection must not project an error onto B",
  );
  assert(
    isCurrentSessionSendIntent(
      7,
      7,
      "session-a",
      "session-a",
      "session-a",
    ),
    "an unchanged persisted session remains eligible for its own error",
  );
  assert(
    isCurrentSessionSendIntent(
      7,
      7,
      null,
      "created-session",
      "created-session",
    ),
    "a blank shell may precreate and then exactly bind its transport session",
  );
  assert(
    !isCurrentSessionSendIntent(7, 7, null, null, "unrelated-session"),
    "a null-session transport must not treat a later unrelated session as its target",
  );
});

Deno.test("the shared generic send path fences every post-await error write", async () => {
  const actions = await source(
    "../components/chat-screen/useSessionActions.ts",
  );
  const send = actions.slice(
    actions.indexOf("const handleSend"),
    actions.indexOf("const handleModelSelect"),
  );
  const originCapture = send.indexOf(
    "const originatingNavigationIntentGeneration",
  );
  const modelAwait = send.indexOf("await ensureModelReady()");
  const modelFence = send.indexOf(
    "if (!isCurrentOriginatingSelection()) return;",
    modelAwait,
  );
  const sessionAwait = send.indexOf("await ensureSessionForSend()");
  const sessionFence = send.indexOf(
    "if (!isCurrentSendTarget()) return;",
    sessionAwait,
  );
  const transportAwait = send.indexOf(".sendMessage(");
  const catchStart = send.indexOf("} catch (err)", transportAwait);
  const catchFence = send.indexOf(
    "} else if (!isCurrentSendTarget())",
    catchStart,
  );
  const errorWrite = send.indexOf("sessionStore.setState({", catchStart);

  assert(
    originCapture >= 0 && modelAwait > originCapture &&
      modelFence > modelAwait &&
      sessionAwait > modelFence && sessionFence > sessionAwait &&
      transportAwait > sessionFence && catchStart > transportAwait &&
      catchFence > catchStart && errorWrite > catchFence,
    "generic readiness, session resolution, and rejection must all fence the originating selection before transport or UI mutation",
  );
  assert(
    send.includes("targetFence?.assertCurrent();") &&
      send.includes("if (targetFence?.rethrowErrors) throw err;"),
    "the generic fence must preserve the exact Worker rejection/draft contract",
  );
});

Deno.test("empty Hive dispatch and New Hive share the root navigation generation", async () => {
  const [index, actions] = await Promise.all([
    source("../app/(tabs)/index.tsx"),
    source("../components/chat-screen/useSessionActions.ts"),
  ]);

  assert(
    index.includes("const navigationIntentGenerationRef = useRef(0)") &&
      index.includes("navigationIntentGenerationRef.current += 1") &&
      index.includes("navigationIntentGenerationRef,"),
    "the root tab dispatcher and actions hook must share one navigation generation",
  );
  assert(
    actions.includes("const navigationIntentGeneration =") &&
      actions.includes("const ensuredSessionId = await") &&
      actions.includes("modeStores.hive.session.getState().sessionId"),
    "New Hive must reject a late companion continuation after newer navigation",
  );

  const dispatch = index.slice(
    index.indexOf("const handleChatBarSend"),
    index.indexOf("const handleHiveWorkerSend"),
  );
  const ensureModel = dispatch.indexOf(
    "const resolvedModel = await ensureModelReady()",
  );
  const dispatchHive = dispatch.indexOf("await client.dispatchHive");
  const refreshSessions = dispatch.indexOf(
    "await sessionsStore.getState().loadSessions()",
  );
  const adoptSession = dispatch.indexOf(
    "await hiveStore.getState().loadSession",
  );
  const emptyPhaseFence = "if (!isCurrentDispatchPhase(null)) return;";
  const beforeEnsureFence = dispatch.lastIndexOf(emptyPhaseFence, ensureModel);
  const afterEnsureFence = dispatch.indexOf(emptyPhaseFence, ensureModel);
  const afterDispatchFence = dispatch.indexOf(emptyPhaseFence, dispatchHive);
  const afterRefreshFence = dispatch.indexOf(emptyPhaseFence, refreshSessions);
  const adoptedFence = dispatch.indexOf(
    "if (!isCurrentDispatchPhase(response.session_id)) return;",
  );
  const successHaptic = dispatch.indexOf("Haptics.notificationAsync");
  const staleFailureFence = dispatch.indexOf("if (!isCurrentFailure) return;");
  const failureWrite = dispatch.indexOf(
    "hiveStore.setState({",
    staleFailureFence,
  );
  assert(
    ensureModel > 0 &&
      beforeEnsureFence > 0 && beforeEnsureFence < ensureModel &&
      afterEnsureFence > ensureModel &&
      dispatchHive > ensureModel &&
      afterDispatchFence > dispatchHive &&
      refreshSessions > dispatchHive &&
      afterRefreshFence > refreshSessions &&
      adoptSession > refreshSessions &&
      adoptedFence > adoptSession &&
      successHaptic > adoptedFence,
    "dispatch must fence model readiness, dispatch, refresh, adoption, and success haptics in order",
  );
  assert(
    staleFailureFence > adoptedFence && failureWrite > staleFailureFence,
    "a stale dispatch failure must return before writing an error",
  );
});
