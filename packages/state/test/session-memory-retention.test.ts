import { compactMessagesForCache, SessionSnapshotCache } from "../src/session/sessionCache.ts";
import {
  MAX_CACHED_SESSION_MESSAGES,
  MAX_LIVE_MESSAGE_CONTENT_LENGTH,
  MAX_LIVE_TOOL_OUTPUT_LENGTH,
} from "../src/session/constants.ts";
import type { ChatMessage } from "../src/session/types.ts";

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

Deno.test("compactMessagesForCache strips base64 and truncates heavy payloads", () => {
  const huge = "x".repeat(MAX_LIVE_MESSAGE_CONTENT_LENGTH + 5_000);
  const toolOut = "y".repeat(MAX_LIVE_TOOL_OUTPUT_LENGTH + 2_000);
  const input: ChatMessage[] = [
    {
      id: "u1",
      role: "user",
      content: "hi",
      attachments: [
        {
          type: "image",
          name: "shot.jpg",
          mimeType: "image/jpeg",
          base64: "a".repeat(20_000),
          uri: "file://shot.jpg",
        },
      ],
    },
    {
      id: "a1",
      role: "assistant",
      content: huge,
      thinking: "t".repeat(50_000),
      toolCalls: [
        {
          id: "tool-1",
          name: "Bash",
          status: "success",
          output: toolOut,
        },
      ],
    },
  ];

  const compact = compactMessagesForCache(input);
  assertEquals(compact.length, 2, "keeps both messages");
  assert(!compact[0]?.attachments?.[0]?.base64, "base64 must be stripped from cache");
  assertEquals(compact[0]?.attachments?.[0]?.uri, "file://shot.jpg", "uri retained");
  assert(
    (compact[1]?.content.length ?? 0) <= MAX_LIVE_MESSAGE_CONTENT_LENGTH + 40,
    "assistant content must be truncated for cache",
  );
  assert(
    (compact[1]?.toolCalls?.[0]?.output?.length ?? 0) <= MAX_LIVE_TOOL_OUTPUT_LENGTH + 40,
    "tool output must be truncated for cache",
  );
});

Deno.test("SessionSnapshotCache hard-caps entries and stores compacted messages", () => {
  const cache = new SessionSnapshotCache(3);
  for (let i = 0; i < 6; i += 1) {
    cache.set({
      sessionId: `s${i}`,
      sessionType: "chat",
      title: `Session ${i}`,
      mode: "build",
      permissionMode: "autonomous",
      model: null,
      modelKey: null,
      tokenCount: 0,
      messages: [
        {
          id: `m${i}`,
          role: "user",
          content: "hello",
          attachments: [
            {
              type: "image",
              name: "x.png",
              mimeType: "image/png",
              base64: "bbbb",
            },
          ],
        },
      ],
      projectDir: null,
      workingDir: null,
      workspaceMode: null,
      targetBranch: null,
      serverState: null,
      updatedAt: Date.now(),
    });
  }

  assertEquals(cache.get("s0"), null, "oldest entries are trimmed");
  assertEquals(cache.get("s1"), null, "oldest entries are trimmed");
  assertEquals(cache.get("s2"), null, "oldest entries are trimmed");
  const hot = cache.get("s5");
  assert(hot, "newest entry remains");
  assert(!hot?.messages[0]?.attachments?.[0]?.base64, "cached snapshot is compacted");
});

Deno.test("compactMessagesForCache keeps only the newest message window", () => {
  const many: ChatMessage[] = Array.from({ length: MAX_CACHED_SESSION_MESSAGES + 25 }, (_, i) => ({
    id: `m${i}`,
    role: i % 2 === 0 ? "user" : "assistant",
    content: `message ${i}`,
  }));
  const compact = compactMessagesForCache(many);
  assertEquals(compact.length, MAX_CACHED_SESSION_MESSAGES, "message window is capped");
  assertEquals(compact[0]?.id, `m${25}`, "keeps the newest window start");
  assertEquals(
    compact[compact.length - 1]?.id,
    `m${MAX_CACHED_SESSION_MESSAGES + 24}`,
    "keeps the newest message",
  );
});

Deno.test("SessionSnapshotCache reuses compacted messages for an unchanged source revision", () => {
  const cache = new SessionSnapshotCache();
  const messages: ChatMessage[] = [
    { id: "u1", role: "user", content: "hello" },
    { id: "a1", role: "assistant", content: "world" },
  ];
  const snapshot = {
    sessionId: "stable",
    sessionType: "chat" as const,
    title: "Stable",
    mode: "build" as const,
    permissionMode: "autonomous" as const,
    model: null,
    modelKey: null,
    tokenCount: 0,
    messages,
    projectDir: null,
    workingDir: null,
    workspaceMode: null,
    targetBranch: null,
    serverState: null,
    updatedAt: 1,
  };

  cache.set(snapshot);
  const first = cache.get("stable");
  assert(first, "first compact snapshot exists");
  cache.set({ ...snapshot, updatedAt: 2 });
  const second = cache.get("stable");
  assert(second, "second compact snapshot exists");
  assertEquals(
    second.messages,
    first.messages,
    "unchanged message source should not be deep-compacted again",
  );

  cache.set({ ...snapshot, messages: first.messages, updatedAt: 3 });
  const third = cache.get("stable");
  assert(third, "cached-source snapshot exists");
  assertEquals(
    third.messages,
    first.messages,
    "reopening and leaving a cached shell should retain compact message identity",
  );
});
