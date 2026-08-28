import type { SessionType } from "@mitsuro/api";

// Structural mirror of the public state-store lease. Keeping this tiny helper
// free of a package-runtime import also lets its orchestration regressions run
// directly under Deno in the mobile workspace.
export interface SessionDeletionAdmission {
  commit(): void;
  rollback(): Promise<void>;
}

interface SessionDeletionState {
  beginSessionDeletionAdmission(
    sessionId: string,
  ): Promise<SessionDeletionAdmission>;
}

interface SessionDeletionStore {
  getState(): SessionDeletionState;
}

export type SessionDeletionModeStores = Record<
  SessionType,
  { session: SessionDeletionStore }
>;

export type SessionDeletionProjectionModeStores = Record<
  SessionType,
  {
    session: {
      getState(): {
        sessionId: string | null;
        clearSession(): void;
      };
    };
  }
>;

const SESSION_MODES = ["chat", "code", "hive"] as const;

function deletionAdmissionError(error: unknown): Error {
  return error instanceof Error
    ? error
    : new Error("Queued recovery could not be secured for deletion.");
}

async function rollbackAdmissions(
  admissions: readonly SessionDeletionAdmission[],
): Promise<void> {
  const results = await Promise.allSettled(
    admissions.map((admission) => admission.rollback()),
  );
  const failed = results.find((result) => result.status === "rejected");
  if (failed?.status === "rejected") {
    throw deletionAdmissionError(failed.reason);
  }
}

/**
 * Fence every mode synchronously, then wait for all pre-existing producers and
 * durable scrubs before returning one lease to hold across server DELETE.
 */
export async function beginAllModeSessionDeletionAdmission(
  modeStores: SessionDeletionModeStores,
  sessionId: string,
): Promise<SessionDeletionAdmission> {
  // Calling every async entrypoint before awaiting is intentional: each core
  // store installs its producer fence synchronously before its first await.
  const pending = SESSION_MODES.map((mode) => {
    try {
      return modeStores[mode].session.getState()
        .beginSessionDeletionAdmission(sessionId);
    } catch (error) {
      // One synchronous acquisition failure must not prevent the remaining
      // mode stores from installing their fences; all fulfilled leases below
      // are rolled back before the failure reaches transport.
      return Promise.reject(error);
    }
  });
  const results = await Promise.allSettled(pending);
  const admissions = results.flatMap((result) =>
    result.status === "fulfilled" ? [result.value] : []
  );
  const failed = results.find((result) => result.status === "rejected");
  if (failed?.status === "rejected") {
    try {
      await rollbackAdmissions(admissions);
    } catch (rollbackError) {
      throw deletionAdmissionError(rollbackError);
    }
    throw deletionAdmissionError(failed.reason);
  }

  let settled = false;
  let rollbackInFlight: Promise<void> | null = null;
  return {
    commit() {
      if (settled) return;
      settled = true;
      for (const admission of admissions) admission.commit();
    },

    async rollback() {
      if (settled) return;
      if (rollbackInFlight) return rollbackInFlight;
      rollbackInFlight = (async () => {
        await rollbackAdmissions(admissions);
        settled = true;
      })();
      try {
        await rollbackInFlight;
      } finally {
        if (!settled) rollbackInFlight = null;
      }
    },
  };
}

/** Roll back every still-held per-session batch lease before returning. */
export async function rollbackSessionDeletionAdmissions(
  admissions: ReadonlyMap<string, SessionDeletionAdmission>,
): Promise<void> {
  await rollbackAdmissions([...admissions.values()]);
}

/** Detach an exact deleted ID from every mode before its fence is released. */
export function clearDeletedSessionFromModeStores(
  modeStores: SessionDeletionProjectionModeStores,
  sessionId: string,
): boolean {
  let cleared = false;
  for (const mode of SESSION_MODES) {
    const state = modeStores[mode].session.getState();
    if (state.sessionId !== sessionId) continue;
    state.clearSession();
    cleared = true;
  }
  return cleared;
}

/** Detach both the admitted graph and a same-lifecycle replacement graph. */
export function clearDeletedSessionFromModeStoreGraphs(
  capturedModeStores: SessionDeletionProjectionModeStores,
  currentModeStores: SessionDeletionProjectionModeStores | null,
  sessionId: string,
): boolean {
  const capturedCleared = clearDeletedSessionFromModeStores(
    capturedModeStores,
    sessionId,
  );
  if (!currentModeStores || currentModeStores === capturedModeStores) {
    return capturedCleared;
  }
  return clearDeletedSessionFromModeStores(currentModeStores, sessionId) ||
    capturedCleared;
}

export interface SessionDeletionBatchResult {
  deletedIds: string[];
  remainingIds: string[];
  error: unknown | null;
  boundaryChanged: boolean;
}

/**
 * Run an already-admitted batch sequentially. The first failed transport
 * restores that session and every session that has not reached transport yet.
 */
export async function runSessionDeletionBatch(
  ids: readonly string[],
  admissions: Map<string, SessionDeletionAdmission>,
  deleteSession: (sessionId: string) => Promise<boolean>,
  isCurrentBoundary: () => boolean = () => true,
  beforeCommit: (sessionId: string) => void = () => {},
): Promise<SessionDeletionBatchResult> {
  const deletedIds: string[] = [];
  const rollbackOutstanding = async (): Promise<unknown | null> => {
    try {
      await rollbackSessionDeletionAdmissions(admissions);
      admissions.clear();
      return null;
    } catch (rollbackError) {
      // Leave unresolved leases visible to this batch. Core keeps admission
      // closed and makes a later begin finish the failed restore before it may
      // acquire a fresh lease or start another DELETE transport.
      return rollbackError;
    }
  };

  for (let index = 0; index < ids.length; index += 1) {
    if (!isCurrentBoundary()) {
      return {
        deletedIds,
        remainingIds: ids.slice(index),
        error: await rollbackOutstanding(),
        boundaryChanged: true,
      };
    }

    const sessionId = ids[index];
    const admission = admissions.get(sessionId);
    if (!admission) {
      const rollbackError = await rollbackOutstanding();
      return {
        deletedIds,
        remainingIds: ids.slice(index),
        error: rollbackError ?? new Error("A deletion admission was lost."),
        boundaryChanged: false,
      };
    }

    let deleted = false;
    let deleteError: unknown = null;
    try {
      deleted = await deleteSession(sessionId);
    } catch (error) {
      deleteError = error;
    }
    if (!deleted) {
      const rollbackError = await rollbackOutstanding();
      return {
        deletedIds,
        remainingIds: ids.slice(index),
        error: rollbackError ?? deleteError,
        boundaryChanged: !isCurrentBoundary(),
      };
    }

    // Server deletion is already authoritative. Detach every producer from
    // this exact ID while its admission remains held, then release the fence.
    beforeCommit(sessionId);
    admission.commit();
    admissions.delete(sessionId);
    deletedIds.push(sessionId);
    if (!isCurrentBoundary()) {
      return {
        deletedIds,
        remainingIds: ids.slice(index + 1),
        error: await rollbackOutstanding(),
        boundaryChanged: true,
      };
    }
  }

  return {
    deletedIds,
    remainingIds: [],
    error: null,
    boundaryChanged: false,
  };
}
