import { presentTool } from "../components/chat/toolPresentation";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`);
  }
}

Deno.test("agent presentation prefers parent name and exact capabilities", () => {
  const presentation = presentTool({
    id: "tool-1",
    name: "agent",
    arguments: {
      name: "focused validator",
      instructions: "Run the focused checks",
      capabilities: ["execute"],
    },
    status: "running",
  });

  assertEquals(presentation.family, "delegated", "new Agent API must use delegated UI");
  assertEquals(presentation.label, "focused validator", "parent name must be visible");
  assertEquals(presentation.summary, "Run the focused checks", "instructions must be visible");
  assertEquals(presentation.meta, "execute", "execute-only must remain distinct");
});

Deno.test("agent presentation retains legacy agent_type fallback", () => {
  const presentation = presentTool({
    id: "tool-legacy",
    name: "agent",
    arguments: { agent_type: "verify", prompt: "check" },
    status: "running",
  });

  assertEquals(presentation.label, "verify", "legacy label must remain compatible");
});
