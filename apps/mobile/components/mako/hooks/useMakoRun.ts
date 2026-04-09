import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useConnection } from "../../../hooks/useConnection";
import { buildWakeEvents } from "../utils";
import type { MakoRunWakeEvent, MakoSessionStatus, StreamCallbacks } from "@krusty/api";

const REFRESH_DEBOUNCE_MS = 180;

function noop() {}

export function useMakoRun(sessionId: string | null, enabled: boolean) {
  const { client, isConnected } = useConnection();
  const [status, setStatus] = useState<MakoSessionStatus | null>(null);
  const [isLoading, setIsLoading] = useState(Boolean(enabled && sessionId));
  const [error, setError] = useState<string | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !isConnected || !sessionId) {
      setStatus(null);
      setIsLoading(false);
      return;
    }

    setError(null);
    try {
      const response = await client.getMakoSessionStatus(sessionId);
      setStatus(response);
    } catch (refreshError) {
      setError(
        refreshError instanceof Error
          ? refreshError.message
          : "Failed to load run",
      );
    } finally {
      setIsLoading(false);
    }
  }, [client, isConnected, sessionId]);

  const queueRefresh = useCallback(() => {
    if (refreshTimerRef.current) {
      clearTimeout(refreshTimerRef.current);
    }
    refreshTimerRef.current = setTimeout(() => {
      void refresh();
    }, REFRESH_DEBOUNCE_MS);
  }, [refresh]);

  useEffect(() => {
    if (!enabled || !client || !isConnected || !sessionId) {
      return;
    }

    setIsLoading(true);
    void refresh();

    const abortController = new AbortController();
    const callbacks: StreamCallbacks = {
      onTextDelta: noop,
      onThinkingDelta: noop,
      onToolCallStart: noop,
      onToolCallComplete: queueRefresh,
      onToolResult: queueRefresh,
      onToolOutputDelta: noop,
      onPlanUpdate: queueRefresh,
      onModeChange: queueRefresh,
      onPlanComplete: queueRefresh,
      onUsage: noop,
      onTitleUpdate: noop,
      onFinish: queueRefresh,
      onError: queueRefresh,
      onToolApprovalRequired: queueRefresh,
      onToolApproved: queueRefresh,
      onToolDenied: queueRefresh,
      onTurnComplete: queueRefresh,
      onUserMessage: queueRefresh,
      onAgentSleeping: queueRefresh,
      onTickInjected: queueRefresh,
      onClassifierDecision: queueRefresh,
      onTeammateSpawned: queueRefresh,
      onTeammateTaskCompleted: queueRefresh,
      onTeammateTaskFailed: queueRefresh,
      onTeammateCancelled: queueRefresh,
    };

    void client.observeMakoSession(sessionId, callbacks, abortController.signal).catch(() => {});

    return () => {
      abortController.abort();
      if (refreshTimerRef.current) {
        clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
    };
  }, [client, enabled, isConnected, queueRefresh, refresh, sessionId]);

  const wake = useMemo<MakoRunWakeEvent[]>(() => buildWakeEvents(status), [status]);

  return {
    status,
    wake,
    isLoading,
    error,
    refresh,
  };
}
