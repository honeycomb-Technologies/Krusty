import { getTerminalHtml } from "../components/toolbox/terminalHtml";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("native terminal HTML bounds output before writing to xterm", () => {
  const html = getTerminalHtml("wss://example.test/ws/terminal", {
    background: "#000",
    foreground: "#fff",
    cursor: "#0ff",
  });

  assert(html.includes("OUTPUT_HIGH_WATERMARK = 512 * 1024"), "missing high watermark");
  assert(html.includes("OUTPUT_LOW_WATERMARK = 128 * 1024"), "missing low watermark");
  assert(html.includes("enqueueOutput(message.data)"), "provider output must enter the bounded queue");
  assert(!html.includes("term.write(message.data)"), "provider output must not bypass backpressure");
  assert(html.includes("ws.close(1008, 'terminal output buffer exceeded')"), "overflow must close explicitly");
});

Deno.test("native terminal HTML drains xterm writes serially", () => {
  const html = getTerminalHtml("wss://example.test/ws/terminal", {
    background: "#000",
    foreground: "#fff",
    cursor: "#0ff",
  });

  assert(html.includes("if (outputWriteActive || pendingOutput.length === 0) return"), "drain should be single-flight");
  assert(html.includes("term.write(chunk, function()"), "the next chunk must wait for xterm's callback");
});
