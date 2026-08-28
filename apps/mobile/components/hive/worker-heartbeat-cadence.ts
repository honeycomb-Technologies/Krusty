export const MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS = 4_294_967_295;

export interface ParsedWorkerHeartbeatCadence {
  value: number | null;
  error: string | null;
}

export interface WorkerHeartbeatCadenceFields {
  heartbeat_interval_secs?: number;
}

/**
 * Parses the editor's optional seconds field against the server's `u32`
 * contract. Blank is a real local state: no explicit value on create and no
 * cadence mutation on edit.
 */
export function parseWorkerHeartbeatCadence(
  input: string,
): ParsedWorkerHeartbeatCadence {
  const trimmed = input.trim();
  if (trimmed.length === 0) {
    return { value: null, error: null };
  }
  if (!/^[0-9]+$/.test(trimmed)) {
    return {
      value: null,
      error: "Enter a whole number of seconds.",
    };
  }

  const value = Number(trimmed);
  if (
    !Number.isSafeInteger(value) ||
    value < 1 ||
    value > MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS
  ) {
    return {
      value: null,
      error:
        `Use a value from 1 to ${MAX_HIVE_WORKER_HEARTBEAT_INTERVAL_SECS} seconds.`,
    };
  }

  return { value, error: null };
}

/** Omit a blank create value so core can apply its autonomy-aware default. */
export function buildWorkerHeartbeatCreateFields(
  value: number | null,
): WorkerHeartbeatCadenceFields {
  return value === null ? {} : { heartbeat_interval_secs: value };
}

/**
 * The current update API treats both an omitted field and JSON null as
 * "retain the stored cadence". Send only a new concrete value so saving an
 * unrelated edit cannot clobber the persisted cadence.
 */
export function buildWorkerHeartbeatUpdateFields(
  value: number | null,
  persistedValue: number | null,
): WorkerHeartbeatCadenceFields {
  return value === null || value === persistedValue
    ? {}
    : { heartbeat_interval_secs: value };
}
