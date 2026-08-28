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

Deno.test("Worker governor reads are abortable and binding-generation fenced", async () => {
  const hook = await source(
    "../components/hive/hooks/useHiveWorkerGovernor.ts",
  );
  assert(
    hook.includes("new AbortController()") &&
      hook.includes("generation !== loadGenerationRef.current") &&
      hook.includes("controller.abort()"),
    "late governor reads must be cancelled and generation fenced",
  );
  assert(
    hook.includes("next.worker_id !== workerId") &&
      hook.includes("next.dm_session_id !== sessionId") &&
      hook.includes("next.policy.worker_id !== workerId"),
    "the response must stay bound to the exact Worker, DM, and policy",
  );
  assert(
    hook.includes("clearTimeout(timerRef.current)") &&
      hook.includes("const GOVERNOR_POLL_MS = 15_000"),
    "current usage polling must be bounded and cleaned up",
  );
  assert(
    hook.includes(".catch((loadError: unknown)") &&
      hook.includes(".finally(() =>"),
    "background reads must own rejection and loading cleanup",
  );
  assert(
    hook.includes("governorBindingKey(") &&
      hook.includes("projectionState?.bindingKey === exactBindingKey") &&
      hook.includes("errorState?.bindingKey === exactBindingKey") &&
      hook.includes("recoveryGrantState?.bindingKey === exactBindingKey") &&
      hook.includes("recoveryErrorState?.bindingKey === exactBindingKey"),
    "an existing Worker A projection or recovery state must be masked synchronously while Worker B loads",
  );
});

Deno.test("unresolved recovery is exact-binding fenced and safely replayable", async () => {
  const hook = await source(
    "../components/hive/hooks/useHiveWorkerGovernor.ts",
  );
  assert(
    hook.includes("projection.unresolved_started_count <= 0") &&
      hook.includes("!projection.response_loss_recovery_required") &&
      hook.includes("workerDmSessionId !== sessionId") &&
      hook.includes("response.worker_id !== workerId") &&
      hook.includes('response.status === "response_loss_acknowledged"') &&
      hook.includes("response.bypass_unresolved_provider_call === false") &&
      hook.includes("response.grant_id === null"),
    "recovery must be offered only for one exact Worker DM boundary and validate grant-free response-loss settlement",
  );
  assert(
    hook.includes("let attempt = recoveryAttemptRef.current") &&
      hook.includes("idempotencyKey: attempt.idempotencyKey") &&
      hook.includes("recoveryAttemptRef.current = null") &&
      hook.includes("bindingGeneration !== bindingGenerationRef.current"),
    "a failed action must retain one key for replay while late responses remain binding fenced",
  );
  assert(
    hook.includes(".grantHiveWorkerGovernorRecovery(workerId") &&
      hook.includes("catch (grantError: unknown)") &&
      hook.includes("setRecoveryErrorState({"),
    "the action must own its rejection and expose a truthful error",
  );
  assert(
    hook.includes("Date.parse(recoveryGrant.expires_at)") &&
      hook.includes('recoveryGrant.status === "response_loss_acknowledged"') &&
      hook.includes("const expiryTimer = setTimeout(") &&
      hook.includes("setRecoveryGrantState((current)") &&
      hook.includes("projection?.unresolved_started_count !== 0") &&
      hook.includes("Prepare a fresh short-lived recovery") &&
      hook.includes("Retry replays the same recovery request safely"),
    "a confirmed grant must clear on expiry or acknowledged projection so the next click rotates authority",
  );
});

Deno.test("Worker direct and editor surfaces show truthful governor recovery", async () => {
  const panel = await source(
    "../components/hive/HiveWorkerGovernorPanel.tsx",
  );
  const direct = await source(
    "../components/hive/HiveWorkerDirectChat.tsx",
  );
  const editor = await source(
    "../components/hive/HiveWorkerEditorModal.tsx",
  );
  for (
    const truth of [
      "Calls",
      "Tokens",
      "Estimated cost",
      "Auto wake",
      "Next",
      "Quiet",
      "Idle",
      "DST gap",
      "Unresolved starts",
    ]
  ) {
    assert(panel.includes(truth), `governor panel must show ${truth}`);
  }
  assert(
    panel.includes("Policy is read only.") &&
      !panel.includes("compare_and_swap") &&
      !panel.includes("updateHiveWorkerGovernor"),
    "policy editing must remain outside the read-only mobile vertical",
  );
  assert(
    panel.includes("hasRecoveryBoundary") &&
      panel.includes("hasResponseLoss") &&
      panel.includes("hasCombinedRecovery") &&
      panel.includes("Prepare next direct message") &&
      panel.includes("Acknowledge missing reply") &&
      panel.includes("creates no bypass grant") &&
      panel.includes("never replays the completed provider call") &&
      panel.includes("response_loss_acknowledged_with_grant") &&
      panel.includes("An older uncertain call still requires the recovery") &&
      panel.includes("older unresolved provider") &&
      panel.includes(
        "prepare one short-lived recovery for the older uncertain call",
      ) &&
      panel.includes("bypasses only unresolved-call reconciliation") &&
      panel.includes("Goal, group, review, and") &&
      panel.includes("immutable") &&
      panel.includes("accounting record") &&
      panel.includes('state.recoveryGrant.status === "already_available"'),
    "the recovery action must distinguish unresolved authority from grant-free response-loss acknowledgment",
  );
  assert(
    direct.includes("<HiveWorkerGovernorPanel") &&
      direct.includes("compact") &&
      editor.includes("<HiveWorkerGovernorPanel") &&
      editor.includes("poll={false}"),
    "the active DM should poll one compact panel while the editor uses a bounded snapshot",
  );
});
