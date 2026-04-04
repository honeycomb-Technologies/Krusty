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
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";
import { Menu, FileSearch } from "lucide-react-native";
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
  ThinkingLevel,
  ToolCall,
} from "@krusty/state";

const TAB_TYPES: SessionType[] = ["chat", "code", "mako"];
const CHAT_BAR_ZONE = 130;
const SELECTED_MODEL_KEY = "krusty_selected_model";

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

export default function ChatScreen() {
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { splashDone } = useSplashState();
  const entrance = useEntranceAnimation(splashDone);
  const {
    sessions: sessionsStore,
    session: sessionStore,
    workspace,
  } = useStores();

  const sessions = useSessionsStore(
    (state) => state.sessions,
  ) as SessionResponse[];
  const sessionId = useSessionStore((state) => state.sessionId);
  const sessionTitle = useSessionStore((state) => state.title);
  const messages = useSessionStore((state) => state.messages);
  const isStreaming = useSessionStore((state) => state.isStreaming);
  const isThinking = useSessionStore((state) => state.isThinking);
  const model = useSessionStore((state) => state.model);
  const thinkingLevel = useSessionStore((state) => state.thinkingLevel);
  const mode = useSessionStore((state) => state.mode);
  const tokenCount = useSessionStore((state) => state.tokenCount);
  const error = useSessionStore((state) => state.error);

  const [models, setModels] = useState<ModelInfo[]>([]);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState(1);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reportsOpen, setReportsOpen] = useState(false);
  const [researchEnabled, setResearchEnabled] = useState(false);

  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const previousStreamingRef = useRef(false);
  const currentStreamSessionIdRef = useRef<string | null>(null);
  const streamStartedAtRef = useRef<number | null>(null);
  const liveActivityOpenRef = useRef(false);
  const notifiedApprovalIdsRef = useRef<Set<string>>(new Set());
  const suppressCompletionRef = useRef(false);

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
    async (toolCallId: string, approved: boolean) => {
      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId || activeToolCallId) {
        return;
      }

      setActiveToolCallId(toolCallId);
      try {
        await sessionStore.getState().submitToolApproval(toolCallId, approved);
      } catch {
        await sessionStore.getState().loadSession(currentSessionId, true);
      } finally {
        setActiveToolCallId(null);
      }
    },
    [activeToolCallId, sessionStore],
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
  const msgLen = messages.length;
  const lastMsgContent = messages[msgLen - 1]?.content?.length ?? 0;

  const scrollToBottom = useCallback(() => {
    const content = contentHeightRef.current;
    const viewport = listHeightRef.current;
    if (!content || !viewport || content <= viewport) {
      return;
    }

    const maxOffset = content - viewport;
    const targetOffset = Math.max(0, maxOffset - CHAT_BAR_ZONE);
    flatListRef.current?.scrollToOffset({
      offset: targetOffset,
      animated: !isStreaming,
    });
  }, [isStreaming]);

  useEffect(() => {
    if (msgLen > 0) {
      requestAnimationFrame(scrollToBottom);
    }
  }, [lastMsgContent, msgLen, scrollToBottom]);

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

      await sessionStore
        .getState()
        .sendMessage(trimmed, attachments as SessionAttachment[]);
    },
    [client, ensureSessionForSend, sessionStore],
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
              onScrollBeginDrag={Keyboard.dismiss}
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
                    void handleToolApprovalAction(toolCallId, true)
                  }
                  onDenyTool={(toolCallId) =>
                    void handleToolApprovalAction(toolCallId, false)
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
              ]}
              onLayout={(event) => {
                listHeightRef.current = event.nativeEvent.layout.height;
              }}
              onContentSizeChange={(_width, height) => {
                contentHeightRef.current = height;
              }}
              keyboardDismissMode="interactive"
              keyboardShouldPersistTaps="handled"
            />
            <LinearGradient
              colors={[t.background, t.background + "00"]}
              style={styles.fadeTop}
              pointerEvents="none"
            />
            <LinearGradient
              colors={[t.background + "00", t.background]}
              style={styles.fadeBottom}
              pointerEvents="none"
            />
          </View>
        )}
      </Animated.View>

      <Animated.View style={[entrance.bottomBarStyle, { overflow: "visible" }]}>
        <ChatBar
          onSend={handleSend}
          onStop={handleStop}
          isStreaming={isStreaming}
          disabled={!isConnected}
          thinkingLevel={thinkingLevel as ThinkingLevel}
          onThinkingChange={(level) =>
            sessionStore.getState().setThinkingLevel(level)
          }
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
    paddingTop: 8,
    paddingBottom: 16,
  },
  listDesktop: {
    maxWidth: 800,
    alignSelf: "center",
    width: "100%",
  },
  fadeTop: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 64,
  },
  fadeBottom: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    height: 120,
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
