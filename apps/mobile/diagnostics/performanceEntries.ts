export interface PerformanceEntryTiming {
  duration?: number;
}

export interface DelayedInteractionBatch {
  count: number;
  maximumDurationMs: number;
}

export function summarizeDelayedInteractions(
  entries: PerformanceEntryTiming[],
  thresholdMs: number,
): DelayedInteractionBatch | null {
  let count = 0;
  let maximumDurationMs = 0;
  for (const entry of entries) {
    const duration = Number(entry.duration ?? 0);
    if (!Number.isFinite(duration) || duration < thresholdMs) continue;
    count += 1;
    maximumDurationMs = Math.max(maximumDurationMs, duration);
  }
  return count > 0 ? { count, maximumDurationMs } : null;
}
