import { createSessionCreationCoordinator } from "../app/(tabs)/chat-screen/sessionCreationCoordinator";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${JSON.stringify(actual)}`,
    );
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

Deno.test("immediate New then Send shares one per-mode creation", async () => {
  const coordinator = createSessionCreationCoordinator<string | null>();
  const response = deferred<string>();
  let creates = 0;
  let binds = 0;
  const createAndBind = (isCurrent: () => boolean) => {
    creates += 1;
    return response.promise.then((id) => {
      if (!isCurrent()) return null;
      binds += 1;
      return id;
    });
  };

  const fromNew = coordinator.run("chat", createAndBind);
  const fromSend = coordinator.run("chat", createAndBind);
  response.resolve("session-1");

  assertEquals(await fromNew, "session-1", "New should receive the durable session");
  assertEquals(await fromSend, "session-1", "Send should await the same durable session");
  assertEquals(creates, 1, "New and Send must issue one create request");
  assertEquals(binds, 1, "the durable session must bind once");
});

Deno.test("double New shares one per-mode creation", async () => {
  const coordinator = createSessionCreationCoordinator<string | null>();
  const response = deferred<string>();
  let creates = 0;
  const task = async (isCurrent: () => boolean) => {
    creates += 1;
    const id = await response.promise;
    return isCurrent() ? id : null;
  };

  const first = coordinator.run("code", task);
  const second = coordinator.run("code", task);
  response.resolve("session-code");

  assertEquals(await Promise.all([first, second]), ["session-code", "session-code"], "both callers share the result");
  assertEquals(creates, 1, "double New must issue one create request");
});

Deno.test("invalidated creation cannot bind over an explicit selection", async () => {
  const coordinator = createSessionCreationCoordinator<string | null>();
  const response = deferred<string>();
  let binds = 0;
  const pending = coordinator.run("chat", async (isCurrent) => {
    const id = await response.promise;
    if (!isCurrent()) return null;
    binds += 1;
    return id;
  });

  coordinator.invalidate("chat");
  response.resolve("late-session");

  assertEquals(await pending, null, "late creation should be discarded");
  assertEquals(binds, 0, "late creation must not bind");
});

Deno.test("a new intent does not reuse an invalidated creation", async () => {
  const coordinator = createSessionCreationCoordinator<string | null>();
  const staleResponse = deferred<string>();
  let creates = 0;
  const stale = coordinator.run("chat", async (isCurrent) => {
    creates += 1;
    const id = await staleResponse.promise;
    return isCurrent() ? id : null;
  });

  coordinator.invalidate("chat");
  const current = coordinator.run("chat", async (isCurrent) => {
    creates += 1;
    return isCurrent() ? "current-session" : null;
  });
  staleResponse.resolve("stale-session");

  assertEquals(await stale, null, "invalidated request remains unable to bind");
  assertEquals(await current, "current-session", "new intent owns a fresh creation");
  assertEquals(creates, 2, "new intent must not await the invalidated request");
});
