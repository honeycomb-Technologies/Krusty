declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

import {
  clearAcceptedHiveGroupMessageAttempt,
  retainHiveGroupMessageAttempt,
} from "../components/hive/hooks/groupMessageReplay.ts";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function sequentialKeys(...keys: string[]): (groupId: string) => string {
  let index = 0;
  return () => {
    const key = keys[index];
    index += 1;
    if (!key) throw new Error("test idempotency key factory exhausted");
    return key;
  };
}

Deno.test("rejected group send retries the identical body with the same key", () => {
  const createKey = sequentialKeys("group-key-a", "group-key-b");
  const rejected = retainHiveGroupMessageAttempt(
    null,
    "group-a",
    { message: "compare the plans" },
    createKey,
  );

  // Rejection deliberately leaves the attempt retained.
  const retry = retainHiveGroupMessageAttempt(
    rejected,
    "group-a",
    { message: "compare the plans" },
    createKey,
  );

  assert(retry === rejected, "an identical retry must retain the attempt");
  assert(
    retry.idempotencyKey === "group-key-a",
    "an identical retry must reuse the original idempotency key",
  );
});

Deno.test("accepted group send releases its key before an intentional repeat", () => {
  const createKey = sequentialKeys("group-key-a", "group-key-b");
  const accepted = retainHiveGroupMessageAttempt(
    null,
    "group-a",
    { message: "compare the plans" },
    createKey,
  );
  const cleared = clearAcceptedHiveGroupMessageAttempt(accepted, accepted);
  const repeated = retainHiveGroupMessageAttempt(
    cleared,
    "group-a",
    { message: "compare the plans" },
    createKey,
  );

  assert(cleared === null, "an accepted attempt must be released");
  assert(
    repeated.idempotencyKey === "group-key-b",
    "a new explicit send after acceptance must receive a new key",
  );
});

Deno.test("changed group or body replaces the replay attempt without stale clearing", () => {
  const createKey = sequentialKeys(
    "group-key-a",
    "group-key-b",
    "group-key-c",
  );
  const first = retainHiveGroupMessageAttempt(
    null,
    "group-a",
    { message: "compare the plans" },
    createKey,
  );
  const changedBody = retainHiveGroupMessageAttempt(
    first,
    "group-a",
    { message: "challenge the plans" },
    createKey,
  );
  const afterStaleAcceptance = clearAcceptedHiveGroupMessageAttempt(
    changedBody,
    first,
  );
  const changedGroup = retainHiveGroupMessageAttempt(
    afterStaleAcceptance,
    "group-b",
    { message: "challenge the plans" },
    createKey,
  );

  assert(
    changedBody.idempotencyKey === "group-key-b",
    "a changed request body must receive a new key",
  );
  assert(
    afterStaleAcceptance === changedBody,
    "an older accepted response must not clear the newer attempt",
  );
  assert(
    changedGroup.idempotencyKey === "group-key-c",
    "a changed group must receive a new key",
  );
});

Deno.test("group room clears replay state only after HTTP acceptance", async () => {
  const hook = await Deno.readTextFile(
    new URL(
      "../components/hive/hooks/useHiveGroupRoom.ts",
      import.meta.url,
    ),
  );
  const awaitSend = hook.indexOf("await client.sendHiveGroupMessage(");
  const clearAccepted = hook.indexOf(
    "sendAttemptRef.current = clearAcceptedHiveGroupMessageAttempt(",
  );
  const rejection = hook.indexOf("} catch (sendError)", awaitSend);

  assert(
    hook.includes("sendAttemptRef.current = attempt") &&
      hook.includes("attempt.idempotencyKey"),
    "the hook must retain and submit the exact prepared attempt",
  );
  assert(
    awaitSend >= 0 &&
      clearAccepted > awaitSend &&
      rejection > clearAccepted,
    "the hook must clear replay state only after the request is accepted",
  );
  assert(
    hook.includes(
      "sameHiveGroupMessageAttempt(sendAttemptRef.current, attempt)",
    ),
    "a stale rejected request must not overwrite a newer attempt",
  );
});
