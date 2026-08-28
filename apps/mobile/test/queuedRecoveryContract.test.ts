import { isExactQueuedRecoveryActionTarget } from "../components/hive/queuedRecoveryActionFence.ts";

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

Deno.test("uncertain queued input exposes explicit Retry and Discard controls", async () => {
  const surface = await source(
    "../components/chat-screen/ActiveConversationSurface.tsx",
  );
  assert(
    surface.includes("queuedRecoveryBlocked: state.queuedRecoveryBlocked") &&
      surface.includes("state.retryQueuedRecovery") &&
      surface.includes("state.discardQueuedRecovery"),
    "the active transcript owner must subscribe to the bounded recovery state",
  );
  assert(
    surface.includes('accessibilityLabel="Retry uncertain queued message"') &&
      surface.includes(
        'accessibilityLabel="Discard uncertain queued message"',
      ),
    "an uncertain non-idempotent request needs accessible explicit choices",
  );
});

Deno.test("desktop Hive queued recovery is accessible and exact-session bound", async () => {
  assert(
    isExactQueuedRecoveryActionTarget("session-a", "session-a", true),
    "the current blocked session may resolve its own queued recovery",
  );
  assert(
    !isExactQueuedRecoveryActionTarget("session-a", "session-b", true),
    "a stale A control must fail closed after B becomes current",
  );
  assert(
    !isExactQueuedRecoveryActionTarget("session-a", "session-a", false),
    "a stale control must not run after the recovery is already resolved",
  );

  const [surface, sessionView] = await Promise.all([
    source("../components/hive/HiveThreadSurface.tsx"),
    source("../components/hive/hooks/useHiveSessionView.ts"),
  ]);
  assert(
    sessionView.includes(
      "queuedRecoveryBlocked: state.queuedRecoveryBlocked",
    ) &&
      sessionView.includes("isExactQueuedRecoveryActionTarget(") &&
      sessionView.includes("await state.retryQueuedRecovery()") &&
      sessionView.includes(
        "await state.discardQueuedRecovery(expectedSessionId)",
      ),
    "the desktop view must select queue recovery state and recheck the rendered session before either action",
  );
  assert(
    surface.includes('accessibilityRole="alert"') &&
      surface.includes('accessibilityLiveRegion="polite"') &&
      surface.includes(
        'accessibilityLabel="Retry uncertain queued message"',
      ) &&
      surface.includes(
        'accessibilityLabel="Discard uncertain queued message"',
      ) &&
      surface.includes("accessibilityState={{"),
    "desktop Hive must expose announced, labelled, disabled-aware recovery controls",
  );
  assert(
    surface.includes("sessionView.retryQueuedRecovery(targetSessionId)") &&
      surface.includes("sessionView.discardQueuedRecovery(targetSessionId)") &&
      surface.includes("queuedRecoveryBlocked ||"),
    "recovery controls and both desktop composers must fail closed on the exact blocked session",
  );
});

Deno.test("session deletion scrubs its private recovery record before transport", async () => {
  const actions = await source(
    "../components/chat-screen/useSessionActions.ts",
  );
  const singleDelete = actions.slice(
    actions.indexOf("const handleDeleteSession"),
    actions.indexOf("const handleSetSessionPinned"),
  );
  const serverDelete = singleDelete.indexOf("deleteSession(");
  const recoveryAdmission = singleDelete.indexOf(
    "await beginAllModeSessionDeletionAdmission(",
  );
  assert(
    recoveryAdmission >= 0 && serverDelete > recoveryAdmission &&
      singleDelete.indexOf("admission.commit()") > serverDelete &&
      singleDelete.indexOf("await admission.rollback()", serverDelete) >
        serverDelete,
    "recovery admission must be held through session deletion transport and settled by its outcome",
  );

  const projectDelete = actions.slice(
    actions.indexOf("const handleDeleteProjectSessions"),
    actions.indexOf("const handleInteractiveToolResult"),
  );
  assert(
    projectDelete.indexOf("deleteSession(id)") >= 0 &&
      projectDelete.indexOf(
          "beginAllModeSessionDeletionAdmission(modeStores, id)",
        ) < projectDelete.indexOf("deleteSession(id)"),
    "batch deletion must acquire every recovery admission before transport",
  );
});

Deno.test("large draft recovery uses platform durable storage", async () => {
  const [nativeStorage, webStorage] = await Promise.all([
    source("../platform/mitsuro-storage.native.ts"),
    source("../platform/mitsuro-storage.web.ts"),
  ]);
  assert(
    nativeStorage.includes("AsyncStorage.getItem") &&
      nativeStorage.includes("AsyncStorage.setItem") &&
      nativeStorage.includes("AsyncStorage.removeItem"),
    "native recovery must use bounded AsyncStorage rather than credential storage",
  );
  assert(
    webStorage.includes("getDurableSync") &&
      webStorage.includes("localStorage.setItem"),
    "web recovery needs a synchronous local mirror plus durable writes",
  );
});
