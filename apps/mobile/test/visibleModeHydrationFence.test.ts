import { isCurrentVisibleModeHydrationIntent } from "../components/chat-screen/visibleModeHydrationFence.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("visible-mode hydration timer yields to the latest Worker session intent", () => {
  assert(
    isCurrentVisibleModeHydrationIntent(
      "worker-a",
      "worker-a",
      "worker-a",
      "worker-a",
    ),
    "an unchanged loading shell should remain eligible for hydration",
  );
  assert(
    isCurrentVisibleModeHydrationIntent(
      "worker-b",
      "worker-a",
      "worker-b",
      "worker-a",
    ),
    "a remembered destination may replace the old store shell",
  );
  assert(
    !isCurrentVisibleModeHydrationIntent(
      "worker-a",
      "worker-a",
      "worker-b",
      "worker-b",
    ),
    "a newer remembered Worker B selection must cancel Worker A hydration",
  );
  assert(
    !isCurrentVisibleModeHydrationIntent(
      "worker-a",
      "worker-a",
      "worker-a",
      "worker-c",
    ),
    "a changed store target must cancel hydration even if a stale remembered ref remains",
  );
});
