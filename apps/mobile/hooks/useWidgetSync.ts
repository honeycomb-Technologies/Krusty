import { useEffect, useRef } from "react";
import { Platform } from "react-native";
import {
  shouldSyncChatWidget,
  type ChatWidgetCadenceState,
} from "./presentationCadence";

// Native-only — widget instances loaded dynamically to avoid crash on web
let MakoWidgetInstance: any = null;
let ChatWidgetInstance: any = null;

if (Platform.OS === "ios") {
  try {
    MakoWidgetInstance = require("../widgets/MakoWidget").default;
    ChatWidgetInstance = require("../widgets/ChatWidget").default;
  } catch {
    // Widgets not available
  }
}

interface ChatState {
  sessionId: string | null;
  hasActiveSession: boolean;
  sessionTitle: string;
  lastMessage: string;
  model: string;
  isStreaming: boolean;
  tokenCount: number;
  serverConnected: boolean;
}

export function useWidgetSync(chatState: ChatState) {
  const previousStateRef = useRef<ChatWidgetCadenceState | null>(null);

  useEffect(() => {
    if (Platform.OS !== "ios") return;
    const nextState: ChatWidgetCadenceState = chatState;
    if (!shouldSyncChatWidget(previousStateRef.current, nextState)) return;

    previousStateRef.current = nextState;
    const { sessionId: _sessionId, ...snapshot } = nextState;

    try {
      ChatWidgetInstance.updateSnapshot(snapshot);
    } catch {
      // Widget may not be configured yet
    }
  }, [
    chatState.sessionId,
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
