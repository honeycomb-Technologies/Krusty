export interface LatestIntentSchedulerOptions<T, TimerHandle = ReturnType<typeof setTimeout>> {
  quietDelayMs: number;
  /**
   * Optional hard deadline for work that must make progress during continuous
   * input. Omit it for heavy UI work: the visible intent can update
   * immediately while the expensive destination is admitted only after input
   * becomes quiet.
   */
  maxDelayMs?: number;
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
 * Coalesces bursty UI intents. The quiet deadline follows the latest
 * submission. Callers may opt into a hard deadline for lightweight work, but
 * heavy navigation should remain quiet-only so sustained input cannot enqueue
 * repeated surface mounts.
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
  if (
    maxDelayMs !== undefined
    && (!Number.isFinite(maxDelayMs) || maxDelayMs < 0)
  ) {
    throw new RangeError("maxDelayMs must be a finite non-negative number");
  }

  const now = options.now ?? Date.now;
  const setTimer = options.setTimer ?? ((callback, delayMs) =>
    setTimeout(callback, delayMs) as TimerHandle);
  const clearTimer = options.clearTimer ?? ((handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>));

  let pending = false;
  let latestIntent: T;
  let hardDeadlineMs: number | null = null;
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
    const dueAtMs = hardDeadlineMs === null
      ? quietDeadlineMs
      : Math.min(quietDeadlineMs, hardDeadlineMs);
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
        hardDeadlineMs = maxDelayMs === undefined
          ? null
          : submittedAtMs + maxDelayMs;
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
