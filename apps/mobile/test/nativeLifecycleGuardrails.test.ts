declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("Live Activity replacement keeps an owned handle until exact end", async () => {
  const source = await Deno.readTextFile(
    new URL("../hooks/useLiveActivity.ts", import.meta.url).pathname,
  );

  assert(
    source.includes("releaseCurrentActivityImmediately"),
    "session replacement must go through one ownership release path",
  );
  assert(
    source.includes("endInFlightRef.current"),
    "a replacement start must wait for the owned handle's end",
  );
  assert(
    source.includes("pendingStartRef.current = { sessionId, chatTitle"),
    "rapid A to B to C requests must coalesce to the latest desired session",
  );
  assert(
    !source.includes("const previousActivity = activityRef.current"),
    "the old timer-only handle ownership path must not return",
  );
  assert(
    source.includes("activityRef.current === activity") &&
      source.includes("sessionIdRef.current === activitySessionId"),
    "an old activity update must not publish state into its replacement session",
  );
  assert(
    source.indexOf("endInFlightRef.current = ending") !==
      source.lastIndexOf("endInFlightRef.current = ending"),
    "both replacement and normal completion must serialize the next start",
  );
});

Deno.test("native WebView recovery is visible-only and bounded", async () => {
  for (
    const relativePath of [
      "../components/toolbox/ToolboxBrowser.tsx",
      "../components/toolbox/ToolboxTerminal.tsx",
    ]
  ) {
    const source = await Deno.readTextFile(
      new URL(relativePath, import.meta.url).pathname,
    );
    assert(
      source.includes("MAX_AUTOMATIC_RECOVERIES = 1"),
      `${relativePath} must cap automatic recovery`,
    );
    assert(
      source.includes("!visibleRef.current"),
      `${relativePath} must not recover while hidden`,
    );
    assert(
      source.includes("WEBVIEW_RECOVERY_COOLDOWN_MS"),
      `${relativePath} must cool down before native recreation`,
    );
    assert(
      source.includes("retryWebView"),
      `${relativePath} must provide explicit manual recovery`,
    );
    assert(
      source.includes("WEBVIEW_MOUNT_SETTLE_MS"),
      `${relativePath} must coalesce frantic tab changes`,
    );
  }
});

Deno.test("browser close does not release an unsettled single-flight request", async () => {
  const source = await Deno.readTextFile(
    new URL("../components/toolbox/ToolboxBrowser.tsx", import.meta.url)
      .pathname,
  );
  const pollingEffect = source.slice(
    source.indexOf("const poll = async"),
    source.indexOf("// Coalesce frantic tab/open transitions"),
  );
  assert(
    !pollingEffect.includes("loadPromiseRef.current = null"),
    "visibility cleanup must leave the active request joinable until settlement",
  );
  assert(
    source.includes(
      "if (loadPromiseRef.current) return loadPromiseRef.current",
    ),
    "reopen must join the active request",
  );
});

Deno.test("rapid mode input defers heavy activation to the latest requested mode", async () => {
  const screen = await Deno.readTextFile(
    new URL("../app/(tabs)/index.tsx", import.meta.url).pathname,
  );
  const actions = await Deno.readTextFile(
    new URL("../app/(tabs)/chat-screen/useSessionActions.ts", import.meta.url).pathname,
  );

  assert(
    screen.includes("createLatestIntentScheduler")
      && screen.includes("quietDelayMs: 24")
      && screen.includes("maxDelayMs: 80")
      && screen.includes("startTransition(() => setActiveMode(mode))"),
    "heavy mode activation must coalesce to the latest intent with a hard deadline",
  );
  assert(
    screen.includes("modeForHorizontalSwipe(\n        requestedMode,"),
    "rapid swipes must advance from the latest intent instead of stale deferred content",
  );
  assert(
    screen.includes("mode={requestedMode}"),
    "the visible tab selection must respond before deferred surface activation",
  );
  assert(
    screen.includes("const activeTab = tabForSessionType(activeMode)"),
    "composer and session actions must remain bound to the committed deferred mode",
  );
  assert(
    screen.includes("activateSessionType(activeMode)"),
    "session warming must follow only the deferred winning mode",
  );
  assert(
    !screen.includes("modeIntentRef") && !screen.includes("commitModeIntent"),
    "mode changes must not drain a serialized commit backlog",
  );
  assert(
    screen.includes("sessionType={activeMode}")
      && !screen.includes('activeMode === "mako" ? ('),
    "Chat, Code, and Mako must reconcile through one stable mobile transcript tree",
  );
  assert(
    !actions.includes("requestedTabRef"),
    "the lower session action layer must not own a competing intent deduper",
  );
  assert(
    !actions.includes("if (index === activeTab)"),
    "rapid duplicate suppression must not compare against stale rendered state",
  );
  assert(
    actions.includes("sessionSelectionSchedulerRef.current?.submit(intent)"),
    "rapid session selection must coalesce before expensive hydration",
  );
});

Deno.test("pending diagnostic completion cannot expose an actionable start button", async () => {
  const provider = await Deno.readTextFile(
    new URL("../diagnostics/MobileDiagnosticsProvider.tsx", import.meta.url).pathname,
  );
  const settings = await Deno.readTextFile(
    new URL("../components/settings/sections.tsx", import.meta.url).pathname,
  );

  assert(
    provider.includes("completionPending: pendingCompletionRef.current"),
    "the diagnostics context must expose durable completion ownership",
  );
  assert(
    settings.includes('disabled={!runId || completionPending || uploadState === "uploading"}'),
    "Start capture must be disabled until completion and any prior upload are settled",
  );
  assert(
    provider.includes("pendingCompletionRef.current || uploadingRef.current"),
    "capture rotation must reject an in-flight old-recorder upload",
  );
});
