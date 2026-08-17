import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CreateHiveGroupRequest,
  HiveGroup,
  HiveGroupDetail,
  MitsuroClient,
  UpdateHiveGroupRequest,
} from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";

export interface HiveGroupsState {
  groups: HiveGroup[];
  isLoading: boolean;
  isRefreshing: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createGroup: (request: CreateHiveGroupRequest) => Promise<HiveGroupDetail>;
  updateGroup: (
    id: string,
    request: UpdateHiveGroupRequest,
  ) => Promise<HiveGroupDetail>;
  archiveGroup: (id: string) => Promise<void>;
}

/** Group roster state; fetched only while a Groups surface is visible. */
export function useHiveGroups(enabled: boolean): HiveGroupsState {
  const { client, isConnected } = useConnection();
  const [groups, setGroups] = useState<HiveGroup[]>([]);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshPromiseRef = useRef<Promise<void> | null>(null);
  const refreshGenerationRef = useRef(0);

  const refresh = useCallback(() => {
    if (!client || !isConnected) {
      setGroups([]);
      setIsLoading(false);
      return Promise.resolve();
    }
    if (refreshPromiseRef.current) return refreshPromiseRef.current;

    const generation = ++refreshGenerationRef.current;
    const request = (async () => {
      setError(null);
      setIsRefreshing(true);
      try {
        const response = await client.listHiveGroups();
        if (generation !== refreshGenerationRef.current) return;
        setGroups((current) =>
          JSON.stringify(current) === JSON.stringify(response.groups)
            ? current
            : response.groups,
        );
      } catch (refreshError) {
        if (generation !== refreshGenerationRef.current) return;
        setError(
          refreshError instanceof Error
            ? refreshError.message
            : "Failed to load Groups",
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

  const createGroup = useCallback(
    (request: CreateHiveGroupRequest) =>
      mutate("Failed to create the Group", (api) => api.createHiveGroup(request)),
    [mutate],
  );

  const updateGroup = useCallback(
    (id: string, request: UpdateHiveGroupRequest) =>
      mutate("Failed to update the Group", (api) =>
        api.updateHiveGroup(id, request),
      ),
    [mutate],
  );

  const archiveGroup = useCallback(
    async (id: string) => {
      await mutate("Failed to archive the Group", (api) =>
        api.archiveHiveGroup(id),
      );
    },
    [mutate],
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
    groups,
    isLoading,
    isRefreshing,
    isSaving,
    error,
    refresh,
    createGroup,
    updateGroup,
    archiveGroup,
  };
}
