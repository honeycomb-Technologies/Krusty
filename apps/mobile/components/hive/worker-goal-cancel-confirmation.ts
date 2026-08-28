export const WORKER_GOAL_CANCEL_TITLE = "Cancel Worker Goal?";
export const WORKER_GOAL_CANCEL_MESSAGE =
  "This stops its Worker runs and cancels the active plan.";

export interface WorkerGoalCancelAlertButton {
  text: string;
  style: "cancel" | "destructive";
  onPress?: () => void;
}

interface ConfirmWorkerGoalCancellationOptions {
  isWeb: boolean;
  confirmWeb?: (message: string) => boolean;
  showNativeAlert: (
    title: string,
    message: string,
    buttons: WorkerGoalCancelAlertButton[],
  ) => void;
  onConfirm: () => void;
}

export interface WorkerGoalCancellationContext {
  targetKey: string | null;
  canCancel: boolean;
  cancel: () => Promise<void>;
}

export interface WorkerGoalCancellationGuard {
  attempt: (
    requestedTargetKey: string | null,
    getCurrentContext: () => WorkerGoalCancellationContext,
  ) => boolean;
}

/**
 * Keeps one destructive cancellation attempt per Goal target. A rejected
 * attempt may be retried while that same target still exposes Cancel; a
 * successful attempt stays latched for that target.
 */
export function createWorkerGoalCancellationGuard(): WorkerGoalCancellationGuard {
  let attemptGeneration = 0;
  const attempts = new Map<
    string,
    { phase: "in_flight" | "succeeded"; generation: number }
  >();

  return {
    attempt(requestedTargetKey, getCurrentContext) {
      if (!requestedTargetKey) return false;

      const current = getCurrentContext();
      if (
        current.targetKey !== requestedTargetKey ||
        !current.canCancel
      ) {
        return false;
      }

      if (attempts.has(requestedTargetKey)) return false;

      const generation = ++attemptGeneration;
      attempts.set(requestedTargetKey, { phase: "in_flight", generation });
      let attempt: Promise<void>;
      try {
        attempt = Promise.resolve(current.cancel());
      } catch {
        if (attempts.get(requestedTargetKey)?.generation === generation) {
          attempts.delete(requestedTargetKey);
        }
        return true;
      }

      void attempt.then(
        () => {
          if (attempts.get(requestedTargetKey)?.generation === generation) {
            attempts.set(requestedTargetKey, {
              phase: "succeeded",
              generation,
            });
          }
        },
        () => {
          if (attempts.get(requestedTargetKey)?.generation === generation) {
            attempts.delete(requestedTargetKey);
          }
        },
      );
      return true;
    },
  };
}

export function confirmWorkerGoalCancellation({
  isWeb,
  confirmWeb,
  showNativeAlert,
  onConfirm,
}: ConfirmWorkerGoalCancellationOptions): void {
  if (isWeb) {
    if (
      confirmWeb?.(
        `${WORKER_GOAL_CANCEL_TITLE}\n\n${WORKER_GOAL_CANCEL_MESSAGE}`,
      )
    ) {
      onConfirm();
    }
    return;
  }

  showNativeAlert(WORKER_GOAL_CANCEL_TITLE, WORKER_GOAL_CANCEL_MESSAGE, [
    { text: "Keep Goal", style: "cancel" },
    {
      text: "Cancel Goal",
      style: "destructive",
      onPress: onConfirm,
    },
  ]);
}
