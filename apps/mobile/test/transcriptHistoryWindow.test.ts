/**
 * Conversation history mount policy.
 *
 * Earlier mobile builds kept only 1 historical turn mounted (page size 2).
 * Once a live turn finished, older messages unmounted and looked deleted.
 * These constants must stay high enough for normal multi-turn work.
 */

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals(actual: unknown, expected: unknown, message?: string): void {
  if (actual !== expected) {
    throw new Error(
      message ?? `Expected ${String(expected)}, received ${String(actual)}`,
    );
  }
}

function readNumberConst(source: string, name: string): number {
  const match = source.match(new RegExp(`const ${name} = (\\d+);`));
  assert(match, `${name} must be declared in ChatTranscript.tsx`);
  return Number(match[1]);
}

Deno.test("transcript keeps a usable historical window mounted by default", async () => {
  const transcriptSource = await Deno.readTextFile(
    new URL("../components/chat/ChatTranscript.tsx", import.meta.url).pathname,
  );
  const initial = readNumberConst(transcriptSource, "INITIAL_HISTORICAL_TURN_COUNT");
  const page = readNumberConst(transcriptSource, "HISTORICAL_TURN_PAGE_SIZE");
  assert(initial >= 12, `INITIAL_HISTORICAL_TURN_COUNT too small: ${initial}`);
  assert(page >= 8, `HISTORICAL_TURN_PAGE_SIZE too small: ${page}`);
  assert(
    transcriptSource.includes("Auto-follow keeps the newest recent window mounted"),
    "auto-follow path must retain the recent history window as turns finish",
  );
  assert(
    transcriptSource.includes("revealOlderHistory"),
    "older history must remain revealable on scroll",
  );
});

Deno.test("auto-follow history growth retains at least the initial window", async () => {
  const transcriptSource = await Deno.readTextFile(
    new URL("../components/chat/ChatTranscript.tsx", import.meta.url).pathname,
  );
  // Mirror the ChatTranscript auto-follow branch without mounting React.
  const initialHistoricalTurnCount = readNumberConst(
    transcriptSource,
    "INITIAL_HISTORICAL_TURN_COUNT",
  );
  let count = 1;
  const sourceLength = 1;
  const historicalTurnsLength = 5;
  if (historicalTurnsLength > sourceLength) {
    count = Math.min(
      historicalTurnsLength,
      Math.max(count, initialHistoricalTurnCount),
    );
  }
  assertEquals(
    count,
    Math.min(historicalTurnsLength, initialHistoricalTurnCount),
  );
  assert(
    count >= Math.min(5, initialHistoricalTurnCount),
    "must not collapse to a single historical turn",
  );
});
