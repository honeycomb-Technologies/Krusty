import { useCallback, useEffect, useState } from "react";
import { useConnection } from "../../../hooks/useConnection";
import type { AgentMemory, MemoryType } from "@krusty/api";
import type { MakoKnowledgeScope } from "../types";

export function useMakoMemories(
  enabled: boolean,
  workspaceDirectory?: string | null,
) {
  const { client, isConnected } = useConnection();
  const [scope, setScope] = useState<MakoKnowledgeScope>(
    workspaceDirectory ? "workspace" : "all",
  );
  const [typeFilter, setTypeFilter] = useState<MemoryType | "all">("all");
  const [memories, setMemories] = useState<AgentMemory[]>([]);
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!workspaceDirectory && scope === "workspace") {
      setScope("all");
    }
  }, [scope, workspaceDirectory]);

  const fetchMemories = useCallback(async () => {
    if (!client || !isConnected) {
      setMemories([]);
      setSelectedMemoryId(null);
      setIsLoading(false);
      setIsRefreshing(false);
      return;
    }

    setError(null);
    try {
      const response = await client.getMemories(
        scope === "workspace" ? workspaceDirectory ?? undefined : undefined,
        typeFilter === "all" ? undefined : typeFilter,
      );
      setMemories(response.memories);
    } catch (loadError) {
      setError(
        loadError instanceof Error
          ? loadError.message
          : "Failed to load memories.",
      );
      setMemories([]);
      setSelectedMemoryId(null);
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [client, isConnected, scope, typeFilter, workspaceDirectory]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    setIsLoading(true);
    void fetchMemories();
  }, [enabled, fetchMemories]);

  useEffect(() => {
    if (
      selectedMemoryId &&
      !memories.some((memory) => memory.id === selectedMemoryId)
    ) {
      setSelectedMemoryId(null);
    }
  }, [memories, selectedMemoryId]);

  const refresh = useCallback(async () => {
    setIsRefreshing(true);
    await fetchMemories();
  }, [fetchMemories]);

  const clearSelection = useCallback(() => {
    setSelectedMemoryId(null);
  }, []);

  const selectedMemory =
    selectedMemoryId
      ? memories.find((memory) => memory.id === selectedMemoryId) ?? null
      : null;

  return {
    scope,
    setScope,
    typeFilter,
    setTypeFilter,
    memories,
    selectedMemoryId,
    selectedMemory,
    isLoading,
    isRefreshing,
    error,
    refresh,
    setSelectedMemoryId,
    clearSelection,
  };
}
