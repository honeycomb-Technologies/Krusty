import {
  canAdoptWorkerSessionBinding,
  isCurrentWorkerSessionLookup,
} from "../components/hive/hooks/workerSessionBindingFence.ts";
import {
  assertCurrentHiveWorkerSendBinding,
  assertHiveWorkerSendAvailable,
} from "../components/hive/workerSessionSendFence.ts";
import { mergeRejectedWorkerDraft } from "../components/hive/workerRejectedDraft.ts";

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

Deno.test("Worker DMs use the durable Worker binding instead of per-message controls", async () => {
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const directChat = await source(
    "../components/hive/HiveWorkerDirectChat.tsx",
  );

  assert(
    thread.includes("<HiveWorkerDirectChatHeader") &&
      thread.includes("<HiveWorkerComposer") &&
      thread.includes("activeWorker && sessionView.sessionId"),
    "an exact Worker DM must select its dedicated header and composer",
  );
  assert(
    directChat.includes("worker.model_key") &&
      directChat.includes("worker.permission_mode") &&
      directChat.includes(
        "Model and permissions are pinned in Worker settings.",
      ),
    "the header must project the persisted Worker model and permission contract",
  );
  assert(
    directChat.includes("workerKey.provider === modelKey.provider") &&
      directChat.includes("workerKey.model_id === modelKey.model_id") &&
      directChat.includes("workerKey.api_format === modelKey.api_format") &&
      directChat.includes("workerKey.auth_scope ?? null"),
    "a display model may be adopted only from the exact Worker model key",
  );
});

Deno.test("Worker composer is text-only and cannot imply unsupported turn overrides", async () => {
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const directChat = await source(
    "../components/hive/HiveWorkerDirectChat.tsx",
  );

  assert(
    directChat.includes(
      "onSend: (sessionId: string, content: string) => Promise<void>",
    ) &&
      directChat.includes("void onSend(sessionId, content)") &&
      directChat.includes(
        "Text uses this Worker’s pinned DM configuration.",
      ),
    "the Worker composer must submit only canonical text to the pinned DM",
  );
  for (
    const unsupported of [
      "onThinkingChange",
      "onPermissionModeToggle",
      "onFastModeToggle",
      "onModeToggle",
      "onModelSelect",
      "ImagePicker",
      "DocumentPicker",
      "attachments",
    ]
  ) {
    assert(
      !directChat.includes(unsupported),
      `Worker composer must not expose ${unsupported}`,
    );
  }
  assert(
    directChat.includes(
      "const draftKey = `worker:${worker.id}:dm:${sessionId}`",
    ) &&
      thread.includes("key={`${activeWorker.id}:${sessionView.sessionId}`}"),
    "Worker drafts must remain isolated and remount across exact Worker/DM changes",
  );
});

Deno.test("mobile Worker DMs swap only lower controls and never expose generic overrides", async () => {
  const [surface, mobileControls] = await Promise.all([
    source("../components/chat-screen/ActiveConversationSurface.tsx"),
    source("../components/hive/HiveMobileThreadControls.tsx"),
  ]);
  const workerStart = mobileControls.indexOf("<HiveWorkerComposer");
  const primaryStart = mobileControls.indexOf("? primaryComposer", workerStart);
  assert(
    surface.includes("renderThreadControls?.(activity)") &&
      mobileControls.includes("memo(") &&
      !mobileControls.includes("useSessionStore") &&
      !mobileControls.includes("<ChatTranscript"),
    "mobile lower controls must reuse the sole transcript owner and avoid token-driven subscriptions",
  );
  assert(
    workerStart >= 0 && primaryStart > workerStart &&
      mobileControls.slice(workerStart, primaryStart).includes(
        "<HiveWorkerComposer",
      ) &&
      !mobileControls.slice(workerStart, primaryStart).includes("<ChatBar"),
    "an exact mobile Worker binding must select only the pinned text composer",
  );
  assert(
    mobileControls.includes('workerBinding.kind === "primary_hive"') &&
      mobileControls.includes("workerBinding.isResolving") &&
      mobileControls.includes("? primaryComposer"),
    "generic controls must wait for a proven primary-Hive binding",
  );
});

Deno.test("Worker composer visibly enforces the server 64 KiB UTF-8 contract", async () => {
  const directChat = await source(
    "../components/hive/HiveWorkerDirectChat.tsx",
  );
  assert(
    directChat.includes("MAX_WORKER_MESSAGE_BYTES = 64 * 1024") &&
      directChat.includes("workerMessageUtf8Bytes(value.trim())") &&
      directChat.includes("contentBytes > MAX_WORKER_MESSAGE_BYTES"),
    "send eligibility must use the server's UTF-8 byte limit",
  );
  assert(
    directChat.includes("maxLength={MAX_WORKER_MESSAGE_BYTES}") &&
      directChat.includes(
        "Message is ${messageBytes.toLocaleString()} bytes",
      ) &&
      directChat.includes('accessibilityLiveRegion="polite"'),
    "the bounded input and over-limit reason must remain visible and accessible",
  );
});

Deno.test("non-Worker Hive thread keeps the existing generic ChatBar", async () => {
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const binding = await source(
    "../components/hive/hooks/useActiveHiveWorkerBinding.ts",
  );
  const compactThread = thread.replace(/\s+/g, " ");
  const workerStart = thread.indexOf("<HiveWorkerComposer");
  const genericStart = thread.indexOf("<ChatBar", workerStart);
  assert(
    workerStart >= 0 && genericStart > workerStart,
    "the generic ChatBar must remain the non-Worker fallback",
  );
  const workerBranch = thread.slice(workerStart, genericStart);
  assert(
    workerBranch.includes("onSend={chat.onWorkerSend}") &&
      !workerBranch.includes("thinkingLevel=") &&
      !workerBranch.includes("permissionMode=") &&
      !workerBranch.includes("fastModeEnabled=") &&
      !workerBranch.includes("onModeToggle=") &&
      !workerBranch.includes("onModelSelect="),
    "Worker messages must not forward generic per-turn override controls",
  );
  const genericBranch = thread.slice(genericStart);
  assert(
    genericBranch.includes("thinkingLevel={chat.thinkingLevel}") &&
      genericBranch.includes("permissionMode={chat.permissionMode}") &&
      genericBranch.includes("fastModeEnabled={chat.fastModeEnabled}") &&
      genericBranch.includes("onModeToggle={chat.onModeToggle}") &&
      genericBranch.includes("onModelSelect={chat.onModelSelect}"),
    "the preexisting generic Hive composer contract must remain intact",
  );
  assert(
    compactThread.includes(
      'const showPrimaryHiveComposer = workerBinding.kind === "primary_hive" || workerBinding.kind === "none"',
    ) && compactThread.includes(": workerBinding.isResolving ? null") &&
      compactThread.includes(": showPrimaryHiveComposer ? ( <ChatBar"),
    "generic controls must not flash while the Worker/session binding is unresolved",
  );
  assert(
    binding.includes(".loadWorkerBySession(sessionId, {") &&
      binding.includes("signal: controller.signal") &&
      binding.includes("response.worker.dm_session_id !== sessionId") &&
      binding.includes('kind: "primary_hive"') &&
      binding.includes('kind: "worker_dm"'),
    "the direct surface must distinguish primary Hive from an exact Worker DM",
  );
});

Deno.test("Worker send aborts after deferred readiness when the active DM changes", async () => {
  let activeMode = "hive";
  let sessionId: string | null = "dm-a";
  let releaseReadiness!: () => void;
  const readiness = new Promise<void>((resolve) => {
    releaseReadiness = resolve;
  });
  let sendCalls = 0;

  const attempt = (async () => {
    const assertCurrent = () =>
      assertCurrentHiveWorkerSendBinding("dm-a", {
        activeMode,
        sessionId,
      });
    assertCurrent();
    await readiness;
    assertCurrent();
    sendCalls += 1;
  })();

  sessionId = "dm-b";
  releaseReadiness();
  let rejected = false;
  try {
    await attempt;
  } catch {
    rejected = true;
  }
  assert(rejected, "a deferred Worker A send must reject after switching to B");
  assert(sendCalls === 0, "a stale Worker A send must never reach Worker B");
});

Deno.test("Worker send rejects while disconnected so its draft can be restored", () => {
  let rejected = false;
  try {
    assertHiveWorkerSendAvailable(false);
  } catch (error) {
    rejected = error instanceof Error && error.message.includes("unavailable");
  }
  assert(
    rejected,
    "an unavailable Worker send must reject to restore its draft",
  );
  assertHiveWorkerSendAvailable(true);
});

Deno.test("a rejected Worker send is restored ahead of newly typed text", () => {
  assert(
    mergeRejectedWorkerDraft("unsent A", "new draft B") ===
      "unsent A\n\nnew draft B",
    "a definite A failure must not erase either the rejected text or newer draft",
  );
  assert(
    mergeRejectedWorkerDraft("unsent A", "") === "unsent A",
    "an otherwise empty composer restores the rejected draft exactly",
  );
});

Deno.test("Worker send integration skips generic model readiness and rethrows exact failures", async () => {
  const [index, actions, directChat, thread, transcript] = await Promise.all([
    source("../app/(tabs)/index.tsx"),
    source("../components/chat-screen/useSessionActions.ts"),
    source("../components/hive/HiveWorkerDirectChat.tsx"),
    source("../components/hive/HiveThreadSurface.tsx"),
    source("../components/chat/ChatTranscript.tsx"),
  ]);
  assert(
    index.includes("assertCurrentHiveWorkerSendBinding(expectedSessionId") &&
      index.includes(
        "assertHiveWorkerSendAvailable(Boolean(client) && isConnected)",
      ) &&
      index.includes("activeMode: activeModeRef.current") &&
      index.includes("skipModelReadiness: true") &&
      index.includes("rethrowErrors: true"),
    "Worker sends must use a live exact-session fence and bypass generic model reconciliation",
  );
  assert(
    index.includes("const handleHiveWorkerStop = useCallback(") &&
      index.includes("onWorkerStop: handleHiveWorkerStop") &&
      directChat.includes("onStop(sessionId)") &&
      thread.includes("onStop={chat.onWorkerStop}"),
    "a delayed Worker stop must remain bound to its exact DM instead of the current Hive session",
  );
  assert(
    actions.includes("targetFence?.assertCurrent();") &&
      actions.includes("if (!targetFence?.skipModelReadiness)") &&
      actions.includes("targetFence.assertCurrent();") &&
      actions.includes("if (targetFence?.rethrowErrors) throw err;"),
    "the shared send path must recheck after awaits, avoid stale error projection, and restore the Worker draft on failure",
  );
  assert(
    actions.includes("currentSessionId !== targetSessionId") &&
      actions.includes(
        "sessionStore.getState().sessionId !== targetSessionId",
      ) &&
      transcript.includes(
        "onSubmitToolResult(sessionId, toolCallId, result)",
      ) &&
      transcript.includes("onPlanConfirm(sessionId, toolCallId, choice)"),
    "persisted interactive transcript actions must remain bound to the session that rendered them",
  );
  assert(
    index.includes(
      "sessionStore.getState().sessionId === targetSessionId",
    ) &&
      index.includes(
        "await sessionStore.getState().loadSession(targetSessionId, true)",
      ),
    "a failed approval from Worker A must not reload A after navigation adopts Worker B",
  );
});

Deno.test("late Worker session lookups cannot cross-bind after a chat switch", () => {
  assert(
    isCurrentWorkerSessionLookup(4, "dm-b", 4, "dm-b"),
    "the current request should remain eligible",
  );
  assert(
    !isCurrentWorkerSessionLookup(3, "dm-a", 4, "dm-b"),
    "a Worker A lookup must become stale after switching to Worker B",
  );
  assert(
    canAdoptWorkerSessionBinding(4, "dm-b", 4, "dm-b", "dm-b"),
    "an exact current response should be adoptable",
  );
  assert(
    !canAdoptWorkerSessionBinding(4, "dm-b", 4, "dm-b", "dm-a"),
    "a mismatched response session must fail closed",
  );
});
