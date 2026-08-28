import {
  confirmWorkerGoalCancellation,
  createWorkerGoalCancellationGuard,
  type WorkerGoalCancelAlertButton,
  type WorkerGoalCancellationContext,
} from "../components/hive/worker-goal-cancel-confirmation.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushPromiseHandlers(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

Deno.test("web Goal cancellation does nothing when confirmation is declined", () => {
  let cancelCalls = 0;
  let confirmCalls = 0;
  let nativeAlertCalls = 0;

  confirmWorkerGoalCancellation({
    isWeb: true,
    confirmWeb: (message) => {
      confirmCalls += 1;
      assert(
        message ===
          "Cancel Worker Goal?\n\nThis stops its Worker runs and cancels the active plan.",
        "web confirmation must preserve the destructive-action copy",
      );
      return false;
    },
    showNativeAlert: () => {
      nativeAlertCalls += 1;
    },
    onConfirm: () => {
      cancelCalls += 1;
    },
  });

  assert(confirmCalls === 1, "web must ask for confirmation exactly once");
  assert(cancelCalls === 0, "declining confirmation must not cancel the Goal");
  assert(nativeAlertCalls === 0, "web must not use react-native Alert");
});

Deno.test("web Goal cancellation calls cancel exactly once when confirmed", () => {
  let cancelCalls = 0;

  confirmWorkerGoalCancellation({
    isWeb: true,
    confirmWeb: () => true,
    showNativeAlert: () => {
      throw new Error("web must not use react-native Alert");
    },
    onConfirm: () => {
      cancelCalls += 1;
    },
  });

  assert(cancelCalls === 1, "confirmed web cancellation must run exactly once");
});

Deno.test("web Goal cancellation is inert during SSR without window", () => {
  let cancelCalls = 0;
  let nativeAlertCalls = 0;

  confirmWorkerGoalCancellation({
    isWeb: true,
    confirmWeb: undefined,
    showNativeAlert: () => {
      nativeAlertCalls += 1;
    },
    onConfirm: () => {
      cancelCalls += 1;
    },
  });

  assert(cancelCalls === 0, "SSR must not cancel without browser confirmation");
  assert(nativeAlertCalls === 0, "SSR web must not fall through to Alert");
});

Deno.test("native Goal cancellation remains Alert based without browser globals", () => {
  let cancelCalls = 0;
  let alertTitle = "";
  let alertMessage = "";
  let alertButtons: WorkerGoalCancelAlertButton[] = [];

  confirmWorkerGoalCancellation({
    isWeb: false,
    showNativeAlert: (title, message, buttons) => {
      alertTitle = title;
      alertMessage = message;
      alertButtons = buttons;
    },
    onConfirm: () => {
      cancelCalls += 1;
    },
  });

  assert(
    alertTitle === "Cancel Worker Goal?",
    "native title must be unchanged",
  );
  assert(
    alertMessage === "This stops its Worker runs and cancels the active plan.",
    "native message must be unchanged",
  );
  assert(alertButtons.length === 2, "native Alert must expose both decisions");
  assert(
    alertButtons[0]?.text === "Keep Goal" &&
      alertButtons[0]?.style === "cancel",
    "native dismissal must preserve its exact label and style",
  );
  assert(
    alertButtons[1]?.text === "Cancel Goal" &&
      alertButtons[1]?.style === "destructive",
    "native confirmation must preserve its exact label and style",
  );
  assert(cancelCalls === 0, "showing native Alert must not cancel eagerly");
  alertButtons[1]?.onPress?.();
  assert(
    Number(cancelCalls) === 1,
    "native destructive action must cancel exactly once",
  );
});

Deno.test("cancellation guard collapses duplicate async activation and latches success", async () => {
  const guard = createWorkerGoalCancellationGuard();
  const pending = deferred();
  let cancelCalls = 0;
  const context: WorkerGoalCancellationContext = {
    targetKey: "worker-1:goal-1",
    canCancel: true,
    cancel: () => {
      cancelCalls += 1;
      return pending.promise;
    },
  };

  assert(
    guard.attempt(context.targetKey, () => context),
    "the first confirmed attempt must start",
  );
  assert(
    !guard.attempt(context.targetKey, () => context),
    "a duplicate in-flight activation must be ignored",
  );
  assert(
    cancelCalls === 1,
    "duplicate activation must call cancel exactly once",
  );

  pending.resolve();
  await flushPromiseHandlers();
  assert(
    !guard.attempt(context.targetKey, () => context),
    "a successful cancellation must stay latched for the same Goal",
  );
  assert(cancelCalls === 1, "the success latch must prevent a second call");
});

Deno.test("repeated native destructive callbacks share the one-shot guard", async () => {
  const guard = createWorkerGoalCancellationGuard();
  const pending = deferred();
  let cancelCalls = 0;
  const context: WorkerGoalCancellationContext = {
    targetKey: "worker-1:goal-1",
    canCancel: true,
    cancel: () => {
      cancelCalls += 1;
      return pending.promise;
    },
  };
  const requestedTargetKey = context.targetKey;
  let alertButtons: WorkerGoalCancelAlertButton[] = [];

  confirmWorkerGoalCancellation({
    isWeb: false,
    showNativeAlert: (_title, _message, buttons) => {
      alertButtons = buttons;
    },
    onConfirm: () => {
      guard.attempt(requestedTargetKey, () => context);
    },
  });

  alertButtons[1]?.onPress?.();
  alertButtons[1]?.onPress?.();
  assert(
    cancelCalls === 1,
    "duplicate native callbacks must collapse in flight",
  );

  pending.resolve();
  await flushPromiseHandlers();
  alertButtons[1]?.onPress?.();
  assert(
    cancelCalls === 1,
    "duplicate native callbacks must remain latched after success",
  );
});

Deno.test("cancellation guard permits a rejected attempt retry only while still allowed", async () => {
  const guard = createWorkerGoalCancellationGuard();
  const rejected = deferred();
  let cancelCalls = 0;
  const context: WorkerGoalCancellationContext = {
    targetKey: "worker-1:goal-1",
    canCancel: true,
    cancel: () => {
      cancelCalls += 1;
      return cancelCalls === 1 ? rejected.promise : Promise.resolve();
    },
  };

  assert(guard.attempt(context.targetKey, () => context), "attempt must start");
  rejected.reject(new Error("expected cancellation rejection"));
  await flushPromiseHandlers();

  context.canCancel = false;
  assert(
    !guard.attempt(context.targetKey, () => context),
    "a rejected attempt must not retry after Cancel disappears",
  );
  assert(cancelCalls === 1, "a disallowed retry must not call cancel");

  context.canCancel = true;
  assert(
    guard.attempt(context.targetKey, () => context),
    "the same allowed Goal may retry after rejection",
  );
  await flushPromiseHandlers();
  assert(
    Number(cancelCalls) === 2,
    "an allowed rejected attempt must retry once",
  );
});

Deno.test("delayed native confirmation cannot cancel a stale Goal target", async () => {
  const guard = createWorkerGoalCancellationGuard();
  let oldCancelCalls = 0;
  let currentCancelCalls = 0;
  const context: WorkerGoalCancellationContext = {
    targetKey: "worker-1:goal-1",
    canCancel: true,
    cancel: () => {
      oldCancelCalls += 1;
      return Promise.resolve();
    },
  };
  const requestedTargetKey = context.targetKey;
  let alertButtons: WorkerGoalCancelAlertButton[] = [];

  confirmWorkerGoalCancellation({
    isWeb: false,
    showNativeAlert: (_title, _message, buttons) => {
      alertButtons = buttons;
    },
    onConfirm: () => {
      guard.attempt(requestedTargetKey, () => context);
    },
  });

  context.targetKey = "worker-2:goal-2";
  context.cancel = () => {
    currentCancelCalls += 1;
    return Promise.resolve();
  };
  alertButtons[1]?.onPress?.();
  alertButtons[1]?.onPress?.();
  await flushPromiseHandlers();

  assert(
    oldCancelCalls === 0,
    "the old Goal action must not survive the dialog",
  );
  assert(
    currentCancelCalls === 0,
    "the stale dialog must not cancel the newly rendered Goal",
  );
});

Deno.test("Goal tracker wires browser confirm, native Alert, and handled cancellation", async () => {
  const tracker = await Deno.readTextFile(
    new URL("../components/hive/HiveWorkerGoalTracker.tsx", import.meta.url),
  );

  assert(
    tracker.includes('isWeb: Platform.OS === "web"') &&
      tracker.includes('typeof window === "undefined"') &&
      tracker.includes("window.confirm(message)"),
    "the rendered web tracker must reach the browser confirmation boundary",
  );
  assert(
    tracker.includes("Alert.alert(alertTitle, alertMessage, buttons)"),
    "the native tracker must remain wired to react-native Alert",
  );
  const confirmHandlerStart = tracker.indexOf("const confirmCancel = () => {");
  const confirmHandlerEnd = tracker.indexOf(
    "const createReady =",
    confirmHandlerStart,
  );
  const confirmHandler = tracker.slice(confirmHandlerStart, confirmHandlerEnd);
  assert(
    confirmHandlerStart >= 0 &&
      confirmHandler.includes("confirmWorkerGoalCancellation({") &&
      confirmHandler.includes(
        "if (!cancelSurfaceMountedRef.current) return;",
      ) &&
      confirmHandler.includes("cancelGuardRef.current?.attempt("),
    "the rendered handler must reject unmounted native callbacks before the one-shot guard",
  );
  assert(
    tracker.includes("cancelSurfaceMountedRef.current = true") &&
      tracker.includes("cancelSurfaceMountedRef.current = false"),
    "a keyed Worker/Goal tracker must invalidate its delayed native confirmation on unmount",
  );

  const cancelControlStart = tracker.indexOf('{actions.has("cancel")');
  const cancelControlEnd = tracker.indexOf("</View>", cancelControlStart);
  const cancelControl = tracker.slice(cancelControlStart, cancelControlEnd);
  assert(
    cancelControlStart >= 0 &&
      cancelControl.includes("onPress={confirmCancel}"),
    "the rendered Cancel control must invoke the confirmation handler",
  );
  assert(
    !tracker.includes("state.cancel("),
    "the tracker must not bypass the guarded cancellation boundary",
  );
});
