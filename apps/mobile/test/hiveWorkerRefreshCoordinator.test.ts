import { createWorkerRosterRefreshCoordinator } from "../components/hive/hooks/workerRosterRefreshCoordinator.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${
        JSON.stringify(actual)
      }`,
    );
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

Deno.test("committed Worker mutation supersedes an older delayed roster refresh", async () => {
  const coordinator = createWorkerRosterRefreshCoordinator();
  const staleResponse = deferred<string>();
  const committedResponse = deferred<string>();
  const adopted: string[] = [];

  const staleRefresh = coordinator.run(async (isCurrent) => {
    const status = await staleResponse.promise;
    if (isCurrent()) adopted.push(status);
  });
  const postCommitRefresh = coordinator.runAfterCommit(async (isCurrent) => {
    const status = await committedResponse.promise;
    if (isCurrent()) adopted.push(status);
  });

  staleResponse.resolve("active:r1");
  await staleRefresh;
  assertEquals(
    adopted,
    [],
    "the pre-commit roster response must not overwrite committed Worker state",
  );

  committedResponse.resolve("paused:r2");
  await postCommitRefresh;
  assertEquals(
    adopted,
    ["paused:r2"],
    "the mutation must await and adopt a distinct post-commit roster generation",
  );
});

Deno.test("useHiveWorkers routes successful mutations through the post-commit fence", async () => {
  const hook = await Deno.readTextFile(
    new URL("../components/hive/hooks/useHiveWorkers.ts", import.meta.url),
  );
  assertEquals(
    hook.includes("await refreshAfterMutation();") &&
      hook.includes("runAfterCommit(task)"),
    true,
    "successful Worker mutations must not join an older ordinary refresh",
  );
});
