import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CreateHiveWorkerRequest,
  HiveWorker,
  HiveWorkerDetail,
  HiveWorkerDmResponse,
  MitsuroClient,
  UpdateHiveWorkerRequest,
} from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";

export interface HiveWorkersState {
  workers: HiveWorker[];
  isLoading: boolean;
  isRefreshing: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  loadWorkerDetail: (id: string) => Promise<HiveWorkerDetail | null>;
  createWorker: (request: CreateHiveWorkerRequest) => Promise<HiveWorkerDetail>;
  updateWorker: (
    id: string,
    request: UpdateHiveWorkerRequest,
  ) => Promise<HiveWorkerDetail>;
  pauseWorker: (id: string) => Promise<void>;
  resumeWorker: (id: string) => Promise<void>;
  archiveWorker: (id: string) => Promise<void>;
  ensureWorkerDm: (id: string) => Promise<HiveWorkerDmResponse | null>;
}

export function useHiveWorkers(enabled: boolean): HiveWorkersState {
  const { client, isConnected } = useConnection();
  const [workers, setWorkers] = useState<HiveWorker[]>([]);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshPromiseRef = useRef<Promise<void> | null>(null);
  const refreshGenerationRef = useRef(0);

  const refresh = useCallback(() => {
    if (!client || !isConnected) {
      setWorkers([]);
      setIsLoading(false);
      return Promise.resolve();
    }
    if (refreshPromiseRef.current) return refreshPromiseRef.current;

    const generation = ++refreshGenerationRef.current;
    const request = (async () => {
      setError(null);
      setIsRefreshing(true);
      try {
        const response = await client.listHiveWorkers();
        if (generation !== refreshGenerationRef.current) return;
        setWorkers((current) =>
          JSON.stringify(current) === JSON.stringify(response.workers)
            ? current
            : response.workers,
        );
      } catch (refreshError) {
        if (generation !== refreshGenerationRef.current) return;
        setError(
          refreshError instanceof Error
            ? refreshError.message
            : "Failed to load Hive Workers",
        );
      } finally {
        if (generation === refreshGenerationRef.current) {
          setIsLoading(false);
          setIsRefreshing(false);
        }
      }
    })();
    refreshPromiseRef.current = request;
    void request.finally(() => {
      if (refreshPromiseRef.current === request) refreshPromiseRef.current = null;
    });
    return request;
  }, [client, isConnected]);

  const loadWorkerDetail = useCallback(
    async (id: string): Promise<HiveWorkerDetail | null> => {
      if (!client || !isConnected) return null;
      try {
        return await client.getHiveWorker(id);
      } catch (loadError) {
        setError(
          loadError instanceof Error
            ? loadError.message
            : "Failed to load the Worker",
        );
        return null;
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
        await refresh();
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
    [client, isConnected, refresh],
  );

  const createWorker = useCallback(
    (request: CreateHiveWorkerRequest) =>
      mutate("Failed to create the Worker", (api) =>
        api.createHiveWorker(request),
      ),
    [mutate],
  );

  const updateWorker = useCallback(
    (id: string, request: UpdateHiveWorkerRequest) =>
      mutate("Failed to update the Worker", (api) =>
        api.updateHiveWorker(id, request),
      ),
    [mutate],
  );

  const pauseWorker = useCallback(
    async (id: string) => {
      await mutate("Failed to pause the Worker", (api) =>
        api.pauseHiveWorker(id),
      );
    },
    [mutate],
  );

  const resumeWorker = useCallback(
    async (id: string) => {
      await mutate("Failed to resume the Worker", (api) =>
        api.resumeHiveWorker(id),
      );
    },
    [mutate],
  );

  const archiveWorker = useCallback(
    async (id: string) => {
      await mutate("Failed to archive the Worker", (api) =>
        api.archiveHiveWorker(id),
      );
    },
    [mutate],
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
      refreshGenerationRef.current += 1;
      refreshPromiseRef.current = null;
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
    createWorker,
    updateWorker,
    pauseWorker,
    resumeWorker,
    archiveWorker,
    ensureWorkerDm,
  };
}
