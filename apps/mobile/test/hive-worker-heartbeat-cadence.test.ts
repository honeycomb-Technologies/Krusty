import {
  buildWorkerHeartbeatCreateFields,
  buildWorkerHeartbeatUpdateFields,
  MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS,
  parseWorkerHeartbeatCadence,
} from "../components/hive/worker-heartbeat-cadence.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${
        JSON.stringify(actual)
      }`,
    );
  }
}

Deno.test("heartbeat cadence parses blank, exact positive u32 values, and leading zeroes", () => {
  assertEquals(
    parseWorkerHeartbeatCadence("   "),
    { value: null, error: null },
    "blank must remain an explicit local null",
  );
  assertEquals(
    parseWorkerHeartbeatCadence("000900"),
    { value: 900, error: null },
    "a valid whole number must retain its exact numeric value",
  );
  assertEquals(
    parseWorkerHeartbeatCadence(
      String(MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS),
    ),
    { value: MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS, error: null },
    "the server's inclusive u32 maximum must be accepted",
  );
});

Deno.test("heartbeat cadence rejects zero, fractions, signs, text, and u32 overflow", () => {
  for (
    const invalid of [
      "0",
      "1.5",
      "-1",
      "+1",
      "one hour",
      String(MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS + 1),
    ]
  ) {
    const parsed = parseWorkerHeartbeatCadence(invalid);
    assert(parsed.value === null, `${invalid} must not produce a value`);
    assert(
      parsed.error !== null,
      `${invalid} must explain its validation error`,
    );
  }
});

Deno.test("heartbeat cadence request fields preserve create null and unchanged update semantics", () => {
  assertEquals(
    buildWorkerHeartbeatCreateFields(null),
    {},
    "blank create must omit the field for core's autonomy-aware default",
  );
  assertEquals(
    buildWorkerHeartbeatCreateFields(120),
    { heartbeat_interval_secs: 120 },
    "create must send the exact entered cadence",
  );
  assertEquals(
    buildWorkerHeartbeatUpdateFields(null, 900),
    {},
    "blank edit must not clear or clobber a stored cadence",
  );
  assertEquals(
    buildWorkerHeartbeatUpdateFields(900, 900),
    {},
    "an unchanged edit must omit the cadence field",
  );
  assertEquals(
    buildWorkerHeartbeatUpdateFields(120, null),
    { heartbeat_interval_secs: 120 },
    "a new edit value must be sent when persisted state was null",
  );
  assertEquals(
    buildWorkerHeartbeatUpdateFields(120, 900),
    { heartbeat_interval_secs: 120 },
    "a changed edit must send the exact new cadence",
  );
});

Deno.test("Worker editor exposes an accessible autonomy-aware cadence control", async () => {
  const [editor, apiTypes] = await Promise.all([
    Deno.readTextFile(
      new URL(
        "../components/hive/HiveWorkerEditorModal.tsx",
        import.meta.url,
      ),
    ),
    Deno.readTextFile(
      new URL("../../../packages/api/src/types.ts", import.meta.url),
    ),
  ]);

  assert(
    editor.includes("Heartbeat cadence · seconds") &&
      editor.includes('accessibilityLabel="Heartbeat cadence in seconds"') &&
      editor.includes('keyboardType="number-pad"') &&
      editor.includes('inputMode="numeric"'),
    "the cadence field must be labeled, screen-reader accessible, and numeric",
  );
  assert(
    editor.includes("Manual and Scheduled Workers can retain a cadence") &&
      editor.includes("only while autonomy is Always on") &&
      editor.includes("Leave blank to use the server default") &&
      editor.includes("keep the stored value unchanged"),
    "the control must explain when cadence applies and its blank semantics",
  );
  assert(
    editor.includes("heartbeatCadence.error === null") &&
      editor.includes("buildWorkerHeartbeatCreateFields(") &&
      editor.includes("buildWorkerHeartbeatUpdateFields("),
    "save must be validation-gated and use mode-specific request semantics",
  );
  assert(
    apiTypes.includes("heartbeat_interval_secs?: number;") &&
      apiTypes.includes(
        "/** Partial update: absent fields keep their current value. */",
      ),
    "the editor helper must remain aligned with the public create/update API",
  );
});
