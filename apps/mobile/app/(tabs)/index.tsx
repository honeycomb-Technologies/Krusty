import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import {
  AppState,
  View,
  FlatList,
  StyleSheet,
  Text,
  Pressable,
  Alert,
  Keyboard,
  NativeScrollEvent,
  NativeSyntheticEvent,
} from "react-native";
import {
  SafeAreaView,
} from "react-native-safe-area-context";
import { router } from "expo-router";
import { Menu, FileSearch, ArrowDown } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import * as SecureStore from "../../platform/secure-store";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import {
  useSessionStore,
  useSessionsStore,
  useStores,
} from "../../hooks/useStores";
import { MessageBubble } from "../../components/chat/MessageBubble";
import { KrustyLogo } from "../../components/ui/KrustyLogo";
import {
  ChatBar,
  type Attachment as ChatBarAttachment,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ReportsViewer } from "../../components/ReportsViewer";
import { PlanTracker } from "../../components/chat/PlanTracker";
import { BlurView } from "../../platform/blur";
import { LinearGradient } from "../../platform/linear-gradient";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useLiveActivity } from "../../hooks/useLiveActivity";
import { useWidgetSync } from "../../hooks/useWidgetSync";
import { useNotifications } from "../../hooks/useNotifications";
import Animated from "react-native-reanimated";
import type { ModelInfo, SessionResponse, SessionType } from "@krusty/api";
import type {
  Attachment as SessionAttachment,
  ChatMessage,
  PermissionMode,
  ThinkingLevel,
  ToolCall,
} from "@krusty/state";
import {
  isFastModeModel,
  supportsFastMode,
  toggleFastModeModel,
} from "@krusty/state";

const TAB_TYPES: SessionType[] = ["chat", "code", "mako"];
const SELECTED_MODEL_KEY = "krusty_selected_model";
const TOP_EDGE_HEIGHT = 64;
const BOTTOM_EDGE_HEIGHT = 88;
const EDGE_GAP = 12;
const TRACKER_GAP = 10;
const SCROLL_FOLLOW_THRESHOLD = 72;

type WorkspaceMode = "neutral" | "selected" | "created";

function sessionTypeForTab(index: number): SessionType {
  return TAB_TYPES[index] ?? "code";
}

function tabForSessionType(type: SessionType): number {
  switch (type) {
    case "chat":
      return 0;
    case "mako":
      return 2;
    default:
      return 1;
  }
}

function getLastAssistantMessage(messages: ChatMessage[]): ChatMessage | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.role === "assistant") {
      return message;
    }
  }
  return null;
}

function flattenToolCalls(messages: ChatMessage[]): ToolCall[] {
  const toolCalls: ToolCall[] = [];
  for (const message of messages) {
    if (message.toolCalls?.length) {
      toolCalls.push(...message.toolCalls);
    }
  }
  return toolCalls;
}

function getActiveToolCall(toolCalls: ToolCall[]): ToolCall | null {
  for (let index = toolCalls.length - 1; index >= 0; index -= 1) {
    const toolCall = toolCalls[index];
    if (
      toolCall &&
      (toolCall.status === "awaiting_approval" ||
        toolCall.status === "running" ||
        toolCall.status === "pending")
    ) {
      return toolCall;
    }
  }
  return null;
}

function getWorkspaceMode(path: string | null): WorkspaceMode {
  return path ? "selected" : "neutral";
}

function distanceFromBottom(
  contentHeight: number,
  viewportHeight: number,
  offsetY: number,
) {
  return Math.max(0, contentHeight - (offsetY + viewportHeight));
}

export default function ChatScreen() {
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { splashDone } = useSplashState();
  const entrance = useEntranceAnimation(splashDone);
  const stores = useStores();

  // Stores not ready yet (before connection)
  if (!stores) {
    return (
      <SafeAreaView style={[{ flex: 1, backgroundColor: theme.colors.background }]}>
        <View style={{ flex: 1, justifyContent: 'center', alignItems: 'center' }}>
          <KrustyLogo />
        </View>
      </SafeAreaView>
    );
  }

  const { sessions: sessionsStore, session: sessionStore, workspace } = stores;

  const sessions = useSessionsStore(
    (state) => state.sessions,
  ) as SessionResponse[];
  const sessionId = useSessionStore((state) => state.sessionId) ?? null;
  const sessionTitle = useSessionStore((state) => state.title) ?? null;
  const messages = useSessionStore((state) => state.messages) ?? [];
  const isStreaming = useSessionStore((state) => state.isStreaming) ?? false;
  const isThinking = useSessionStore((state) => state.isThinking) ?? false;
  const model = useSessionStore((state) => state.model) ?? null;
  const thinkingLevel =
    useSessionStore((state) => state.thinkingLevel) ?? "medium";
  const permissionMode =
    useSessionStore((state) => state.permissionMode) ?? "supervised";
  const mode = useSessionStore((state) => state.mode) ?? "build";
  const tokenCount = useSessionStore((state) => state.tokenCount) ?? 0;
  const error = useSessionStore((state) => state.error) ?? null;
  const fastModeSupported = supportsFastMode(model);
  const fastModeEnabled = isFastModeModel(model);

  const [models, setModels] = useState<ModelInfo[]>([]);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState(1);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reportsOpen, setReportsOpen] = useState(false);
  const [researchEnabled, setResearchEnabled] = useState(false);
  const [chatBarHeight, setChatBarHeight] = useState(0);
  const [planTrackerHeight, setPlanTrackerHeight] = useState(0);
  const [isNearBottom, setIsNearBottom] = useState(true);

  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const scrollOffsetRef = useRef(0);
  const previousStreamingRef = useRef(false);
  const currentStreamSessionIdRef = useRef<string | null>(null);
  const streamStartedAtRef = useRef<number | null>(null);
  const liveActivityOpenRef = useRef(false);
  const notifiedApprovalIdsRef = useRef<Set<string>>(new Set());
  const suppressCompletionRef = useRef(false);
  const autoFollowRef = useRef(true);
  const pendingAutoScrollRef = useRef(false);
  const pendingAutoScrollAnimatedRef = useRef(false);
  const isUserDraggingRef = useRef(false);
  const loadedSessionIdRef = useRef<string | null>(null);

  const lastAssistantMessage = useMemo(
    () => getLastAssistantMessage(messages),
    [messages],
  );
  const toolCalls = useMemo(() => flattenToolCalls(messages), [messages]);
  const awaitingApprovalCalls = useMemo(
    () =>
      toolCalls.filter((toolCall) => toolCall.status === "awaiting_approval"),
    [toolCalls],
  );
  const activeToolCall = useMemo(
    () => getActiveToolCall(toolCalls),
    [toolCalls],
  );

  const lastAssistantSnippet =
    lastAssistantMessage?.content?.slice(0, 200) ?? "";

  const handleToolApprovalAction = useCallback(
    async (targetSessionId: string, toolCallId: string, approved: boolean) => {
      if (!targetSessionId || activeToolCallId) {
        return;
      }

      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId && !client) {
        return;
      }

      setActiveToolCallId(toolCallId);
      try {
        if (currentSessionId === targetSessionId) {
          await sessionStore.getState().submitToolApproval(toolCallId, approved);
        } else if (client) {
          await client.submitToolApproval(targetSessionId, toolCallId, approved);
        }
      } catch {
        if (currentSessionId === targetSessionId) {
          await sessionStore.getState().loadSession(targetSessionId, true);
        }
      } finally {
        setActiveToolCallId(null);
      }
    },
    [activeToolCallId, client, sessionStore],
  );

  const { startActivity, updateActivity, endActivity } = useLiveActivity({
    onToolApproval: handleToolApprovalAction,
  });
  const { notifyToolApproval, notifyStreamComplete } = useNotifications({
    onToolApproval: handleToolApprovalAction,
  });

  useWidgetSync({
    hasActiveSession: Boolean(sessionId),
    sessionTitle: sessionTitle || "Untitled",
    lastMessage: lastAssistantSnippet,
    model: model || "",
    isStreaming,
    tokenCount,
    serverConnected: isConnected,
  });

  const t = theme.colors;
  const isDark = theme.scheme === "dark";
  const edgeTint = isDark
    ? ("systemChromeMaterialDark" as const)
    : ("systemChromeMaterialLight" as const);
  const jumpTint = isDark
    ? ("systemMaterialDark" as const)
    : ("systemMaterialLight" as const);
  const edgeOverlay = isDark
    ? "rgba(11,17,25,0.18)"
    : "rgba(255,255,255,0.18)";
  const jumpOverlay = isDark
    ? "rgba(11,17,25,0.78)"
    : "rgba(255,255,255,0.78)";
  const msgLen = messages.length;
  const lastMsgContent = messages[msgLen - 1]?.content?.length ?? 0;
  const effectivePlanTrackerHeight = isDesktop ? 0 : planTrackerHeight;
  const listTopPadding =
    TOP_EDGE_HEIGHT +
    EDGE_GAP +
    (effectivePlanTrackerHeight > 0
      ? effectivePlanTrackerHeight + TRACKER_GAP
      : 0);
  const listBottomPadding = chatBarHeight + BOTTOM_EDGE_HEIGHT + EDGE_GAP;
  const showJumpToLatest = messages.length > 0 && isStreaming && !isNearBottom;

  const scrollToBottom = useCallback((animated: boolean) => {
    if (!flatListRef.current) {
      return;
    }

    scrollOffsetRef.current = Math.max(
      0,
      contentHeightRef.current - listHeightRef.current,
    );
    setIsNearBottom(true);
    flatListRef.current.scrollToEnd({ animated });
  }, []);

  const queueAutoScroll = useCallback((animated: boolean) => {
    pendingAutoScrollRef.current = true;
    pendingAutoScrollAnimatedRef.current = animated;
  }, []);

  const flushAutoScroll = useCallback(() => {
    if (
      !pendingAutoScrollRef.current ||
      !flatListRef.current ||
      isUserDraggingRef.current
    ) {
      return;
    }

    if (!autoFollowRef.current) {
      pendingAutoScrollRef.current = false;
      return;
    }

    pendingAutoScrollRef.current = false;
    const animated = pendingAutoScrollAnimatedRef.current;
    requestAnimationFrame(() => {
      scrollToBottom(animated);
    });
  }, [scrollToBottom]);

  const updateNearBottom = useCallback((offsetY = scrollOffsetRef.current) => {
    const nextNearBottom =
      distanceFromBottom(
        contentHeightRef.current,
        listHeightRef.current,
        offsetY,
      ) <= SCROLL_FOLLOW_THRESHOLD;

    setIsNearBottom((current) =>
      current === nextNearBottom ? current : nextNearBottom,
    );

    if (nextNearBottom) {
      autoFollowRef.current = true;
    } else if (isUserDraggingRef.current) {
      autoFollowRef.current = false;
    }

    return nextNearBottom;
  }, []);

  useEffect(() => {
    if (!sessionId) {
      loadedSessionIdRef.current = null;
      autoFollowRef.current = true;
      pendingAutoScrollRef.current = false;
      scrollOffsetRef.current = 0;
      setIsNearBottom(true);
      return;
    }

    if (loadedSessionIdRef.current === sessionId) {
      return;
    }

    loadedSessionIdRef.current = sessionId;
    autoFollowRef.current = true;
    scrollOffsetRef.current = 0;
    setIsNearBottom(true);
    queueAutoScroll(false);
  }, [queueAutoScroll, sessionId]);

  useEffect(() => {
    if (msgLen === 0) {
      return;
    }

    if (autoFollowRef.current) {
      queueAutoScroll(!isStreaming);
    }
  }, [isStreaming, lastMsgContent, msgLen, queueAutoScroll]);

  useEffect(() => {
    if (msgLen === 0 || !autoFollowRef.current) {
      return;
    }

    queueAutoScroll(false);
  }, [chatBarHeight, msgLen, queueAutoScroll]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
    void client
      .getModels()
      .then(async (response) => {
        setModels(response.models);
        if (!sessionStore.getState().model) {
          const saved = await SecureStore.getItemAsync(SELECTED_MODEL_KEY);
          const selectedModel =
            saved && response.models.some((candidate) => candidate.id === saved)
              ? saved
              : response.default_model;

          if (selectedModel) {
            sessionStore.getState().setModel(selectedModel);
          }
        }
      })
      .catch(() => {});
  }, [client, isConnected, model, sessionStore, sessionsStore]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState !== "active") {
        return;
      }

      const currentSessionId = sessionStore.getState().sessionId;
      if (currentSessionId) {
        void sessionStore.getState().loadSession(currentSessionId, true);
      }
    });

    return () => subscription.remove();
  }, [sessionStore]);

  useEffect(() => {
    if (!sessionId) {
      return;
    }

    const activeSession = sessions.find((session) => session.id === sessionId);
    if (!activeSession) {
      return;
    }

    const nextTab = tabForSessionType(activeSession.session_type);
    setActiveTab((currentTab) =>
      currentTab === nextTab ? currentTab : nextTab,
    );
  }, [sessionId, sessions]);

  useEffect(() => {
    const nextNotifiedIds = new Set<string>();
    if (!sessionId) {
      notifiedApprovalIdsRef.current = nextNotifiedIds;
      return;
    }

    for (const toolCall of awaitingApprovalCalls) {
      nextNotifiedIds.add(toolCall.id);
      if (notifiedApprovalIdsRef.current.has(toolCall.id)) {
        continue;
      }

      void notifyToolApproval(toolCall.id, toolCall.name, sessionId);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
    }

    notifiedApprovalIdsRef.current = nextNotifiedIds;
  }, [awaitingApprovalCalls, notifyToolApproval, sessionId]);

  useEffect(() => {
    const awaitingApproval =
      awaitingApprovalCalls[awaitingApprovalCalls.length - 1] ?? null;
    const shouldKeepActivity =
      Boolean(sessionId) && (isStreaming || Boolean(awaitingApproval));

    if (isStreaming && !previousStreamingRef.current) {
      suppressCompletionRef.current = false;
      currentStreamSessionIdRef.current = sessionId;
      streamStartedAtRef.current = Date.now();
    }

    if (shouldKeepActivity && !liveActivityOpenRef.current) {
      startActivity(sessionTitle || "Chat", model || "unknown");
      liveActivityOpenRef.current = true;
    }

    if (shouldKeepActivity) {
      updateActivity({
        chatTitle: sessionTitle || "Chat",
        model: model || "unknown",
        status: awaitingApproval
          ? "awaiting_approval"
          : isThinking
            ? "thinking"
            : activeToolCall
              ? "tool_call"
              : "streaming",
        currentText: isThinking
          ? "Thinking..."
          : (lastAssistantMessage?.content?.slice(-200) ?? ""),
        currentTool: awaitingApproval?.name || activeToolCall?.name || "",
        tokenCount,
        progress: awaitingApproval ? 0.85 : isStreaming ? 0.5 : 1,
        toolApprovalId: awaitingApproval?.id,
        toolApprovalName: awaitingApproval?.name,
        toolApprovalSessionId: awaitingApproval ? sessionId ?? undefined : undefined,
      });
    } else if (liveActivityOpenRef.current) {
      endActivity();
      liveActivityOpenRef.current = false;
    }

    if (
      previousStreamingRef.current &&
      !isStreaming &&
      !awaitingApproval &&
      !suppressCompletionRef.current &&
      currentStreamSessionIdRef.current &&
      currentStreamSessionIdRef.current === sessionId
    ) {
      const startedAt = streamStartedAtRef.current ?? Date.now();
      const elapsedSeconds = Math.max(
        0,
        Math.floor((Date.now() - startedAt) / 1000),
      );
      void notifyStreamComplete(
        currentStreamSessionIdRef.current,
        sessionTitle || "Chat",
        tokenCount,
        elapsedSeconds,
      );
      currentStreamSessionIdRef.current = null;
      streamStartedAtRef.current = null;
    }

    if (!shouldKeepActivity && !isStreaming) {
      suppressCompletionRef.current = false;
      currentStreamSessionIdRef.current = null;
      streamStartedAtRef.current = null;
    }

    previousStreamingRef.current = isStreaming;
  }, [
    activeToolCall,
    awaitingApprovalCalls,
    endActivity,
    isStreaming,
    isThinking,
    lastAssistantMessage,
    model,
    notifyStreamComplete,
    sessionId,
    sessionTitle,
    startActivity,
    tokenCount,
    updateActivity,
  ]);

  const stopCurrentStream = useCallback(
    (suppressCompletion = true) => {
      if (sessionStore.getState().isStreaming) {
        suppressCompletionRef.current = suppressCompletion;
        sessionStore.getState().stopStreaming();
      }
      setActiveToolCallId(null);
    },
    [sessionStore],
  );

  const bootstrapSession = useCallback(
    async (session: SessionResponse) => {
      const currentModel = sessionStore.getState().model;
      const currentThinkingLevel = sessionStore.getState().thinkingLevel;
      const directory = session.project_dir ?? session.working_dir ?? null;
      const workspaceMode = (session.workspace_mode ??
        getWorkspaceMode(directory)) as WorkspaceMode;

      sessionStore.getState().initSession(session.id, session.title || "");
      workspace
        .getState()
        .initFromSession(session.id, directory, workspaceMode);

      if (currentModel) {
        sessionStore.getState().setModel(currentModel);
      }
      if (sessionStore.getState().thinkingLevel !== currentThinkingLevel) {
        sessionStore.getState().setThinkingLevel(currentThinkingLevel);
      }

      await sessionsStore.getState().loadSessions();
    },
    [sessionStore, sessionsStore, workspace],
  );

  const createSessionForCurrentTab = useCallback(
    async (directory?: string) => {
      if (!client) {
        return null;
      }

      stopCurrentStream();

      try {
        const session = await client.createSession(
          undefined,
          directory,
          undefined,
          directory ? "selected" : undefined,
          sessionTypeForTab(activeTab),
        );
        await bootstrapSession(session);
        setActiveToolCallId(null);
        setDrawerOpen(false);
        void Haptics.notificationAsync(
          Haptics.NotificationFeedbackType.Success,
        );
        return session;
      } catch {
        return null;
      }
    },
    [activeTab, bootstrapSession, client, stopCurrentStream],
  );

  const ensureSessionForSend = useCallback(async () => {
    const currentSessionId = sessionStore.getState().sessionId;
    if (currentSessionId) {
      return currentSessionId;
    }

    if (!client) {
      return null;
    }

    try {
      const session = await client.createSession(
        undefined,
        undefined,
        undefined,
        undefined,
        sessionTypeForTab(activeTab),
      );
      await bootstrapSession(session);
      return session.id;
    } catch {
      return null;
    }
  }, [activeTab, bootstrapSession, client, sessionStore]);

  const loadSession = useCallback(
    async (session: SessionResponse) => {
      stopCurrentStream();
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(session.session_type));
      await sessionStore.getState().loadSession(session.id);
    },
    [sessionStore, stopCurrentStream],
  );

  const handleNewSession = useCallback(async () => {
    await createSessionForCurrentTab();
  }, [createSessionForCurrentTab]);

  const handleDirectorySelected = useCallback(
    async (path: string) => {
      await createSessionForCurrentTab(path);
    },
    [createSessionForCurrentTab],
  );

  const handleDeleteSession = useCallback(
    (id: string) => {
      if (!client) {
        return;
      }

      Alert.alert("Delete Session", "Delete this session?", [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: async () => {
            try {
              if (sessionStore.getState().sessionId === id) {
                stopCurrentStream();
              }
              await client.deleteSession(id);
              await sessionsStore.getState().loadSessions();
              if (sessionStore.getState().sessionId === id) {
                sessionStore.getState().clearSession();
                setActiveToolCallId(null);
              }
            } catch {
              // silent
            }
          },
        },
      ]);
    },
    [client, sessionsStore, stopCurrentStream, sessionStore],
  );

  const handleInteractiveToolResult = useCallback(
    async (toolCallId: string, result: string) => {
      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId || activeToolCallId) {
        return;
      }

      setActiveToolCallId(toolCallId);
      try {
        await sessionStore.getState().submitToolResult(toolCallId, result);
      } catch {
        await sessionStore.getState().loadSession(currentSessionId, true);
      } finally {
        setActiveToolCallId(null);
      }
    },
    [activeToolCallId, sessionStore],
  );

  const handlePlanConfirm = useCallback(
    async (toolCallId: string, choice: "execute" | "abandon") => {
      if (choice === "execute") {
        sessionStore.getState().setMode("build");
      }
      await handleInteractiveToolResult(toolCallId, JSON.stringify({ choice }));
    },
    [handleInteractiveToolResult, sessionStore],
  );

  const handleSend = useCallback(
    async (content: string, attachments: ChatBarAttachment[] = []) => {
      const trimmed = content.trim();
      if (!client || (!trimmed && attachments.length === 0)) {
        return;
      }

      const ensuredSessionId = await ensureSessionForSend();
      if (!ensuredSessionId) {
        return;
      }

      autoFollowRef.current = true;
      setIsNearBottom(true);
      queueAutoScroll(false);
      await sessionStore
        .getState()
        .sendMessage(trimmed, attachments as SessionAttachment[]);
    },
    [client, ensureSessionForSend, queueAutoScroll, sessionStore],
  );

  const handleStop = useCallback(() => {
    stopCurrentStream();
  }, [stopCurrentStream]);

  const handleModelSelect = useCallback(
    (modelId: string) => {
      sessionStore.getState().setModel(modelId);
      void SecureStore.setItemAsync(SELECTED_MODEL_KEY, modelId);
    },
    [sessionStore],
  );

  const handleFastModeToggle = useCallback(() => {
    const currentModel = sessionStore.getState().model;
    const nextModel = toggleFastModeModel(currentModel);
    if (!nextModel || nextModel === currentModel) {
      return;
    }

    sessionStore.getState().setModel(nextModel);
    void SecureStore.setItemAsync(SELECTED_MODEL_KEY, nextModel);
  }, [sessionStore]);

  const handleTabChange = useCallback(
    (index: number) => {
      setActiveTab(index);

      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId) {
        return;
      }

      const currentSession = sessions.find(
        (session) => session.id === currentSessionId,
      );
      if (
        !currentSession ||
        currentSession.session_type !== sessionTypeForTab(index)
      ) {
        stopCurrentStream();
        sessionStore.getState().clearSession();
      }
    },
    [sessionStore, sessions, stopCurrentStream],
  );

  const handleRenameSession = useCallback(() => {
    if (!sessionId || !sessionTitle) {
      return;
    }

    Alert.prompt(
      "Rename Session",
      undefined,
      async (newTitle?: string) => {
        const nextTitle = newTitle?.trim();
        if (!nextTitle) {
          return;
        }

        sessionStore.getState().setTitle(nextTitle);
        await sessionStore.getState().updateTitle(sessionId, nextTitle);
      },
      "plain-text",
      sessionTitle,
    );
  }, [sessionId, sessionStore, sessionTitle]);

  const handleListScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      scrollOffsetRef.current = event.nativeEvent.contentOffset.y;
      updateNearBottom(event.nativeEvent.contentOffset.y);
    },
    [updateNearBottom],
  );

  const handleJumpToLatest = useCallback(() => {
    autoFollowRef.current = true;
    setIsNearBottom(true);
    queueAutoScroll(false);
    scrollToBottom(false);
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
  }, [queueAutoScroll, scrollToBottom]);

  const chatContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      <Animated.View style={[styles.topBar, entrance.topBarStyle]}>
        {!isDesktop && (
          <Pressable
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              setDrawerOpen(true);
            }}
            style={styles.menuBtn}
          >
            <Menu size={22} color={t.foreground} strokeWidth={1.8} />
          </Pressable>
        )}

        <Pressable
          onPress={handleRenameSession}
          style={styles.titleBtn}
          disabled={!sessionTitle}
        >
          <Text
            style={[
              styles.title,
              { color: sessionTitle ? t.foreground : "transparent" },
            ]}
            numberOfLines={1}
          >
            {sessionTitle || " "}
          </Text>
        </Pressable>

        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setReportsOpen(true);
          }}
          style={styles.menuBtn}
        >
          <FileSearch size={20} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
      </Animated.View>

      <Animated.View style={[styles.flex, entrance.contentStyle]}>
        {messages.length === 0 ? (
          <Pressable style={styles.empty} onPress={Keyboard.dismiss}>
            <KrustyLogo />
            {error ? (
              <Text style={[styles.emptyHint, { color: t.error }]}>
                {error}
              </Text>
            ) : null}
          </Pressable>
        ) : (
          <View style={styles.flex}>
            <FlatList
              ref={flatListRef}
              data={messages}
              keyExtractor={(_, index) => String(index)}
              onScrollBeginDrag={() => {
                isUserDraggingRef.current = true;
                Keyboard.dismiss();
              }}
              onScrollEndDrag={() => {
                isUserDraggingRef.current = false;
                updateNearBottom();
                flushAutoScroll();
              }}
              onMomentumScrollEnd={() => {
                isUserDraggingRef.current = false;
                updateNearBottom();
                flushAutoScroll();
              }}
              onScroll={handleListScroll}
              scrollEventThrottle={16}
              renderItem={({
                item,
                index,
              }: {
                item: ChatMessage;
                index: number;
              }) => (
                <MessageBubble
                  message={item}
                  isLast={index === messages.length - 1}
                  isStreaming={isStreaming && index === messages.length - 1}
                  isThinking={isThinking && index === messages.length - 1}
                  activeToolCallId={activeToolCallId}
                  onApproveTool={(toolCallId) =>
                    sessionId
                      ? void handleToolApprovalAction(sessionId, toolCallId, true)
                      : undefined
                  }
                  onDenyTool={(toolCallId) =>
                    sessionId
                      ? void handleToolApprovalAction(sessionId, toolCallId, false)
                      : undefined
                  }
                  onSubmitToolResult={(toolCallId, result) =>
                    void handleInteractiveToolResult(toolCallId, result)
                  }
                  onPlanConfirm={(toolCallId, choice) =>
                    void handlePlanConfirm(toolCallId, choice)
                  }
                />
              )}
              style={styles.flex}
              contentContainerStyle={[
                styles.list,
                isDesktop && styles.listDesktop,
                {
                  paddingTop: listTopPadding,
                  paddingBottom: listBottomPadding,
                },
              ]}
              onLayout={(event) => {
                listHeightRef.current = event.nativeEvent.layout.height;
                updateNearBottom();
                flushAutoScroll();
              }}
              onContentSizeChange={(_width, height) => {
                contentHeightRef.current = height;
                updateNearBottom();
                flushAutoScroll();
              }}
              keyboardDismissMode="interactive"
              keyboardShouldPersistTaps="handled"
            />
            <View pointerEvents="none" style={[styles.edgeChrome, styles.edgeTop]}>
              <BlurView intensity={40} tint={edgeTint} style={StyleSheet.absoluteFill} />
              <View style={[StyleSheet.absoluteFill, { backgroundColor: edgeOverlay }]} />
              <LinearGradient
                colors={[`${t.background}F0`, `${t.background}C8`, `${t.background}00`]}
                style={StyleSheet.absoluteFill}
              />
            </View>
            {!isDesktop && <PlanTracker onHeightChange={setPlanTrackerHeight} />}
            <View
              pointerEvents="none"
              style={[
                styles.edgeChrome,
                styles.edgeBottom,
                { bottom: chatBarHeight },
              ]}
            >
              <BlurView intensity={40} tint={edgeTint} style={StyleSheet.absoluteFill} />
              <View style={[StyleSheet.absoluteFill, { backgroundColor: edgeOverlay }]} />
              <LinearGradient
                colors={[`${t.background}00`, `${t.background}C8`, `${t.background}F0`]}
                style={StyleSheet.absoluteFill}
              />
            </View>
            {showJumpToLatest && (
              <Pressable
                onPress={handleJumpToLatest}
                style={[styles.jumpToLatest, { bottom: chatBarHeight + EDGE_GAP }]}
              >
                <BlurView
                  intensity={28}
                  tint={jumpTint}
                  style={StyleSheet.absoluteFill}
                />
                <View
                  style={[
                    StyleSheet.absoluteFill,
                    { backgroundColor: jumpOverlay },
                  ]}
                />
                <ArrowDown size={15} color={t.foreground} strokeWidth={2} />
                <Text style={[styles.jumpToLatestText, { color: t.foreground }]}>
                  Jump to latest
                </Text>
              </Pressable>
            )}
          </View>
        )}
      </Animated.View>

      <Animated.View style={[entrance.bottomBarStyle, { overflow: "visible" }]}>
        <ChatBar
          onSend={handleSend}
          onStop={handleStop}
          isStreaming={isStreaming}
          disabled={!isConnected}
          onHeightChange={setChatBarHeight}
          thinkingLevel={thinkingLevel as ThinkingLevel}
          onThinkingChange={(level) =>
            sessionStore.getState().setThinkingLevel(level)
          }
          permissionMode={permissionMode as PermissionMode}
          onPermissionModeToggle={() =>
            sessionStore.getState().togglePermissionMode()
          }
          fastModeEnabled={fastModeEnabled}
          fastModeSupported={fastModeSupported}
          onFastModeToggle={handleFastModeToggle}
          mode={mode}
          onModeToggle={() =>
            sessionStore.getState().setMode(mode === "build" ? "plan" : "build")
          }
          onModelSelect={handleModelSelect}
          model={model}
          models={models}
          sessionType={sessionTypeForTab(activeTab)}
          researchEnabled={researchEnabled}
          onResearchToggle={() => setResearchEnabled((current) => !current)}
          tokenCount={tokenCount}
        />
      </Animated.View>

      <ReportsViewer
        visible={reportsOpen}
        onClose={() => setReportsOpen(false)}
      />
    </SafeAreaView>
  );

  return (
    <DesktopShell
      sessions={sessions}
      activeSessionId={sessionId}
      onSelectSession={(session) => void loadSession(session)}
      onNewSession={() => void handleNewSession()}
      onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
      onDeleteSession={handleDeleteSession}
      onOpenSettings={() => router.push("/(tabs)/settings")}
      activeTab={activeTab}
      onTabChange={handleTabChange}
    >
      {chatContent}

      {!isDesktop && (
        <SessionDrawer
          isOpen={drawerOpen}
          onClose={() => setDrawerOpen(false)}
          sessions={sessions}
          activeSessionId={sessionId}
          onSelectSession={(session) => void loadSession(session)}
          onNewSession={() => void handleNewSession()}
          onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
          onDeleteSession={handleDeleteSession}
          onOpenSettings={() => {
            setDrawerOpen(false);
            router.push("/(tabs)/settings");
          }}
          activeTab={activeTab}
          onTabChange={handleTabChange}
        />
      )}
    </DesktopShell>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  topBar: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 12,
  },
  menuBtn: {
    padding: 4,
  },
  titleBtn: {
    flex: 1,
  },
  title: {
    fontSize: 17,
    fontWeight: "600",
    textAlign: "center",
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  list: {
    paddingHorizontal: 16,
  },
  listDesktop: {
    maxWidth: 800,
    alignSelf: "center",
    width: "100%",
  },
  edgeChrome: {
    position: "absolute",
    left: 0,
    right: 0,
  },
  edgeTop: {
    top: 0,
    height: TOP_EDGE_HEIGHT,
  },
  edgeBottom: {
    height: BOTTOM_EDGE_HEIGHT,
  },
  jumpToLatest: {
    position: "absolute",
    left: 16,
    alignSelf: "flex-start",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 999,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.12)",
    zIndex: 60,
  },
  jumpToLatestText: {
    fontSize: 13,
    fontWeight: "600",
  },
  empty: {
    flex: 1,
    justifyContent: "flex-start",
    alignItems: "center",
    paddingTop: "35%",
    gap: 16,
  },
  emptyTitle: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  emptyHint: {
    fontSize: 17,
  },
  stubTitle: {
    fontSize: 24,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
  stubText: {
    fontSize: 15,
    marginTop: 8,
  },
  modalBackdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: "rgba(0,0,0,0.6)",
    justifyContent: "flex-end",
    zIndex: 200,
  },
  modelPicker: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: "60%",
    paddingTop: 20,
    paddingBottom: 40,
    backgroundColor: "#1a1f2e",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: "rgba(255,255,255,0.1)",
  },
  modelPickerTitle: {
    fontSize: 18,
    fontWeight: "700",
    textAlign: "center",
    marginBottom: 16,
  },
  modelList: {
    paddingHorizontal: 16,
  },
  modelItem: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    marginBottom: 8,
  },
  modelName: {
    fontSize: 16,
    fontWeight: "500",
  },
  modelProvider: {
    fontSize: 13,
    marginTop: 2,
  },
});
