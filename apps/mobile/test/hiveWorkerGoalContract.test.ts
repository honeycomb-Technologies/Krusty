import { isCurrentWorkerGoalMutation } from "../components/hive/hooks/workerGoalBindingFence.ts";

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

Deno.test("late Worker A Goal mutations cannot affect Worker B UI state", () => {
  const workerA = { generation: 4, workerId: "worker-a", sessionId: "dm-a" };
  assert(
    isCurrentWorkerGoalMutation(workerA, 4, "worker-a", "dm-a"),
    "the exact initiating binding should remain current",
  );
  assert(
    !isCurrentWorkerGoalMutation(workerA, 5, "worker-b", "dm-b"),
    "a late A response must be stale after switching to B",
  );
});

Deno.test("Worker A Goal projection is masked while Worker B loads", async () => {
  const hook = await source("../components/hive/hooks/useHiveWorkerGoal.ts");
  assert(
    hook.includes("selectCurrentHiveWorkerGoalProjection(") &&
      hook.includes("projection.worker_id !== workerId") &&
      hook.includes("projection.session_id !== sessionId") &&
      hook.includes("projection: currentProjection"),
    "the hook must never return a projection owned by a previous Worker or DM",
  );
  assert(
    hook.includes("const currentProjection =") &&
      hook.includes("!currentProjection ||") &&
      hook.includes("return currentProjection;"),
    "UI reads and outbound Goal mutations must share the exact current projection",
  );
});

Deno.test("Worker Goal hook fences reads, mutations, replays, and rollover polling", async () => {
  const hook = await source("../components/hive/hooks/useHiveWorkerGoal.ts");
  assert(
    hook.includes("new AbortController()") &&
      hook.includes("generation !== loadGenerationRef.current") &&
      hook.includes("bindingRef.current.workerId !== workerId") &&
      hook.includes("bindingRef.current.sessionId !== sessionId"),
    "Goal projection reads and failures must be abortable and exact-binding fenced",
  );
  assert(
    hook.includes("attemptsRef.current.clear()") &&
      hook.includes("savingAttemptRef.current === savingAttempt") &&
      hook.includes("isCurrentWorkerGoalMutation("),
    "binding changes must clear attempts and stale mutations must not alter saving/error state",
  );
  assert(
    hook.includes("mutationError instanceof MitsuroApiError") &&
      hook.includes("mutationError.status === 409") &&
      hook.includes("attemptsRef.current.delete(attemptId)"),
    "definite stale conflicts must release their replay key and refresh",
  );
  assert(
    hook.includes(
      "next.active_run || next.pending_acceptance ||",
    ) &&
      hook.includes("const ACTIVE_GOAL_POLL_MS = 5_000"),
    "an active Goal must keep a bounded safety poll through the no-run rollover gap",
  );
  assert(
    hook.includes("currentProjection.worker_id !== current.workerId") &&
      hook.includes("currentProjection.session_id !== current.sessionId") &&
      hook.includes("const currentProjection = requireCurrentProjection();"),
    "create and workspace bodies must come from the exact current Worker/DM projection",
  );
});

Deno.test("Worker Goal tracker is dedicated, measured, and handles rejected actions", async () => {
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const tracker = await source("../components/hive/HiveWorkerGoalTracker.tsx");
  const cancelGuard = await source(
    "../components/hive/worker-goal-cancel-confirmation.ts",
  );
  assert(
    thread.includes('workerBinding.kind === "worker_dm"') &&
      thread.includes("introductionAllowsGoalTracker") &&
      thread.includes("<HiveWorkerGoalTracker") &&
      thread.includes("key={`${activeWorker.id}:${sessionView.sessionId}`}"),
    "the tracker and its draft state must be scoped to one exact Worker DM",
  );
  assert(
    thread.includes("goalTrackerReserveHeight") &&
      thread.includes("onHeightChange={setGoalTrackerReserveHeight}"),
    "measured tracker height must reserve transcript space",
  );
  assert(
    tracker.includes("state.approve().catch(() => undefined)") &&
      tracker.includes("state.activate().catch(() => undefined)") &&
      tracker.includes("state.pause().catch(() => undefined)") &&
      tracker.includes("state.accept({") &&
      tracker.includes("state.reject(reviewReason).catch(() => undefined)") &&
      tracker.includes("state.setWorkspace(path)") &&
      tracker.includes(".catch(() => undefined)"),
    "hook-owned visible failures must never become unhandled promise rejections",
  );
  assert(
    tracker.includes('label="Retry Goal"') &&
      tracker.includes("onPress={state.refresh}") &&
      tracker.includes("state.error && !projection"),
    "a failed Goal projection read must remain visibly retryable",
  );
  assert(
    tracker.includes('label="Cancel"') &&
      tracker.includes("onPress={confirmCancel}") &&
      tracker.includes("confirmWorkerGoalCancellation({") &&
      tracker.includes("cancelGuardRef.current?.attempt(") &&
      tracker.includes("cancel: state.cancel") &&
      cancelGuard.includes("void attempt.then(") &&
      (cancelGuard.match(/attempts\.delete\(requestedTargetKey\);/g)?.length ??
          0) >= 2,
    "Cancel must remain visible and confirmation-guarded, with synchronous and asynchronous rejection paths released for safe retry",
  );
  assert(
    tracker.includes("<KeyboardAvoidingView") &&
      tracker.includes("<ScrollView") &&
      tracker.includes('label="Plan steps (one per line)"') &&
      tracker.includes("up to 12 plan steps"),
    "small phones need a keyboard-safe bounded multi-step Goal authoring sheet",
  );
});

Deno.test("Worker Goal acceptance is exact, evidence-bound, and replay safe", async () => {
  const hook = await source("../components/hive/hooks/useHiveWorkerGoal.ts");
  const tracker = await source("../components/hive/HiveWorkerGoalTracker.tsx");
  const apiTypes = await source("../../../packages/api/src/types.ts");
  assert(
    hook.includes(
      "pending.expected_worker_revision !== currentProjection.worker_revision",
    ) &&
      hook.includes("pending.expected_goal_revision !==") &&
      hook.includes("pending.acceptance_run_id") &&
      hook.includes('mutate(\n      "resolve_acceptance"'),
    "acceptance mutations must use the exact Worker, Goal, and acceptance-run fences through replay-key ownership",
  );
  assert(
    hook.includes('decision: "passed" | "waived"') &&
      hook.includes(
        "Pass or waive every required Goal criterion with concrete evidence",
      ) &&
      hook.includes("criteria.length !== expected.size"),
    "final-step acceptance must decide each required criterion exactly once with evidence",
  );
  assert(
    tracker.includes('accessibilityLabel="Review Worker Goal step"') &&
      tracker.includes('label="Accept result"') &&
      tracker.includes('label="Reject result"') &&
      tracker.includes(".catch(() => undefined)"),
    "accept/reject controls must be accessible and handle hook-owned promise failures",
  );
  assert(
    apiTypes.includes(
      "source_summary: HiveWorkerGoalAcceptanceSourceSummary",
    ) &&
      apiTypes.includes("evidence: HiveWorkerGoalSourceEvidence[]") &&
      apiTypes.includes("effect: HiveWorkerGoalSourceEffect") &&
      apiTypes.includes("counters: HiveWorkerGoalOutcomeCounters"),
    "the client contract must expose only the bounded typed acceptance source summary",
  );
  assert(
    tracker.includes('accessibilityLabel="Worker Goal source summary"') &&
      tracker.includes("pendingAcceptance.source_summary.effect.summary") &&
      tracker.includes("pendingAcceptance.source_summary.evidence.map") &&
      tracker.includes("pendingAcceptance.source_summary.counters.tool_calls"),
    "the review card must show exact-source evidence, effect, and basic counters before a decision",
  );
});

Deno.test("Worker Goal surface never mounts generic Agent plan controls", async () => {
  const thread = await source("../components/hive/HiveThreadSurface.tsx");
  const compact = thread.replace(/\s+/g, " ");
  assert(
    compact.includes(
      'onPlanConfirm={workerBinding.kind === "primary_hive" ? chat.onPlanConfirm : undefined}',
    ),
    "generic PlanConfirm must remain exclusive to the primary Hive conversation",
  );
  assert(
    !compact.includes('workerBinding.kind === "worker_dm" ? onPlanConfirm'),
    "a Worker DM must never start the generic Agent plan loop",
  );
});
