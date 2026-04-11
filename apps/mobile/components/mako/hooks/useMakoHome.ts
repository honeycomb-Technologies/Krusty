import { useCallback, useEffect, useState } from "react";
import type {
  MakoCrewDocumentKind,
  MakoCrewResponse,
  MakoHomeDocumentKind,
  MakoHomeResponse,
} from "@krusty/api";
import { useConnection } from "../../../hooks/useConnection";

export function useMakoHome(enabled: boolean) {
  const { client, isConnected } = useConnection();
  const [home, setHome] = useState<MakoHomeResponse | null>(null);
  const [crew, setCrew] = useState<MakoCrewResponse | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isBootstrapping, setIsBootstrapping] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !isConnected) {
      setHome(null);
      setCrew(null);
      setIsLoading(false);
      return;
    }

    setError(null);
    setIsRefreshing(true);
    try {
      const [nextHome, nextCrew] = await Promise.all([
        client.getMakoHome(),
        client.getMakoCrew(),
      ]);
      setHome(nextHome);
      setCrew(nextCrew);
    } catch (refreshError) {
      setError(
        refreshError instanceof Error
          ? refreshError.message
          : "Failed to load Mako home",
      );
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [client, isConnected]);

  const bootstrap = useCallback(async () => {
    if (!client || !isConnected) {
      return;
    }

    setIsBootstrapping(true);
    setError(null);
    try {
      const response = await client.bootstrapMakoHome();
      setHome(response.home);
      const nextCrew = await client.getMakoCrew();
      setCrew(nextCrew);
    } catch (bootstrapError) {
      setError(
        bootstrapError instanceof Error
          ? bootstrapError.message
          : "Failed to bootstrap Mako home",
      );
    } finally {
      setIsBootstrapping(false);
    }
  }, [client, isConnected]);

  const updateHomeDocument = useCallback(
    async (kind: MakoHomeDocumentKind, content: string) => {
      if (!client || !isConnected) {
        return;
      }

      setIsSaving(true);
      setError(null);
      try {
        const nextHome = await client.updateMakoHomeDocument(kind, content);
        setHome(nextHome);
        const nextCrew = await client.getMakoCrew();
        setCrew(nextCrew);
      } catch (saveError) {
        setError(
          saveError instanceof Error
            ? saveError.message
            : "Failed to update Mako home",
        );
        throw saveError;
      } finally {
        setIsSaving(false);
      }
    },
    [client, isConnected],
  );

  const updateCrewDocument = useCallback(
    async (slug: string, kind: MakoCrewDocumentKind, content: string) => {
      if (!client || !isConnected) {
        return;
      }

      setIsSaving(true);
      setError(null);
      try {
        const nextHome = await client.updateMakoCrewDocument(slug, kind, content);
        setHome(nextHome);
        const nextCrew = await client.getMakoCrew();
        setCrew(nextCrew);
      } catch (saveError) {
        setError(
          saveError instanceof Error
            ? saveError.message
            : "Failed to update crew profile",
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
