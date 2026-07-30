import { createStreamEffectSelector } from "../components/chat/streamEffectsModel";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("text deltas preserve stream side-effect selector identity", () => {
  const select = createStreamEffectSelector();
  const toolCall = { id: "tool-1", name: "bash", status: "running" as const };
  const base = {
    sessionId: "session-1",
    title: "Chat",
    isStreaming: true,
    tokenCount: 10,
    model: "gpt-test",
  };
  const first = select({
    ...base,
    messages: [
      { id: "user-1", role: "user", content: "hello" },
      {
        id: "assistant-1",
        role: "assistant",
        content: "one",
        toolCalls: [toolCall],
      },
    ],
  });
  const second = select({
    ...base,
    messages: [
      { id: "user-1", role: "user", content: "hello" },
      {
        id: "assistant-1",
        role: "assistant",
        content: "one two",
        toolCalls: [toolCall],
      },
    ],
  });

  assert(
    first.currentTurnToolCalls === second.currentTurnToolCalls,
    "unchanged tool references should remain selector-stable",
  );
  assert(
    first.settledAssistantSnippet === "" && second.settledAssistantSnippet === "",
    "live token text should not enter native side-effect state",
  );
  assert(
    Object.keys(first).every(
      (key) =>
        first[key as keyof typeof first] === second[key as keyof typeof second],
    ),
    "every shallow-selected field should remain equal for text-only deltas",
  );
});

Deno.test("completion publishes the settled assistant snippet once", () => {
  const select = createStreamEffectSelector();
  const completed = select({
    sessionId: "session-1",
    title: "Chat",
    isStreaming: false,
    tokenCount: 20,
    model: "gpt-test",
    messages: [
      { id: "user-1", role: "user", content: "hello" },
      { id: "assistant-1", role: "assistant", content: "final response" },
    ],
  });

  assert(
    completed.settledAssistantSnippet === "final response",
    "settled content should become available to widgets after completion",
  );
});

Deno.test("queued steering does not hide a pending approval", () => {
  const select = createStreamEffectSelector();
  const approval = {
    id: "tool-approval",
    name: "bash",
    status: "awaiting_approval" as const,
  };
  const view = select({
    sessionId: "session-1",
    title: "Chat",
    isStreaming: true,
    tokenCount: 10,
    model: "gpt-test",
    messages: [
      { id: "user-1", role: "user", content: "run it" },
      {
        id: "assistant-1",
        role: "assistant",
        content: "",
        toolCalls: [approval],
      },
      {
        id: "user-queued",
        role: "user",
        content: "and explain it",
        isQueued: true,
      },
    ],
  });

  assert(
    view.currentTurnToolCalls[0] === approval,
    "queued steering must not become a tool-activity boundary",
  );
});
