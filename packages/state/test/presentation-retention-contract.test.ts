import {
  compactHistoricalMessage,
  isTurnInRichWindow,
  RICH_RECENT_TURN_COUNT,
} from "../../../apps/mobile/components/chat/presentationRetention.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`);
  }
}

Deno.test("only recent turns stay in the rich window", () => {
  assertEquals(isTurnInRichWindow(0, 10), false, "old turn is compact");
  assertEquals(isTurnInRichWindow(9, 10), true, "latest turn is rich");
  assertEquals(
    isTurnInRichWindow(10 - RICH_RECENT_TURN_COUNT, 10),
    true,
    "rich window includes configured recent turns",
  );
});

Deno.test("historical assistant messages collapse heavy tool detail", () => {
  const compact = compactHistoricalMessage({
    id: "a1",
    role: "assistant",
    content: "done",
    thinking: "t".repeat(5_000),
    toolCalls: [
      {
        id: "tool-1",
        name: "Bash",
        status: "success",
        output: "o".repeat(5_000),
      },
    ],
    attachments: [
      {
        type: "image",
        name: "x.png",
        mimeType: "image/png",
        base64: "abc",
        uri: "file://x.png",
      },
    ],
  });

  assert((compact.thinking?.length ?? 0) < 2_000, "thinking collapsed");
  assert((compact.toolCalls?.[0]?.output?.length ?? 0) < 2_000, "tool output collapsed");
  assert(!compact.attachments?.[0]?.base64, "historical base64 stripped");
  assertEquals(compact.attachments?.[0]?.uri, "file://x.png", "uri retained");
});
