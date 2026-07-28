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
