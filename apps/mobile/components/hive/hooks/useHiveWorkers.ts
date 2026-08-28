import { useCallback, useEffect, useRef, useState } from "react";
import type {
  ConfirmHiveWorkerIntroductionRequest,
  CreateHiveWorkerRequest,
  HiveWorker,
  HiveWorkerDetail,
  HiveWorkerDmResponse,
  HiveWorkerSessionBindingResponse,
  KeepTalkingHiveWorkerIntroductionRequest,
  MitsuroClient,
  UpdateHiveWorkerRequest,
} from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";
import { createWorkerRosterRefreshCoordinator } from "./workerRosterRefreshCoordinator";

export type UpdateHiveWorkerPatch = Omit<
  UpdateHiveWorkerRequest,
  "expected_revision"
>;

function workerIntroductionKey(): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  if (randomUuid) return `worker-introduction:${randomUuid}`;
  return `worker-introduction:${Date.now()}:${
    Math.random().toString(36).slice(2)
  }`;
}

function workerIntroductionActionKey(
  action: "retry" | "skip" | "confirm" | "keep-talking",
  workerId: string,
): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  const nonce = randomUuid ??
    `${Date.now()}:${Math.random().toString(36).slice(2)}`;
  return `worker-introduction:${workerId}:${action}:${nonce}`;
}

function workerMutationKey(action: string, workerId: string): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  const nonce = randomUuid ??
    `${Date.now()}:${Math.random().toString(36).slice(2)}`;
  return `worker:${workerId}:${action}:${nonce}`;
}

export interface HiveWorkersState {
  workers: HiveWorker[];
  isLoading: boolean;
  isRefreshing: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  loadWorkerDetail: (
    id: string,
    options?: { signal?: AbortSignal; throwOnError?: boolean },
  ) => Promise<HiveWorkerDetail | null>;
  loadWorkerBySession: (
    sessionId: string,
    options?: { signal?: AbortSignal },
  ) => Promise<HiveWorkerSessionBindingResponse | null>;
  createWorker: (request: CreateHiveWorkerRequest) => Promise<HiveWorkerDetail>;
  retryIntroduction: (id: string) => Promise<HiveWorkerDetail>;
  skipIntroduction: (id: string) => Promise<HiveWorkerDetail>;
  confirmIntroduction: (
    id: string,
    request: ConfirmHiveWorkerIntroductionRequest,
  ) => Promise<HiveWorkerDetail>;
  keepTalkingIntroduction: (
    id: string,
    request: KeepTalkingHiveWorkerIntroductionRequest,
  ) => Promise<HiveWorkerDetail>;
  updateWorker: (
    id: string,
    request: UpdateHiveWorkerPatch,
  ) => Promise<HiveWorkerDetail>;
  pauseWorker: (id: string) => Promise<HiveWorkerDetail>;
  resumeWorker: (id: string) => Promise<HiveWorkerDetail>;
  archiveWorker: (id: string) => Promise<HiveWorkerDetail>;
  ensureWorkerDm: (id: string) => Promise<HiveWorkerDmResponse | null>;
}

export function useHiveWorkers(enabled: boolean): HiveWorkersState {
  const { client, isConnected } = useConnection();
  const [workers, setWorkers] = useState<HiveWorker[]>([]);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshCoordinatorRef = useRef(
    createWorkerRosterRefreshCoordinator(),
  );
  const createAttemptRef = useRef<
    {
      fingerprint: string;
      idempotencyKey: string;
    } | null
  >(null);
  const introductionActionAttemptsRef = useRef<
    Map<string, { fingerprint: string; idempotencyKey: string }>
  >(new Map());
  const workerMutationAttemptsRef = useRef<
    Map<
      string,
      {
        fingerprint: string;
        idempotencyKey: string;
        expectedRevision: number;
      }
    >
  >(new Map());

  const runRefresh = useCallback((afterCommit: boolean) => {
    if (!client || !isConnected) {
      refreshCoordinatorRef.current.invalidate();
      setWorkers([]);
      setIsLoading(false);
      setIsRefreshing(false);
      return Promise.resolve();
    }

    const task = async (isCurrent: () => boolean) => {
      setError(null);
      setIsRefreshing(true);
      try {
        const response = await client.listHiveWorkers();
        if (!isCurrent()) return;
        setWorkers((current) =>
          JSON.stringify(current) === JSON.stringify(response.workers)
            ? current
            : response.workers
        );
      } catch (refreshError) {
        if (!isCurrent()) return;
        setError(
          refreshError instanceof Error
            ? refreshError.message
            : "Failed to load Hive Workers",
        );
      } finally {
        if (isCurrent()) {
          setIsLoading(false);
          setIsRefreshing(false);
        }
      }
    };
    return afterCommit
      ? refreshCoordinatorRef.current.runAfterCommit(task)
      : refreshCoordinatorRef.current.run(task);
  }, [client, isConnected]);

  const refresh = useCallback(() => runRefresh(false), [runRefresh]);
  const refreshAfterMutation = useCallback(
    () => runRefresh(true),
    [runRefresh],
  );

  const loadWorkerDetail = useCallback(
    async (
      id: string,
      options?: { signal?: AbortSignal; throwOnError?: boolean },
    ): Promise<HiveWorkerDetail | null> => {
      if (!client || !isConnected) {
        if (options?.throwOnError) {
          throw new Error("Not connected to the Hive server");
        }
        return null;
      }
      try {
        return await client.getHiveWorker(id, options);
      } catch (loadError) {
        if (
          options?.signal?.aborted ||
          (loadError instanceof Error && loadError.name === "AbortError")
        ) {
          return null;
        }
        setError(
          loadError instanceof Error
            ? loadError.message
            : "Failed to load the Worker",
        );
        if (options?.throwOnError) throw loadError;
        return null;
      }
    },
    [client, isConnected],
  );

  const loadWorkerBySession = useCallback(
    async (
      sessionId: string,
      options?: { signal?: AbortSignal },
    ): Promise<HiveWorkerSessionBindingResponse | null> => {
      if (!client || !isConnected) return null;
      try {
        return await client.getHiveWorkerBySession(sessionId, options);
      } catch (loadError) {
        if (
          options?.signal?.aborted ||
          (loadError instanceof Error && loadError.name === "AbortError")
        ) {
          return null;
        }
        setError(
          loadError instanceof Error
            ? loadError.message
            : "Failed to resolve the Hive conversation",
        );
        throw loadError;
      }
    },
    [client, isConnected],
  );

  const mutate = useCallback(
    async <T>(
      fallbackMessage: string,
      run: (client: MitsuroClient) => Promise<T>,
    ): Promise<T> => {
      if (!client || !isConnected) {
        throw new Error("Not connected to the Hive server");
      }
      setIsSaving(true);
      setError(null);
      try {
        const result = await run(client);
        await refreshAfterMutation();
        return result;
      } catch (mutationError) {
        setError(
          mutationError instanceof Error
            ? mutationError.message
            : fallbackMessage,
        );
        throw mutationError;
      } finally {
        setIsSaving(false);
      }
    },
    [client, isConnected, refreshAfterMutation],
  );

  const createWorker = useCallback(
    async (request: CreateHiveWorkerRequest) => {
      const fingerprint = JSON.stringify(request);
      if (createAttemptRef.current?.fingerprint !== fingerprint) {
        createAttemptRef.current = {
          fingerprint,
          idempotencyKey: workerIntroductionKey(),
        };
      }
      const attempt = createAttemptRef.current;
      try {
        const created = await mutate(
          "Failed to create the Worker",
          (api) =>
            api.createHiveWorker(request, {
              idempotencyKey: attempt.idempotencyKey,
            }),
        );
        createAttemptRef.current = null;
        return created;
      } catch (error) {
        // Preserve the same key while the modal remains on the same request.
        // Retrying a lost HTTP response must replay the committed Worker.
        throw error;
      }
    },
    [mutate],
  );

  const updateWorker = useCallback(
    async (id: string, request: UpdateHiveWorkerPatch) => {
      const worker = workers.find((candidate) => candidate.id === id);
      if (!worker) throw new Error("Worker is no longer in the roster");
      const attemptId = `${id}:update`;
      const fingerprint = JSON.stringify(request);
      let attempt = workerMutationAttemptsRef.current.get(attemptId);
      if (attempt?.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: workerMutationKey("update", id),
          expectedRevision: worker.revision,
        };
        workerMutationAttemptsRef.current.set(attemptId, attempt);
      }
      try {
        const detail = await mutate(
          "Failed to update the Worker",
          (api) =>
            api.updateHiveWorker(
              id,
              { ...request, expected_revision: attempt.expectedRevision },
              { idempotencyKey: attempt.idempotencyKey },
            ),
        );
        workerMutationAttemptsRef.current.delete(attemptId);
        return detail;
      } catch (mutationError) {
        if (
          mutationError instanceof Error &&
          mutationError.message.toLowerCase().includes("revision")
        ) {
          workerMutationAttemptsRef.current.delete(attemptId);
          await refresh();
        }
        throw mutationError;
      }
    },
    [mutate, refresh, workers],
  );

  const retryIntroduction = useCallback(
    async (id: string) => {
      const attemptId = `${id}:retry`;
      const fingerprint = "retry";
      let attempt = introductionActionAttemptsRef.current.get(attemptId);
      if (attempt?.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: workerIntroductionActionKey("retry", id),
        };
        introductionActionAttemptsRef.current.set(attemptId, attempt);
      }
      try {
        const detail = await mutate(
          "Failed to retry the Worker Introduction",
          (api) =>
            api.retryHiveWorkerIntroduction(id, {
              idempotencyKey: attempt.idempotencyKey,
            }),
        );
        introductionActionAttemptsRef.current.delete(attemptId);
        return detail;
      } catch (actionError) {
        // Preserve the key when an HTTP response is lost so a user retry
        // adopts the daemon's original transaction instead of creating a
        // second run.
        throw actionError;
      }
    },
    [mutate],
  );

  const skipIntroduction = useCallback(
    async (id: string) => {
      const attemptId = `${id}:skip`;
      const fingerprint = "skip";
      let attempt = introductionActionAttemptsRef.current.get(attemptId);
      if (attempt?.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: workerIntroductionActionKey("skip", id),
        };
        introductionActionAttemptsRef.current.set(attemptId, attempt);
      }
      try {
        const detail = await mutate(
          "Failed to skip the Worker Introduction",
          (api) =>
            api.skipHiveWorkerIntroduction(id, {
              idempotencyKey: attempt.idempotencyKey,
            }),
        );
        introductionActionAttemptsRef.current.delete(attemptId);
        return detail;
      } catch (actionError) {
        throw actionError;
      }
    },
    [mutate],
  );

  const confirmIntroduction = useCallback(
    async (id: string, request: ConfirmHiveWorkerIntroductionRequest) => {
      const attemptId = `${id}:confirm`;
      const fingerprint = JSON.stringify(request);
      let attempt = introductionActionAttemptsRef.current.get(attemptId);
      if (attempt?.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: workerIntroductionActionKey("confirm", id),
        };
        introductionActionAttemptsRef.current.set(attemptId, attempt);
      }
      try {
        const detail = await mutate(
          "Failed to confirm the Worker Introduction",
          (api) =>
            api.confirmHiveWorkerIntroduction(id, request, {
              idempotencyKey: attempt.idempotencyKey,
            }),
        );
        introductionActionAttemptsRef.current.delete(attemptId);
        return detail;
      } catch (actionError) {
        throw actionError;
      }
    },
    [mutate],
  );

  const keepTalkingIntroduction = useCallback(
    async (id: string, request: KeepTalkingHiveWorkerIntroductionRequest) => {
      const attemptId = `${id}:keep-talking`;
      const fingerprint = JSON.stringify(request);
      let attempt = introductionActionAttemptsRef.current.get(attemptId);
      if (attempt?.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: workerIntroductionActionKey("keep-talking", id),
        };
        introductionActionAttemptsRef.current.set(attemptId, attempt);
      }
      try {
        const detail = await mutate(
          "Failed to keep talking with the Worker",
          (api) =>
            api.keepTalkingHiveWorkerIntroduction(id, request, {
              idempotencyKey: attempt.idempotencyKey,
            }),
        );
        introductionActionAttemptsRef.current.delete(attemptId);
        return detail;
      } catch (actionError) {
        throw actionError;
      }
    },
    [mutate],
  );

  const mutateWorkerStatus = useCallback(
    async (id: string, action: "pause" | "resume" | "archive") => {
      const worker = workers.find((candidate) => candidate.id === id);
      if (!worker) throw new Error("Worker is no longer in the roster");
      const attemptId = `${id}:${action}`;
      const fingerprint = `${action}:${worker.revision}`;
      let attempt = workerMutationAttemptsRef.current.get(attemptId);
      if (attempt?.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: workerMutationKey(action, id),
          expectedRevision: worker.revision,
        };
        workerMutationAttemptsRef.current.set(attemptId, attempt);
      }
      try {
        const detail = await mutate(`Failed to ${action} the Worker`, (api) => {
          const options = { idempotencyKey: attempt.idempotencyKey };
          switch (action) {
            case "pause":
              return api.pauseHiveWorker(id, attempt.expectedRevision, options);
            case "resume":
              return api.resumeHiveWorker(
                id,
                attempt.expectedRevision,
                options,
              );
            case "archive":
              return api.archiveHiveWorker(
                id,
                attempt.expectedRevision,
                options,
              );
          }
        });
        workerMutationAttemptsRef.current.delete(attemptId);
        return detail;
      } catch (mutationError) {
        if (
          mutationError instanceof Error &&
          mutationError.message.toLowerCase().includes("revision")
        ) {
          workerMutationAttemptsRef.current.delete(attemptId);
          await refresh();
        }
        throw mutationError;
      }
    },
    [mutate, refresh, workers],
  );

  const pauseWorker = useCallback(
    (id: string) => mutateWorkerStatus(id, "pause"),
    [mutateWorkerStatus],
  );

  const resumeWorker = useCallback(
    (id: string) => mutateWorkerStatus(id, "resume"),
    [mutateWorkerStatus],
  );

  const archiveWorker = useCallback(
    (id: string) => mutateWorkerStatus(id, "archive"),
    [mutateWorkerStatus],
  );

  const ensureWorkerDm = useCallback(
    async (id: string): Promise<HiveWorkerDmResponse | null> => {
      if (!client || !isConnected) return null;
      setError(null);
      try {
        const response = await client.ensureHiveWorkerDm(id);
        if (response.created) {
          void refresh();
        }
        return response;
      } catch (dmError) {
        setError(
          dmError instanceof Error
            ? dmError.message
            : "Failed to open the Worker DM",
        );
        return null;
      }
    },
    [client, isConnected, refresh],
  );

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void refresh();
    return () => {
      refreshCoordinatorRef.current.invalidate();
    };
  }, [enabled, refresh]);

  return {
    workers,
    isLoading,
    isRefreshing,
    isSaving,
    error,
    refresh,
    loadWorkerDetail,
    loadWorkerBySession,
    createWorker,
    retryIntroduction,
    skipIntroduction,
    confirmIntroduction,
    keepTalkingIntroduction,
    updateWorker,
    pauseWorker,
    resumeWorker,
    archiveWorker,
    ensureWorkerDm,
  };
}
