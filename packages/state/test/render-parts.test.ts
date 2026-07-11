/**
 * Deno tests for render-part text join parity (live stream vs stored reload).
 *
 * Run: deno test packages/state/test/render-parts.test.ts
 */

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (actual !== expected) {
    throw new Error(
      `${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`,
    );
  }
}

/** Must match streaming.ts and messages.ts joinAdjacentText policy. */
function joinAdjacentText(existing: string, next: string): string {
  if (!existing) return next;
  if (!next) return existing;
  return existing + next;
}

Deno.test("live and stored text joins use raw concatenation", () => {
  // Stream deltas: "Hel" + "lo" + " world"
  const live = ["Hel", "lo", " world"].reduce(
    (acc, chunk) => joinAdjacentText(acc, chunk),
    "",
  );
  // Stored consecutive text blocks for the same continuous prose
  const stored = ["Hello", " world"].reduce(
    (acc, chunk) => joinAdjacentText(acc, chunk),
    "",
  );
  assertEquals(live, "Hello world", "live stream should concat");
  assertEquals(stored, "Hello world", "stored blocks should match live");
  assertEquals(live, stored, "parity between live and stored join");
});

Deno.test("joinAdjacentText does not invent separators", () => {
  assertEquals(joinAdjacentText("code", "base"), "codebase", "no glue space");
  assertEquals(joinAdjacentText("a\n", "b"), "a\nb", "preserve existing newline");
  assertEquals(joinAdjacentText("", "only"), "only", "empty existing");
  assertEquals(joinAdjacentText("only", ""), "only", "empty next");
});
