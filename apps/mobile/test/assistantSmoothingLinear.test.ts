import {
  appendContinuationText,
  shouldMergeContinuationText,
  smoothInterruptedText,
  type AssistantVisualSegment,
} from "../components/chat/assistantTextSmoothing";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function tool(id: string): AssistantVisualSegment {
  return {
    type: "exploration",
    id,
    tools: [],
  };
}

Deno.test("assistant smoothing preserves soft interruption order in one pass", () => {
  const input: AssistantVisualSegment[] = [
    { type: "text", id: "t1", content: "we only" },
    tool("tool-1"),
    { type: "text", id: "t2", content: "skimmed." },
    {
      type: "thinking",
      id: "thinking-1",
      content: "boundary",
    },
    { type: "text", id: "t3", content: "New block." },
  ];

  const output = smoothInterruptedText(input);
  assert(output.length === 4, "one continuation should merge");
  assert(output[0]?.type === "text", "first item remains text");
  assert(
    output[0]?.type === "text" && output[0].content === "we only skimmed.",
    "merged prose must stay byte-exact",
  );
  assert(output[1]?.id === "tool-1", "tool slot order must remain stable");
  assert(output[2]?.type === "thinking", "hard boundaries must remain stable");
  assert(output[3]?.id === "t3", "text after a hard boundary must not merge");
});

Deno.test("assistant smoothing stays bounded for tool-heavy committed turns", () => {
  const input: AssistantVisualSegment[] = [
    { type: "text", id: "text-start", content: "fragment" },
  ];
  for (let index = 0; index < 2_000; index += 1) {
    input.push(tool(`tool-${index}`));
    input.push({ type: "text", id: `text-${index}`, content: `part${index}` });
  }

  const startedAt = performance.now();
  const output = smoothInterruptedText(input);
  const durationMs = performance.now() - startedAt;
  assert(output.length === 2_001, "every continuation must merge into one text slot");
  assert(durationMs < 50, `linear smoothing took ${durationMs.toFixed(1)}ms`);
});

function referenceSmooth(
  segments: AssistantVisualSegment[],
): AssistantVisualSegment[] {
  const smoothed: AssistantVisualSegment[] = [];
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
      previousText?.type === "text"
      && shouldMergeContinuationText(
        previousText.content,
        segment.content,
        intervening,
      )
    ) {
      smoothed[previousTextIndex] = {
        ...previousText,
        content: appendContinuationText(previousText.content, segment.content),
      };
    } else {
      smoothed.push(segment);
    }
  }
  return smoothed;
}

Deno.test("linear smoothing remains byte-equivalent to the reference policy", () => {
  let seed = 0x5eed1234;
  const random = () => {
    seed = (Math.imul(seed, 1_664_525) + 1_013_904_223) >>> 0;
    return seed / 0x1_0000_0000;
  };
  const texts = [
    "we only",
    "skimmed.",
    "New block.",
    ", then continued",
    "fragment ",
    "## Heading",
    "```",
  ];

  for (let sample = 0; sample < 100; sample += 1) {
    const segments: AssistantVisualSegment[] = [];
    for (let index = 0; index < 80; index += 1) {
      const roll = random();
      if (roll < 0.45) {
        segments.push({
          type: "text",
          id: `sample-${sample}-text-${index}`,
          content: texts[Math.floor(random() * texts.length)] ?? "text",
        });
      } else if (roll < 0.8) {
        segments.push(tool(`sample-${sample}-tool-${index}`));
      } else {
        segments.push({
          type: "thinking",
          id: `sample-${sample}-thinking-${index}`,
          content: "boundary",
        });
      }
    }
    const expected = JSON.stringify(referenceSmooth(segments));
    const actual = JSON.stringify(smoothInterruptedText(segments));
    assert(actual === expected, `sample ${sample} changed smoothing output`);
  }
});
