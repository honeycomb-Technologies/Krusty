import { useCallback, useEffect, useRef, useState } from "react";
import type {
  HiveCrewDocumentKind,
  HiveCrewResponse,
  HiveHomeDocumentKind,
  HiveHomeResponse,
} from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";

export function useHiveHome(enabled: boolean) {
  const { client, isConnected } = useConnection();
  const [home, setHome] = useState<HiveHomeResponse | null>(null);
  const [crew, setCrew] = useState<HiveCrewResponse | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isBootstrapping, setIsBootstrapping] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshPromiseRef = useRef<Promise<void> | null>(null);
  const refreshGenerationRef = useRef(0);

  const refresh = useCallback(() => {
    if (!client || !isConnected) {
      setHome(null);
      setCrew(null);
      setIsLoading(false);
      return Promise.resolve();
    }
    if (refreshPromiseRef.current) return refreshPromiseRef.current;

    const generation = ++refreshGenerationRef.current;
    const request = (async () => {
      setError(null);
      setIsRefreshing(true);
      try {
        const [nextHome, nextCrew] = await Promise.all([
          client.getHiveHome(),
          client.getHiveCrew(),
        ]);
        if (generation !== refreshGenerationRef.current) return;
        setHome((current) => JSON.stringify(current) === JSON.stringify(nextHome)
          ? current
          : nextHome);
        setCrew((current) => JSON.stringify(current) === JSON.stringify(nextCrew)
          ? current
          : nextCrew);
      } catch (refreshError) {
        if (generation !== refreshGenerationRef.current) return;
        setError(
          refreshError instanceof Error
            ? refreshError.message
            : "Failed to load Hive home",
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

  const bootstrap = useCallback(async () => {
    if (!client || !isConnected) {
      return;
    }

    setIsBootstrapping(true);
    setError(null);
    try {
      const response = await client.bootstrapHiveHome();
      setHome(response.home);
      const nextCrew = await client.getHiveCrew();
      setCrew(nextCrew);
    } catch (bootstrapError) {
      setError(
        bootstrapError instanceof Error
          ? bootstrapError.message
          : "Failed to bootstrap Hive home",
      );
    } finally {
      setIsBootstrapping(false);
    }
  }, [client, isConnected]);

  const updateHomeDocument = useCallback(
    async (kind: HiveHomeDocumentKind, content: string) => {
      if (!client || !isConnected) {
        return;
      }

      setIsSaving(true);
      setError(null);
      try {
        const nextHome = await client.updateHiveHomeDocument(kind, content);
        setHome(nextHome);
        const nextCrew = await client.getHiveCrew();
        setCrew(nextCrew);
      } catch (saveError) {
        setError(
          saveError instanceof Error
            ? saveError.message
            : "Failed to update Hive home",
        );
        throw saveError;
      } finally {
        setIsSaving(false);
      }
    },
    [client, isConnected],
  );

  const updateCrewDocument = useCallback(
    async (slug: string, kind: HiveCrewDocumentKind, content: string) => {
      if (!client || !isConnected) {
        return;
      }

      setIsSaving(true);
      setError(null);
      try {
        const nextHome = await client.updateHiveCrewDocument(slug, kind, content);
        setHome(nextHome);
        const nextCrew = await client.getHiveCrew();
        setCrew(nextCrew);
      } catch (saveError) {
        setError(
          saveError instanceof Error
            ? saveError.message
            : "Failed to update Hive Worker profile",
        );
        throw saveError;
      } finally {
        setIsSaving(false);
      }
    },
    [client, isConnected],
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
    home,
    crew,
    isLoading,
    isRefreshing,
    isBootstrapping,
    isSaving,
    error,
    refresh,
    bootstrap,
    updateHomeDocument,
    updateCrewDocument,
  };
}
