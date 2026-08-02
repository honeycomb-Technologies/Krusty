import { useEffect, useRef } from "react";
import { Platform } from "react-native";
import {
  shouldSyncChatWidget,
  type ChatWidgetCadenceState,
} from "./presentationCadence";

// Native-only — widget instances loaded dynamically to avoid crash on web.
// Update both kinds while installed widgets and native/OTA generations overlap.
const HiveWidgetInstances: any[] = [];
let ChatWidgetInstance: any = null;

if (Platform.OS === "ios") {
  try {
    HiveWidgetInstances.push(require("../widgets/HiveWidget").default);
  } catch {
    // The installed native build may predate the canonical widget kind.
  }
  try {
    HiveWidgetInstances.push(require("../widgets/MakoWidget").default);
  } catch {
    // Compatibility kind may be absent after its eventual retirement.
  }
  try {
    ChatWidgetInstance = require("../widgets/ChatWidget").default;
  } catch {
    // Widget support is unavailable in this native build.
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

interface HiveState {
  status: "active" | "idle" | "running" | "offline";
  lastUpdate: string;
  briefing: string;
  taskName?: string;
  taskProgress?: number;
  completedTasks: number;
  totalTasks: number;
  serverConnected: boolean;
}

export function useHiveWidgetSync(state: HiveState) {
  useEffect(() => {
    if (Platform.OS !== "ios") return;

    for (const widget of HiveWidgetInstances) {
      try {
        widget.updateSnapshot(state);
      } catch {
        // This widget kind may not exist in the installed native generation.
      }
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
