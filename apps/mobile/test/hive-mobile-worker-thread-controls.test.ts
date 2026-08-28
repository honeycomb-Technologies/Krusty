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

Deno.test("mobile Worker controls reuse one roster and the stable transcript", async () => {
  const index = await source("../app/(tabs)/index.tsx");
  const screen = await source("../components/hive/HiveScreen.tsx");
  const [controls, activeSurface] = await Promise.all([
    source("../components/hive/HiveMobileThreadControls.tsx"),
    source("../components/chat-screen/ActiveConversationSurface.tsx"),
  ]);

  assert(
    index.includes(
      'const hiveWorkers = useHiveWorkers(activeMode === "hive")',
    ) &&
      index.includes("workers={hiveWorkers}") &&
      controls.includes("workers: HiveWorkersState"),
    "desktop management and mobile thread controls must share one Hive roster owner",
  );
  assert(
    !screen.includes("useHiveWorkers(") &&
      screen.includes("workers: HiveWorkersState"),
    "HiveScreen must consume the shared roster instead of starting a duplicate read",
  );
  assert(
    index.includes("<ActiveConversationSurface") &&
      index.includes("<HiveMobileThreadControls") &&
      !controls.includes("<ChatTranscript") &&
      !controls.includes("useSessionStore") &&
      activeSurface.includes("renderThreadControls?.(activity)"),
    "mobile Worker controls must augment, never duplicate, the stable transcript",
  );
  assert(
    activeSurface.includes("const tail = messages.at(-1)") &&
      activeSurface.includes("transcriptTailKey:") &&
      activeSurface.includes("const messageCount = messages.length") &&
      activeSurface.includes("useMemo<ActiveConversationActivity>"),
    "the sole transcript owner must memoize the primitive control projection",
  );
});

Deno.test("mobile Worker DMs expose exact Introduction, Goal, and composer controls", async () => {
  const index = await source("../app/(tabs)/index.tsx");
  const controls = await source(
    "../components/hive/HiveMobileThreadControls.tsx",
  );

  assert(
    controls.includes("useActiveHiveWorkerBinding(") &&
      controls.includes("useActiveHiveWorkerIntroduction({") &&
      controls.includes("useHiveWorkerGoal({"),
    "all Worker-owned controls must bind to the exact active DM",
  );
  assert(
    controls.includes("<HiveWorkerIntroductionSheet") &&
      controls.includes("<HiveWorkerGoalTracker") &&
      controls.includes("<HiveWorkerComposer") &&
      controls.includes("introductionAllowsGoalTracker"),
    "mobile Worker DMs must expose the same dedicated lower controls as desktop",
  );
  assert(
    controls.includes('introductionStatus === "review_ready"') &&
      controls.includes("introductionIsResolving") &&
      controls.includes("introduction.detail?.id !== activeWorker.id"),
    "review-ready and unresolved Introduction state must fail closed before sending",
  );
  assert(
    controls.includes('workerBinding.kind === "primary_hive"') &&
      controls.includes("workerBinding.isResolving") &&
      controls.includes("primaryComposer"),
    "generic Hive controls must remain primary-thread-only and hidden while binding resolves",
  );
  assert(
    index.includes("mobileHiveIntroductionReserveHeight +") &&
      index.includes("mobileHiveGoalTrackerReserveHeight +") &&
      controls.includes("bottom={composerHeight + introductionHeight + 8}") &&
      controls.includes("bottom={composerHeight}"),
    "measured mobile sheets must reserve transcript space in their exact stack order",
  );
});

Deno.test("mobile binding failures stay fail-closed and expose accessible retry", async () => {
  const controls = await source(
    "../components/hive/HiveMobileThreadControls.tsx",
  );
  assert(
    controls.includes("workerBinding.error && !workerBinding.isResolving") &&
      controls.includes("onRetry={workerBinding.retry}") &&
      controls.includes(
        'retryLabel="Retry Hive conversation binding"',
      ) &&
      controls.includes('accessibilityRole="alert"') &&
      controls.includes("accessibilityLabel={retryLabel}"),
    "an exhausted binding lookup must keep the composer hidden while exposing an accessible retry",
  );
  assert(
    controls.includes("introductionDetailError") &&
      controls.includes('retryLabel="Retry Worker details"') &&
      controls.includes("onRetry={introduction.refresh}") &&
      controls.includes("bottom={composerHeight + 8}") &&
      controls.includes("zIndex: 30"),
    "a failed exact Worker detail read must stay read-only with explicit recovery",
  );
});
