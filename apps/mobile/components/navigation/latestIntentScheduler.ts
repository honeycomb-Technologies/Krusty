export interface LatestIntentSchedulerOptions<T, TimerHandle = ReturnType<typeof setTimeout>> {
  quietDelayMs: number;
  maxDelayMs: number;
  onFlush(intent: T): void;
  now?: () => number;
  setTimer?: (callback: () => void, delayMs: number) => TimerHandle;
  clearTimer?: (handle: TimerHandle) => void;
}

export interface LatestIntentScheduler<T> {
  /** Replace any pending intent and schedule the latest value for admission. */
  submit(intent: T): void;
  /** Immediately admit the latest pending intent, if one exists. */
  flush(): boolean;
  /** Drop the pending intent without admitting it. */
  cancel(): boolean;
  hasPending(): boolean;
}

/**
 * Coalesces bursty UI intents without allowing continuous input to postpone work
 * forever. The quiet deadline follows the latest submission, while the hard
 * deadline remains anchored to the first submission in the current burst.
 */
export function createLatestIntentScheduler<
  T,
  TimerHandle = ReturnType<typeof setTimeout>,
>(
  options: LatestIntentSchedulerOptions<T, TimerHandle>,
): LatestIntentScheduler<T> {
  const { quietDelayMs, maxDelayMs, onFlush } = options;
  if (!Number.isFinite(quietDelayMs) || quietDelayMs < 0) {
    throw new RangeError("quietDelayMs must be a finite non-negative number");
  }
  if (!Number.isFinite(maxDelayMs) || maxDelayMs < 0) {
    throw new RangeError("maxDelayMs must be a finite non-negative number");
  }

  const now = options.now ?? Date.now;
  const setTimer = options.setTimer ?? ((callback, delayMs) =>
    setTimeout(callback, delayMs) as TimerHandle);
  const clearTimer = options.clearTimer ?? ((handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>));

  let pending = false;
  let latestIntent: T;
  let hardDeadlineMs = 0;
  let quietDeadlineMs = 0;
  let timer: TimerHandle | null = null;
  let timerGeneration = 0;

  const clearScheduledTimer = () => {
    timerGeneration += 1;
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
  };

  const flush = (): boolean => {
    if (!pending) return false;

    const intent = latestIntent;
    pending = false;
    clearScheduledTimer();
    onFlush(intent);
    return true;
  };

  const schedulePending = () => {
    clearScheduledTimer();
    const dueAtMs = Math.min(quietDeadlineMs, hardDeadlineMs);
    const generation = timerGeneration;
    timer = setTimer(() => {
      if (generation !== timerGeneration || !pending) return;
      timer = null;

      // Tolerate timer implementations that invoke slightly before their due time.
      if (now() < dueAtMs) {
        schedulePending();
        return;
      }
      flush();
    }, Math.max(0, dueAtMs - now()));
  };

  return {
    submit(intent) {
      const submittedAtMs = now();
      latestIntent = intent;
      quietDeadlineMs = submittedAtMs + quietDelayMs;
      if (!pending) {
        pending = true;
        hardDeadlineMs = submittedAtMs + maxDelayMs;
      }
      schedulePending();
    },
    flush,
    cancel() {
      if (!pending) return false;
      pending = false;
      clearScheduledTimer();
      return true;
    },
    hasPending() {
      return pending;
    },
  };
}
