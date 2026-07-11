/**
 * Deno tests for assistant visual text-smoothing heuristics.
 *
 * Mirrors the pure helpers in apps/mobile/components/chat/assistantRenderPlan.ts
 * so mid-stream tool interruptions rejoin prose without gluing real boundaries.
 *
 * Run: deno test packages/state/test/assistant-smoothing.test.ts
 */

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (actual !== expected) {
    throw new Error(
      `${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`,
    );
  }
}

type Segment =
  | { type: "text"; id: string; content: string }
  | { type: "thinking"; id: string; content: string }
  | { type: "tool"; id: string }
  | { type: "exploration"; id: string };

function startsLikeNewBlock(content: string): boolean {
  return /^(#{1,6}\s|[-*+]\s|\d+[.)]\s|```|>|<\w)/.test(content);
}

function isSoftInterruption(segment: Segment): boolean {
  return (
    segment.type === "exploration" ||
    segment.type === "thinking" ||
    segment.type === "tool"
  );
}

function shouldMergeContinuationText(
  previousContent: string,
  nextContent: string,
  interveningSegments: Segment[],
): boolean {
  if (interveningSegments.length === 0) return false;
  if (!interveningSegments.every(isSoftInterruption)) return false;

  const previous = previousContent.trimEnd();
  const next = nextContent.trimStart();
  if (!previous || !next || startsLikeNewBlock(next)) return false;
  if (/```\s*$/.test(previous) || /^```/.test(next)) return false;

  const previousLooksUnfinished = /[A-Za-z0-9_'")\]]$/.test(previous);
  const nextLooksContinued = /^[a-z,.;:!?'"()\]}]/.test(next);
  return previousLooksUnfinished && nextLooksContinued;
}

function appendContinuationText(previous: string, next: string): string {
  if (!previous) return next;
  if (!next) return previous;
  if (/\s$/.test(previous) || /^\s/.test(next)) return previous + next;
  return previous + next.trimStart();
}

function smoothInterruptedText(segments: Segment[]): Segment[] {
  const smoothed: Segment[] = [];
  for (const segment of segments) {
    if (segment.type !== "text") {
      smoothed.push(segment);
      continue;
    }
    let previousTextIndex = -1;
    for (let index = smoothed.length - 1; index >= 0; index -= 1) {
      if (smoothed[index]?.type === "text") {
        previousTextIndex = index;
        break;
      }
    }
    const previousText = smoothed[previousTextIndex];
    const intervening =
      previousTextIndex >= 0 ? smoothed.slice(previousTextIndex + 1) : [];
    if (
      previousText?.type === "text" &&
      shouldMergeContinuationText(
        previousText.content,
        segment.content,
        intervening,
      )
    ) {
      smoothed[previousTextIndex] = {
        ...previousText,
        content: appendContinuationText(previousText.content, segment.content),
      };
      continue;
    }
    smoothed.push(segment);
  }
  return smoothed;
}

Deno.test("merges mid-sentence tool interruption", () => {
  const segments: Segment[] = [
    { type: "text", id: "t1", content: "Looking at the " },
    { type: "tool", id: "tool-1" },
    { type: "text", id: "t2", content: "registry next." },
  ];
  const smoothed = smoothInterruptedText(segments);
  // previous text + intervening tool remain; second text merges into first.
  assertEquals(smoothed.length, 2, "text+tool after merge");
  assert(smoothed[0]?.type === "text", "merged text first");
  assert(smoothed[1]?.type === "tool", "tool stays after merged text");
  if (smoothed[0]?.type === "text") {
    assertEquals(
      smoothed[0].content,
      "Looking at the registry next.",
      "prose rejoined",
    );
  }
});

Deno.test("does not merge when next starts a list/code block", () => {
  assert(
    !shouldMergeContinuationText(
      "See the following:",
      "- item one",
      [{ type: "tool", id: "t" }],
    ),
    "list boundary",
  );
  assert(
    !shouldMergeContinuationText(
      "Example:",
      "```ts\nconst x = 1\n```",
      [{ type: "thinking", id: "th", content: "..." }],
    ),
    "code fence start",
  );
});

Deno.test("does not merge across closed code fences", () => {
  assert(
    !shouldMergeContinuationText(
      "```\ncode\n```",
      "more prose here",
      [{ type: "tool", id: "t" }],
    ),
    "closed fence should not continue",
  );
});

Deno.test("merges legitimate lowercase continuation after tool", () => {
  assert(
    shouldMergeContinuationText(
      "The helper returns",
      "null when missing.",
      [{ type: "exploration", id: "e" }],
    ),
    "lowercase continuation after unfinished phrase",
  );
});

Deno.test("does not merge when previous looks finished", () => {
  assert(
    !shouldMergeContinuationText(
      "All done.",
      "Next we restart.",
      [{ type: "tool", id: "t" }],
    ),
    "sentence end should not glue next capital start — capital fails continued check",
  );
});
