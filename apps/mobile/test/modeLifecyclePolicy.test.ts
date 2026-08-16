import { resolveModeLifecyclePolicy } from "../components/chat-screen/modeLifecyclePolicy";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("hidden idle modes release presence and polling", () => {
  const policy = resolveModeLifecyclePolicy("chat", "code", false);
  assert(!policy.keepPresence, "hidden mode should not advertise a viewer");
  assert(!policy.keepPolling, "hidden idle mode should not poll");
});

Deno.test("hidden streaming modes retain recovery without viewer presence", () => {
  const policy = resolveModeLifecyclePolicy("chat", "code", true);
  assert(!policy.keepPresence, "hidden stream should not advertise a viewer");
  assert(policy.keepPolling, "hidden stream should retain recovery polling");
});

Deno.test("active mode owns viewer presence", () => {
  const policy = resolveModeLifecyclePolicy("code", "code", false);
  assert(policy.keepPresence, "active mode should own viewer presence");
});
