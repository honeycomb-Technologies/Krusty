import { canRenderAsPlainChatText } from "../components/chat/markdownFastPath";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown): void {
  if (actual !== expected) {
    throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
  }
}

Deno.test("plain chat prose skips the Markdown renderer", () => {
  assertEquals(
    canRenderAsPlainChatText("The app stayed responsive through every switch."),
    true,
  );
  assertEquals(
    canRenderAsPlainChatText("A long paragraph can contain punctuation, (parentheses), and 2 + 2."),
    true,
  );
});

Deno.test("formatted chat content keeps the rich Markdown renderer", () => {
  for (const content of [
    "## Result",
    "- first item",
    "1. first item",
    "**important**",
    "*emphasis*",
    "_emphasis_",
    "`inline code`",
    "[Open report](https://example.com)",
    "https://example.com",
    "```ts\nconst ready = true;\n```",
    "| State | Result |\n| --- | --- |",
    "---",
    "Setext heading\n===",
  ]) {
    assertEquals(canRenderAsPlainChatText(content), false);
  }
});
