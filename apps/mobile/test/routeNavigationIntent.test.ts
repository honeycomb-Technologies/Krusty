// @ts-ignore Deno requires an explicit extension; the Expo compiler resolves
// the same source through its extensionless application imports.
import { resolveRouteNavigationIntent } from "../components/navigation/routeNavigationIntent.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals<T>(actual: T, expected: T, message: string): void {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

Deno.test("route navigation intent normalizes array parameters", () => {
  const intent = resolveRouteNavigationIntent({
    sessionId: ["session-1", "ignored"],
    focus: ["code"],
  });

  assert(intent, "a deep link must resolve to an intent");
  assertEquals(intent.params.sessionId, "session-1", "first session id wins");
  assertEquals(intent.params.focus, "code", "first focus wins");
});

Deno.test("route navigation key changes only when route intent changes", () => {
  const first = resolveRouteNavigationIntent({
    sessionId: "session-1",
    focus: "code",
  });
  const same = resolveRouteNavigationIntent({
    sessionId: ["session-1"],
    focus: ["code"],
  });
  const next = resolveRouteNavigationIntent({
    sessionId: "session-2",
    focus: "chat",
  });

  assert(first && same && next, "all supplied routes must resolve");
  assertEquals(first.key, same.key, "equivalent route forms share one key");
  assert(first.key !== next.key, "a new route must produce a new key");
});

Deno.test("empty route has no navigation intent", () => {
  assertEquals(
    resolveRouteNavigationIntent({}),
    null,
    "ordinary in-app navigation must not create a route intent",
  );
});
