import { useCallback, useEffect, useMemo, useState } from "react";
import type { MakoAttentionResponse, MakoCurrentResponse } from "@krusty/api";
import { useConnection } from "../../../hooks/useConnection";
import type { MakoAttentionItem } from "../types";

interface AttentionSections {
  needsAction: MakoAttentionItem[];
  updates: MakoAttentionItem[];
}

function mapItem(
  item: MakoAttentionResponse["items"][number],
): MakoAttentionItem {
  return {
    id: item.id,
    kind: item.kind,
    section: item.section,
    title: item.title,
    summary: item.summary,
    detail: item.detail,
    createdAt: item.created_at,
    read: item.read,
    active: item.active,
    runId: item.run_id ?? null,
    projectDir: item.project_dir ?? null,
    toolCallId: item.tool_call_id ?? null,
    sessionId: item.session_id ?? null,
    threadSessionId: item.thread_session_id ?? null,
    threadMessageId: item.thread_message_id ?? null,
  };
}

export function useMakoAttention(
  current: MakoCurrentResponse | null,
  threadSessionId: string | null,
) {
  const { client, isConnected } = useConnection();
  const [items, setItems] = useState<MakoAttentionItem[]>([]);
  const [badgeCount, setBadgeCount] = useState(0);
  const [unreadCount, setUnreadCount] = useState(0);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !isConnected) {
      setItems([]);
      setBadgeCount(0);
      setUnreadCount(0);
      setIsLoading(false);
      return;
    }

    setError(null);
    try {
      const response = await client.getMakoAttention({
        threadSessionId,
      });
      setItems(response.items.map(mapItem));
      setBadgeCount(response.badge_count);
      setUnreadCount(response.unread_count);
    } catch (refreshError) {
      setError(
        refreshError instanceof Error
          ? refreshError.message
          : "Failed to load attention",
      );
    } finally {
      setIsLoading(false);
    }
  }, [client, isConnected, threadSessionId]);

  useEffect(() => {
    void refresh();
  }, [refresh, current]);

  const sections = useMemo<AttentionSections>(() => {
    const needsAction = items.filter((item) => item.section === "needs_action");
    const updates = items.filter((item) => item.section === "updates");
    return { needsAction, updates };
  }, [items]);

  const markRead = useCallback(
    async (itemId: string, read: boolean) => {
      setItems((previous) =>
        previous.map((item) =>
          item.id === itemId ? { ...item, read } : item,
        ),
      );
      setUnreadCount((currentCount) =>
        read ? Math.max(0, currentCount - 1) : currentCount + 1,
      );
      setBadgeCount((currentCount) => {
        const target = items.find((item) => item.id === itemId);
        if (!target || target.section !== "needs_action") {
          return currentCount;
        }
        return read ? Math.max(0, currentCount - 1) : currentCount + 1;
      });

      if (!client || !isConnected) {
        return;
      }

      try {
        await client.setMakoAttentionRead(itemId, read);
      } catch (markError) {
        setError(
          markError instanceof Error
            ? markError.message
            : "Failed to update attention item",
        );
        void refresh();
      }
    },
    [client, isConnected, items, refresh],
  );

  const clearItem = useCallback(
    async (itemId: string) => {
      const target = items.find((item) => item.id === itemId);
      setItems((previous) => previous.filter((item) => item.id !== itemId));
      if (target && !target.read) {
        setUnreadCount((currentCount) => Math.max(0, currentCount - 1));
      }
      if (target && target.section === "needs_action" && !target.read) {
        setBadgeCount((currentCount) => Math.max(0, currentCount - 1));
      }

      if (!client || !isConnected) {
        return;
      }

      try {
        await client.setMakoAttentionCleared(itemId, true);
      } catch (clearError) {
        setError(
          clearError instanceof Error
            ? clearError.message
            : "Failed to clear attention item",
        );
        void refresh();
      }
    },
    [client, isConnected, items, refresh],
  );

  return {
    items,
    sections,
    badgeCount,
    unreadCount,
    isLoading,
    error,
    refresh,
    markRead,
    clearItem,
  };
}
