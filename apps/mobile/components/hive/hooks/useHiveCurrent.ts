import { useCallback, useEffect, useRef, useState } from "react";
import { useConnection } from "../../../hooks/useConnection";
import type {
  HiveCurrentResponse,
  HiveRunPriority,
  ModelKey,
} from "@mitsuro/api";

export function useHiveCurrent(enabled: boolean) {
  const { client, isConnected } = useConnection();
  const [current, setCurrent] = useState<HiveCurrentResponse | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isDispatching, setIsDispatching] = useState(false);
  const [isRecovering, setIsRecovering] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshPromiseRef = useRef<Promise<void> | null>(null);
  const refreshGenerationRef = useRef(0);

  const refresh = useCallback(() => {
    if (!client || !isConnected) {
      setCurrent(null);
      setIsLoading(false);
      return Promise.resolve();
    }
    if (refreshPromiseRef.current) return refreshPromiseRef.current;

    const generation = ++refreshGenerationRef.current;
    const request = (async () => {
      setError(null);
      setIsRefreshing(true);
      try {
        const response = await client.getHiveCurrent();
        if (generation !== refreshGenerationRef.current) return;
        setCurrent((current) => JSON.stringify(current) === JSON.stringify(response)
          ? current
          : response);
      } catch (refreshError) {
        if (generation !== refreshGenerationRef.current) return;
        setError(
          refreshError instanceof Error
            ? refreshError.message
            : "Failed to load Hive",
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

  const setCourse = useCallback(
    async (
      task: string,
        options?: {
          projectDir?: string | null;
          model?: string | null;
          modelKey?: ModelKey | null;
          startAt?: string | null;
          priority?: HiveRunPriority | null;
          crewSlug?: string | null;
        },
    ) => {
      if (!client || !isConnected) {
        return null;
      }

      setIsDispatching(true);
      setError(null);
      try {
        const response = await client.dispatchHive(task, {
          projectDir: options?.projectDir ?? undefined,
          model: options?.model ?? undefined,
          modelKey: options?.modelKey ?? undefined,
          startAt: options?.startAt ?? undefined,
          priority: options?.priority ?? undefined,
          crewSlug: options?.crewSlug ?? undefined,
        });
        await refresh();
        return response.session_id;
      } catch (dispatchError) {
        setError(
          dispatchError instanceof Error
            ? dispatchError.message
            : "Failed to start Hive run",
        );
        return null;
      } finally {
        setIsDispatching(false);
      }
    },
    [client, isConnected, refresh],
  );

  const recoverDaemon = useCallback(async () => {
    if (!client || !isConnected) {
      return 0;
    }

    setIsRecovering(true);
    setError(null);
    try {
      const response = await client.recoverHiveDaemon();
      await refresh();
      return response.recovered_count;
    } catch (recoverError) {
      setError(
        recoverError instanceof Error
          ? recoverError.message
          : "Failed to recover daemon",
      );
      return 0;
    } finally {
      setIsRecovering(false);
    }
  }, [client, isConnected, refresh]);

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
    current,
    isLoading,
    isRefreshing,
    isRecovering,
    error,
    refresh,
    setCourse,
    recoverDaemon,
    isDispatching,
  };
}
