import { useCallback } from "react";
import { useShallow } from "zustand/react/shallow";

import { useSessionStore, useStores } from "../../../hooks/useStores";
import { isExactQueuedRecoveryActionTarget } from "../queuedRecoveryActionFence";

export function useHiveSessionView() {
  const stores = useStores();
  const sessionView = useSessionStore(
    useShallow((state) => ({
      sessionId: state.sessionId,
      title: state.title,
      messages: state.messages,
      isStreaming: state.isStreaming,
      isThinking: state.isThinking,
      isLoading: state.isLoading,
      tokenCount: state.tokenCount,
      error: state.error,
      queuedRecoveryBlocked: state.queuedRecoveryBlocked,
    })),
    "hive",
  );

  const retryQueuedRecovery = useCallback(async (
    expectedSessionId: string,
  ) => {
    const store = stores?.modes.hive.session;
    if (!store) return;
    const state = store.getState();
    if (
      !isExactQueuedRecoveryActionTarget(
        expectedSessionId,
        state.sessionId,
        state.queuedRecoveryBlocked,
      )
    ) return;
    try {
      await state.retryQueuedRecovery();
    } catch {
      // Durable state remains blocked and visible; never let a rejected button
      // callback escape as an unhandled promise.
    }
  }, [stores]);

  const discardQueuedRecovery = useCallback(async (
    expectedSessionId: string,
  ) => {
    const store = stores?.modes.hive.session;
    if (!store) return;
    const state = store.getState();
    if (
      !isExactQueuedRecoveryActionTarget(
        expectedSessionId,
        state.sessionId,
        state.queuedRecoveryBlocked,
      )
    ) return;
    try {
      await state.discardQueuedRecovery(expectedSessionId);
    } catch {
      // Keep the recovery record and blocked UI intact when deletion fails.
    }
  }, [stores]);

  return {
    ...sessionView,
    retryQueuedRecovery,
    discardQueuedRecovery,
  };
}
