declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

import { canAdoptHiveWorkerIntroductionAction } from "../components/hive/hooks/workerIntroductionBinding.ts";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function source(path: string): Promise<string> {
  return Deno.readTextFile(new URL(path, import.meta.url));
}

Deno.test("Hive Worker Introduction controls preserve replay keys across lost responses", async () => {
  const hook = await source("../components/hive/hooks/useHiveWorkers.ts");
  assert(
    hook.includes("const attemptId = `${id}:retry`;") &&
      hook.includes("const attemptId = `${id}:skip`;"),
    "retry and skip must keep independent per-Worker attempts",
  );
  assert(
    hook.includes("introductionActionAttemptsRef.current.get(attemptId)") &&
      hook.includes(
        "introductionActionAttemptsRef.current.set(attemptId, attempt)",
      ),
    "an uncertain HTTP result must reuse its original idempotency key",
  );
  assert(
    hook.includes("introductionActionAttemptsRef.current.delete(attemptId)"),
    "a completed action must release its replay key for a future explicit action",
  );
});

Deno.test("review decisions fingerprint typed bodies and keep replay-stable keys", async () => {
  const hook = await source("../components/hive/hooks/useHiveWorkers.ts");
  assert(
    hook.includes("const attemptId = `${id}:confirm`;") &&
      hook.includes("const attemptId = `${id}:keep-talking`;"),
    "confirm and keep-talking must retain independent attempts",
  );
  assert(
    hook.includes("const fingerprint = JSON.stringify(request);") &&
      hook.includes("api.confirmHiveWorkerIntroduction") &&
      hook.includes("api.keepTalkingHiveWorkerIntroduction"),
    "proposal revision, selections, and action must bind the replay key",
  );
});

Deno.test("active DM Introduction polling is core-authored and subscription-free", async () => {
  const hook = await source(
    "../components/hive/hooks/useActiveHiveWorkerIntroduction.ts",
  );
  const binding = await source(
    "../components/hive/hooks/workerIntroductionBinding.ts",
  );
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const screen = await source("../components/hive/HiveScreen.tsx");
  assert(
    hook.includes("worker: activeWorker") &&
      !hook.includes("workers.workers.find") &&
      hook.includes("next.introduction?.review_projection.should_poll"),
    "Introduction must consume the already-proven Worker binding and poll only from core projection",
  );
  assert(
    hook.includes("new AbortController()") &&
      hook.includes("generation !== generationRef.current") &&
      hook.includes("clearTimeout(timerRef.current)"),
    "detail requests and timers must be aborted and generation fenced",
  );
  assert(
    hook.includes("bindingRef.current") &&
      hook.includes("canAdoptHiveWorkerIntroductionAction") &&
      binding.includes("current.generation === expected.generation") &&
      binding.includes("result.workerId === expected.workerId"),
    "a late Worker A action response must never be adopted under Worker B",
  );
  assert(
    hook.includes("localError?.workerId === activeWorker.id") &&
      hook.includes("localError.sessionId === sessionId") &&
      hook.includes("throwOnError: true") &&
      !hook.includes("error: workers.error"),
    "detail and action errors must remain scoped to the exact Worker/DM instead of leaking shared roster errors",
  );
  assert(
    !hook.includes("useSessionStore") &&
      thread.includes("transcriptTailKey:") &&
      thread.includes("isStreaming: sessionView.isStreaming"),
    "the detail hook must consume the existing transcript projection, not subscribe again",
  );
  assert(
    screen.includes('navigation.topLevel === "hive"') &&
      screen.includes("workers={workers}"),
    "the direct Hive surface must make its one roster fetch available downstream",
  );
});

Deno.test("late Worker A action response is discarded after switching to Worker B", () => {
  const expected = {
    generation: 4,
    workerId: "worker-a",
    sessionId: "dm-a",
  };
  assert(
    canAdoptHiveWorkerIntroductionAction(expected, expected, {
      workerId: "worker-a",
      sessionId: "dm-a",
    }),
    "the exact initiating binding should be adoptable",
  );
  assert(
    !canAdoptHiveWorkerIntroductionAction(
      { generation: 5, workerId: "worker-b", sessionId: "dm-b" },
      expected,
      { workerId: "worker-a", sessionId: "dm-a" },
    ),
    "a response from Worker A must be discarded under Worker B",
  );
});

Deno.test("active Introduction state masks Worker A before Worker B detail resolves", async () => {
  const hook = await source(
    "../components/hive/hooks/useActiveHiveWorkerIntroduction.ts",
  );
  assert(
    hook.includes("selectCurrentHiveWorkerIntroductionDetail(") &&
      hook.includes("detail.id !== workerId") &&
      hook.includes("detail.dm_session_id !== sessionId") &&
      hook.includes("detail: currentDetail") &&
      hook.includes("introduction: currentDetail?.introduction ?? null"),
    "rendered Introduction detail must be synchronously masked to the exact current Worker DM",
  );
  assert(
    hook.includes("const captureCurrentDetailAction = useCallback") &&
      hook.includes("currentDetailRef.current") &&
      hook.includes("workers.retryIntroduction(action.detail.id)") &&
      hook.includes("workers.skipIntroduction(action.detail.id)") &&
      hook.includes("workers.confirmIntroduction(action.detail.id") &&
      hook.includes("workers.keepTalkingIntroduction(action.detail.id"),
    "every outbound Introduction action must recapture an exact current detail binding",
  );
});

Deno.test("Goal visibility distinguishes unresolved detail from loaded legacy null Introduction", async () => {
  const [hook, mobile, desktop] = await Promise.all([
    source("../components/hive/hooks/useActiveHiveWorkerIntroduction.ts"),
    source("../components/hive/HiveMobileThreadControls.tsx"),
    source("../components/hive/HiveThreadSurface.tsx"),
  ]);
  assert(
    hook.includes("if (!detail) return false;") &&
      hook.includes("detail.introduction?.status ?? null") &&
      hook.includes('status === null || status === "confirmed"') &&
      hook.includes('status === "skipped"'),
    "an unresolved detail must fail closed while an exact loaded legacy detail may have no Introduction",
  );
  const currentDetailGate =
    /canShowHiveWorkerGoalForIntroduction\(\s*introduction\.detail,?\s*\)/;
  assert(
    currentDetailGate.test(mobile) && currentDetailGate.test(desktop),
    "mobile and desktop Goal trackers must share the loaded-current-detail gate",
  );
});

Deno.test("review sheet keeps evidence typed and reserves transcript space", async () => {
  const sheet = await source(
    "../components/hive/HiveWorkerIntroductionSheet.tsx",
  );
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  assert(
    sheet.includes("What I understand") &&
      sheet.includes("fact.evidence_excerpt") &&
      sheet.includes("CATEGORY_LABELS[fact.kind]") &&
      sheet.includes('tool_expectation: "Tool expectation"') &&
      sheet.includes('memory_expectation: "Memory expectation"') &&
      sheet.includes("new Set(proposal?.facts.map") &&
      sheet.includes("aria-checked={selected}"),
    "the sheet must show exact evidence and select all proposed facts initially",
  );
  assert(
    sheet.includes("Confirm selected") && sheet.includes("Keep talking"),
    "the proposal exposes one confirm action and a quiet conversation escape",
  );
  assert(
    thread.includes("composerReserveHeight + introductionReserveHeight") &&
      thread.includes("onHeightChange={setIntroductionReserveHeight}"),
    "the measured sheet height must become transcript bottom padding",
  );
  for (
    const status of [
      '"queued"',
      '"running"',
      '"review_ready"',
      '"failed"',
      '"needs_recovery"',
    ]
  ) {
    assert(
      thread.includes(`introductionStatus === ${status}`),
      `composer disable contract must include ${status}`,
    );
  }
  assert(
    !thread.includes('introductionStatus === "awaiting_context"'),
    "awaiting-context conversation remains writable",
  );
  assert(
    thread.includes("const introductionIsResolving = Boolean(") &&
      thread.includes("introduction.detail?.id !== activeWorker.id") &&
      thread.includes("introductionIsResolving ||") &&
      thread.includes("? workerBinding.retry") &&
      thread.includes('? "Retry Hive conversation binding"'),
    "desktop must keep the Worker composer fail-closed until exact Introduction detail resolves and expose binding recovery",
  );
  assert(
    thread.includes("introductionDetailError") &&
      thread.includes('"Retry Worker details"') &&
      thread.includes(": introduction.refresh") &&
      thread.includes("? composerReserveHeight") &&
      thread.includes("zIndex: 30"),
    "desktop must expose exact-detail recovery without enabling the composer",
  );
  assert(
    thread.includes("sessionView.messages.length === 0") &&
      thread.includes("sessionView.isStreaming || sessionView.isThinking"),
    "the preexisting empty opening-stream composer guard must remain intact",
  );
  assert(
    sheet.includes("reviewIsStale") &&
      sheet.includes('label="Keep talking"') &&
      sheet.includes('projection?.state === "review_ready"') &&
      sheet.includes("projection.is_current_through") &&
      !sheet
        .slice(
          sheet.indexOf("if (reviewIsStale && proposal)"),
          sheet.indexOf("if (proposalCanConfirm && proposal)"),
        )
        .includes("state.confirm"),
    "a stale review-ready proposal must expose Keep talking but never confirmation",
  );
  assert(
    sheet.includes(
      "Keep talking in the composer below to create a new review boundary",
    ),
    "NeedsAttention without a proposal must present continued canonical context as the primary escape",
  );
});

Deno.test("normal Introduction context gathering keeps an exact-bound Skip escape", async () => {
  const sheet = await source(
    "../components/hive/HiveWorkerIntroductionSheet.tsx",
  );
  const crew = await source("../components/hive/HiveCrewView.tsx");
  const gatheringStart = sheet.indexOf(
    'if (introduction.status === "awaiting_context")',
  );
  const gatheringEnd = sheet.indexOf("return null;", gatheringStart);
  assert(
    gatheringStart >= 0 &&
      gatheringEnd > gatheringStart &&
      sheet.slice(gatheringStart, gatheringEnd).includes(
        'label="Skip setup"',
      ) &&
      sheet.slice(gatheringStart, gatheringEnd).includes("state.skip()"),
    "a normal awaiting-context DM must offer the binding-fenced Skip action",
  );
  assert(
    sheet.includes('introduction.status === "awaiting_context"') &&
      sheet.includes("Shape this Worker"),
    "normal context gathering must remain visibly projected above the composer",
  );
  assert(
    crew.includes(
      'worker.introduction_status === "awaiting_context"',
    ) && crew.includes("introductionGathering"),
    "the Worker roster must retain the same Skip escape during gathering",
  );
});

Deno.test("Hive Worker Introduction failures stay visible without unhandled promises", async () => {
  const crew = await source("../components/hive/HiveCrewView.tsx");
  const retryStart = crew.indexOf(".retryIntroduction(worker.id)");
  const retryEnd = crew.indexOf("onSkipIntroduction", retryStart);
  const skipStart = crew.indexOf("workers.skipIntroduction(worker.id)");
  assert(
    retryStart >= 0 && retryEnd > retryStart,
    "retry control must be rendered",
  );
  assert(skipStart >= 0, "skip control must be rendered");
  assert(
    crew.slice(retryStart, retryEnd).includes(".catch(() => undefined)"),
    "retry rejection must be consumed after the hook projects workers.error",
  );
  assert(
    crew.slice(skipStart, skipStart + 120).includes(".catch(() => undefined)"),
    "skip rejection must be consumed after the hook projects workers.error",
  );
  assert(
    crew.includes("{workers.error}") &&
      crew.includes("introduction_last_error"),
    "request and durable Introduction failures must remain visible in the roster",
  );
  assert(
    crew.includes('worker.introduction_status === "failed"') &&
      crew.includes('worker.introduction_status === "needs_recovery"'),
    "retry controls must be limited to recoverable lifecycle states",
  );
});

Deno.test("Worker lifecycle mutations are revisioned, replay-stable, and read-only when inactive", async () => {
  const hook = await source("../components/hive/hooks/useHiveWorkers.ts");
  const activeHook = await source(
    "../components/hive/hooks/useActiveHiveWorkerIntroduction.ts",
  );
  const compactHook = hook.replace(/\s+/g, " ");
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const sheet = await source(
    "../components/hive/HiveWorkerIntroductionSheet.tsx",
  );
  assert(
    hook.includes("expectedRevision: worker.revision") &&
      hook.includes("workerMutationAttemptsRef.current.get(attemptId)") &&
      hook.includes("idempotencyKey: workerMutationKey(action, id)"),
    "Worker mutations must freeze roster revision and replay key together",
  );
  assert(
    compactHook.includes("api.pauseHiveWorker(id, attempt.expectedRevision") &&
      compactHook.includes(
        "api.resumeHiveWorker( id, attempt.expectedRevision",
      ) &&
      compactHook.includes(
        "api.archiveHiveWorker( id, attempt.expectedRevision",
      ),
    "every lifecycle action must submit the frozen expected revision",
  );
  assert(
    thread.includes('introduction.worker?.status === "paused"') &&
      thread.includes('introduction.worker?.status === "archived"') &&
      thread.includes("workerDisablesComposer ||") &&
      thread.includes("sessionView.messages.length === 0"),
    "paused/archive read-only state must compose with the original opening-stream guard",
  );
  assert(
    sheet.includes("Worker paused") &&
      sheet.includes('label="Resume Worker"') &&
      sheet.includes("state.resume().catch") &&
      sheet.includes("Worker archived") &&
      sheet.includes("history are retained"),
    "inactive conversations must explain retained history and expose direct resume",
  );
  assert(
    activeHook.includes("const action = captureCurrentDetailAction();") &&
      activeHook.includes("await runCurrentAction(") &&
      activeHook.includes("action.expected,") &&
      activeHook.includes("() => workers.resumeWorker(action.detail.id)") &&
      activeHook.includes("adopt(await action(), expected)"),
    "direct resume must use the same Worker/session/generation adoption fence",
  );
});
