import { useEffect } from "react";
import { Platform } from "react-native";

import MakoWidgetInstance from "../widgets/MakoWidget";
import ChatWidgetInstance from "../widgets/ChatWidget";

interface ChatState {
  hasActiveSession: boolean;
  sessionTitle: string;
  lastMessage: string;
  model: string;
  isStreaming: boolean;
  tokenCount: number;
  serverConnected: boolean;
}

export function useWidgetSync(chatState: ChatState) {
  useEffect(() => {
    if (Platform.OS !== "ios") return;

    try {
      ChatWidgetInstance.updateSnapshot(chatState);
    } catch {
      // Widget may not be configured yet
    }
  }, [
    chatState.hasActiveSession,
    chatState.sessionTitle,
    chatState.lastMessage,
    chatState.model,
    chatState.isStreaming,
    chatState.tokenCount,
    chatState.serverConnected,
  ]);
}

interface MakoState {
  status: "active" | "idle" | "running" | "offline";
  lastUpdate: string;
  briefing: string;
  taskName?: string;
  taskProgress?: number;
  completedTasks: number;
  totalTasks: number;
  serverConnected: boolean;
}

export function useMakoWidgetSync(state: MakoState) {
  useEffect(() => {
    if (Platform.OS !== "ios") return;

    try {
      MakoWidgetInstance.updateSnapshot(state);
    } catch {
      // Widget may not be configured yet
    }
  }, [
    state.status,
    state.lastUpdate,
    state.briefing,
    state.taskName,
    state.taskProgress,
    state.completedTasks,
    state.totalTasks,
    state.serverConnected,
  ]);
}
