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

Deno.test("active Worker binding retries transient lookup failures and surfaces recovery", async () => {
  const binding = await source(
    "../components/hive/hooks/useActiveHiveWorkerBinding.ts",
  );

  assert(
    binding.includes("retry: () => void") &&
      binding.includes("const retry = useCallback(() =>") &&
      binding.includes(
        "setDirectVerificationSessionId(sessionIdRef.current)",
      ) &&
      binding.includes("setRetryRevision((current) => current + 1)") &&
      binding.includes("directVerificationSessionId !== sessionId") &&
      binding.includes("scopedError?.sessionId === sessionId"),
    "a terminal binding failure must expose retry without re-adopting the roster before direct verification",
  );
  assert(
    binding.includes("BINDING_AUTO_RETRY_DELAYS_MS = [200, 600]") &&
      binding.includes("attempt <= BINDING_AUTO_RETRY_DELAYS_MS.length") &&
      binding.includes("isTransientLookupError(lookupError)") &&
      binding.includes("await waitForRetryDelay(") &&
      binding.includes('signal.addEventListener("abort", handleAbort'),
    "transient lookups must retry a bounded number of times with abortable backoff",
  );
  assert(
    binding.includes("status === 408") && binding.includes("status === 425") &&
      binding.includes("status === 429") && binding.includes("status >= 500") &&
      binding.includes('kind: "invalid"') &&
      binding.includes("isResolving: false"),
    "permanent or exhausted lookup failures must remain fail-closed and stop claiming to resolve",
  );
});

Deno.test("Worker DM selection preserves the coalesced loader behind an exact-session fence", async () => {
  const [index, actions] = await Promise.all([
    source("../app/(tabs)/index.tsx"),
    source("../components/chat-screen/useSessionActions.ts"),
  ]);
  const compact = index.replace(/\s+/g, " ");

  assert(
    actions.includes("quietDelayMs: 72") &&
      actions.includes(
        "scheduleSessionSelection({ id, sessionType: targetType })",
      ),
    "Worker navigation must retain the existing latest-intent coalescing boundary",
  );
  assert(
    compact.includes(
      "pendingHiveThreadSessionId !== null && sessionId !== pendingHiveThreadSessionId",
    ) &&
      compact.includes(
        "if (sessionId !== pendingHiveThreadSessionId) return;",
      ) &&
      compact.includes(
        'setPendingHiveThreadSessionId(null); setHiveTopLevel("hive");',
      ),
    "the old transcript must stay fenced until the store adopts the exact requested session",
  );
  assert(
    compact.includes(
      "const handleOpenSession = useCallback( (session: SessionResponse) => { setPendingHiveThreadSessionId(null); void loadSession(session);",
    ) &&
      compact.includes('if (activeMode !== "hive") {') &&
      compact.includes('if (requestedMode !== "hive") {') &&
      compact.includes(
        "setPendingHiveThreadSessionId(null); } return; } if (sessionId !== pendingHiveThreadSessionId) return;",
      ),
    "a cross-mode Hive request must retain the fence while a superseding non-Hive selection clears it",
  );
});
