import { useShallow } from "zustand/react/shallow";

import { useSessionStore } from "../../../hooks/useStores";

export function useHiveSessionView() {
  return useSessionStore(
    useShallow((state) => ({
      sessionId: state.sessionId,
      title: state.title,
      messages: state.messages,
      isStreaming: state.isStreaming,
      isThinking: state.isThinking,
      isLoading: state.isLoading,
      tokenCount: state.tokenCount,
      error: state.error,
    })),
    "hive",
  );
}
