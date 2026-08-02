import { useCallback, useEffect, useState } from "react";
import type { HiveChannelsResponse } from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";

export function useHiveChannels(enabled: boolean) {
  const { client, isConnected } = useConnection();
  const [channels, setChannels] = useState<HiveChannelsResponse | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !isConnected) {
      setChannels(null);
      setIsLoading(false);
      return;
    }

    setError(null);
    setIsRefreshing(true);
    try {
      setChannels(await client.getHiveChannels());
    } catch (refreshError) {
      setError(
        refreshError instanceof Error
          ? refreshError.message
          : "Failed to load Hive channels",
      );
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [client, isConnected]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void refresh();
  }, [enabled, refresh]);

  return {
    channels,
    isLoading,
    isRefreshing,
    error,
    refresh,
  };
}
