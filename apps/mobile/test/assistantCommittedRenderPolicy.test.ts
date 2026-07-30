import { shouldSegmentAssistantContent } from "../components/chat/assistantSegments";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown): void {
  if (actual !== expected) {
    throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
  }
}

Deno.test("committed assistant content uses one renderer subtree", () => {
  assertEquals(shouldSegmentAssistantContent(false), false);
});

Deno.test("only a live streaming tail keeps stable block segmentation", () => {
  assertEquals(shouldSegmentAssistantContent(true), true);
});
