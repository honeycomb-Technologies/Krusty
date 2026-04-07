import { useCallback, useEffect, useState } from "react";
import { useConnection } from "../../../hooks/useConnection";
import type { MakoCurrentResponse } from "@krusty/api";

export function useMakoCurrent(enabled: boolean) {
  const { client, isConnected } = useConnection();
  const [current, setCurrent] = useState<MakoCurrentResponse | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isDispatching, setIsDispatching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !isConnected) {
      setCurrent(null);
      setIsLoading(false);
      return;
    }

    setError(null);
    setIsRefreshing(true);
    try {
      const response = await client.getMakoCurrent();
      setCurrent(response);
    } catch (refreshError) {
      setError(
        refreshError instanceof Error
          ? refreshError.message
          : "Failed to load Mako",
      );
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [client, isConnected]);

  const setCourse = useCallback(
    async (task: string, options?: { projectDir?: string | null; model?: string | null }) => {
      if (!client || !isConnected) {
        return null;
      }

      setIsDispatching(true);
      setError(null);
      try {
        const response = await client.dispatchMako(task, {
          projectDir: options?.projectDir ?? undefined,
          model: options?.model ?? undefined,
        });
        await refresh();
        return response.session_id;
      } catch (dispatchError) {
        setError(
          dispatchError instanceof Error
            ? dispatchError.message
            : "Failed to set course",
        );
        return null;
      } finally {
        setIsDispatching(false);
      }
    },
    [client, isConnected, refresh],
  );

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void refresh();
  }, [enabled, refresh]);

  return {
    current,
    isLoading,
    isRefreshing,
    error,
    refresh,
    setCourse,
    isDispatching,
  };
}
