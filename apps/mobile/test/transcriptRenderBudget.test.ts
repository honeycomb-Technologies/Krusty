import { summarizeTranscriptRenderBudget } from "../components/chat/transcriptRenderBudget";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown): void {
  if (actual !== expected) {
    throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
  }
}

Deno.test("visible transcript telemetry keeps only bounded numeric shape", () => {
  const summary = summarizeTranscriptRenderBudget([
    {
      content: "hello",
      renderParts: [{ type: "text" }],
      toolCalls: [],
    },
    {
      content: "result",
      renderParts: [{ type: "tool" }, { type: "text" }],
      toolCalls: [{ id: "tool-1" }],
    },
  ]);

  assertEquals(summary.messageCount, 2);
  assertEquals(summary.renderPartCount, 3);
  assertEquals(summary.toolCount, 1);
  assertEquals(summary.markdownCharacterCount, 11);
  assertEquals("content" in summary, false);
});
