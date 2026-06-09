import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import {
  AppState,
  View,
  Text,
  Pressable,
  Alert,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";
import { Menu, Toolbox } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import * as SecureStore from "../../platform/secure-store";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import {
  useSessionStore,
  useSessionsStore,
  useStores,
  useWorkspaceStore,
} from "../../hooks/useStores";
import { ChatTranscript } from "../../components/chat/ChatTranscript";
import { KrustyLogo } from "../../components/ui/KrustyLogo";
import {
  ChatBar,
  type Attachment as ChatBarAttachment,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ToolboxPanel } from "../../components/ToolboxPanel";
import { MakoScreen } from "../../components/mako/MakoScreen";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useLiveActivity } from "../../hooks/useLiveActivity";
import { useWidgetSync } from "../../hooks/useWidgetSync";
import { useNotifications } from "../../hooks/useNotifications";
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  interpolate,
  withSpring,
  withTiming,
  runOnJS,
} from "react-native-reanimated";

import type {
  ModelInfo,
  SessionResponse,
} from "@krusty/api";
import type { MakoTopLevelView } from "../../components/mako/types";
import type {
  Attachment as SessionAttachment,
  PermissionMode,
  ThinkingLevel,
} from "@krusty/state";
import { supportsFastMode } from "@krusty/state";

import { ChatBootScreen } from "./chat-screen/BootScreen";
import {
  CHAT_BAR_ZONE,
  SELECTED_MODEL_KEY,
  SPLIT_PANEL_HEIGHT,
  flattenToolCalls,
  getActiveToolCall,
  getLastAssistantMessage,
  isModelUsable,
  normalizeProviderId,
  sessionTypeForTab,
  tabForSessionType,
} from "./chat-screen/helpers";
import { styles } from "./chat-screen/styles";
import { useSessionActions } from "./chat-screen/useSessionActions";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;

export default function ChatScreen() {
  const {
    status,
    error: connectionError,
    reconnect,
    isConfigured,
  } = useConnection();
  const stores = useStores();

  if (!stores) {
    return (
      <ChatBootScreen
        status={status}
        isConfigured={isConfigured}
        connectionError={connectionError}
        onRetryConnection={() => {
          void reconnect();
        }}
        onOpenSetup={() => {
          router.replace("/onboarding");
        }}
      />
    );
  }

  return <ChatScreenContent stores={stores} />;
}

function ChatScreenContent({ stores }: { stores: LoadedStores }) {
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { splashDone } = useSplashState();
  const entrance = useEntranceAnimation(splashDone);

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
    useSessionStore((state) => state.permissionMode) ?? "autonomous";
  const fastModeStoreEnabled =
    useSessionStore((state) => state.fastModeEnabled) ?? false;
  const mode = useSessionStore((state) => state.mode) ?? "build";
  const tokenCount = useSessionStore((state) => state.tokenCount) ?? 0;
  const error = useSessionStore((state) => state.error) ?? null;
  const isLoading = useSessionStore((state) => state.isLoading) ?? false;
  const workspaceDirectory =
    useWorkspaceStore((state) => state.directory) ?? null;
  const workspaceSessionId =
    useWorkspaceStore((state) => state.sessionId) ?? null;

  const [models, setModels] = useState<ModelInfo[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState(1);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [makoTopLevel, setMakoTopLevel] = useState<MakoTopLevelView>("mako");
  const [toolboxOpen, setToolboxOpen] = useState(false);
  const [toolboxTab, setToolboxTab] = useState(2);
  const splitProgress = useSharedValue(0);
  const [isSplit, setIsSplit] = useState(false);
  const [researchEnabled, setResearchEnabled] = useState(false);
  const [composerReserveHeight, setComposerReserveHeight] =
    useState(CHAT_BAR_ZONE);
  const selectedModelInfo = useMemo(
    () => models.find((candidate) => candidate.id === model) ?? null,
    [model, models],
  );
  const fastModeSupported = supportsFastMode(
    model,
    selectedModelInfo?.provider ?? null,
  );
  const fastModeEnabled = fastModeSupported && fastModeStoreEnabled;

  useEffect(() => {
    if (!model || !selectedModelInfo) {
      return;
    }
    sessionStore.getState().setModel(model, selectedModelInfo.provider);
  }, [model, selectedModelInfo, sessionStore]);

  const previousStreamingRef = useRef(false);
  const currentStreamSessionIdRef = useRef<string | null>(null);
  const streamStartedAtRef = useRef<number | null>(null);
  const liveActivityOpenRef = useRef(false);
  const notifiedApprovalIdsRef = useRef<Set<string>>(new Set());
  const suppressCompletionRef = useRef(false);
  const attemptedWorkspaceSessionHydrationRef = useRef<string | null>(null);

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

  const handleNotificationNavigate = useCallback(
    async (_route: string, params?: Record<string, string>) => {
      const focus = params?.focus;
      const targetSessionId = params?.sessionId;
      const shouldOpenReports = params?.openReports === "true";

      if (focus === "mako") {
        setActiveTab(2);
      }
      if (shouldOpenReports) {
        setToolboxOpen(true);
      }
      if (!targetSessionId) {
        return;
      }

      try {
        await sessionStore.getState().loadSession(targetSessionId, true);
      } catch {
        void sessionsStore.getState().loadSessions();
      }
    },
    [sessionStore, sessionsStore],
  );

  const handleRegisterNativeDevice = useCallback(
    async (deviceToken: string) => {
      if (!client || !isConnected || !deviceToken) {
        return;
      }

      try {
        await client.registerApnsDevice(deviceToken);
      } catch (error) {
        console.debug("Failed to register APNs device", error);
      }
    },
    [client, isConnected],
  );

  const { startActivity, updateActivity, endActivity } = useLiveActivity({
    onToolApproval: handleToolApprovalAction,
  });
  const { notifyToolApproval, notifyStreamComplete } = useNotifications({
    onToolApproval: handleToolApprovalAction,
    onNavigate: handleNotificationNavigate,
    onRegisterNativeDevice: handleRegisterNativeDevice,
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

  const loadModelCatalog = useCallback(async () => {
    if (!client || !isConnected) {
      return null;
    }

    const [response, credentials] = await Promise.all([
      client.getModels(),
      client.getCredentials().catch(() => []),
    ]);
    const nextConfiguredProviders = credentials
      .filter((provider) => provider.configured || provider.has_oauth)
      .map((provider) => normalizeProviderId(provider.name));
    setModels(response.models);
    setDefaultModelId(response.default_model ?? null);
    setConfiguredProviders(nextConfiguredProviders);
    return {
      response,
      configuredProviders: nextConfiguredProviders,
    };
  }, [client, isConnected]);

  const ensureModelReady = useCallback(async () => {
    const existingModel = sessionStore.getState().model;
    let catalog = models;
    let fallbackDefault = defaultModelId;
    let allowedProviders = configuredProviders;

    if (catalog.length === 0) {
      const result = await loadModelCatalog().catch(() => null);
      if (!result) {
        return null;
      }
      catalog = result.response.models;
      fallbackDefault = result.response.default_model ?? null;
      allowedProviders = result.configuredProviders;
    }

    const existingModelInfo = catalog.find(
      (candidate) => candidate.id === existingModel,
    );
    if (isModelUsable(existingModel, catalog, allowedProviders)) {
      sessionStore.getState().setModel(existingModel, existingModelInfo?.provider ?? null);
      return existingModel;
    }

    const selectedModel = isModelUsable(fallbackDefault, catalog, allowedProviders)
      ? fallbackDefault
      : null;

    if (selectedModel) {
      const selectedModelInfo = catalog.find(
        (candidate) => candidate.id === selectedModel,
      );
      sessionStore.getState().setModel(selectedModel, selectedModelInfo?.provider ?? null);
      await SecureStore.setItemAsync(SELECTED_MODEL_KEY, selectedModel);
      return selectedModel;
    }

    sessionStore.getState().setModel(null);
    await SecureStore.deleteItemAsync(SELECTED_MODEL_KEY).catch(() => {});
    return null;
  }, [configuredProviders, defaultModelId, loadModelCatalog, models, sessionStore]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
    void ensureModelReady();
  }, [client, ensureModelReady, isConnected, sessionsStore]);

  useEffect(() => {
    if (!client || !isConnected || sessionId || !workspaceSessionId) {
      return;
    }
    if (attemptedWorkspaceSessionHydrationRef.current === workspaceSessionId) {
      return;
    }

    attemptedWorkspaceSessionHydrationRef.current = workspaceSessionId;
    void sessionStore
      .getState()
      .loadSession(workspaceSessionId, true)
      .catch(() => {
        void sessionsStore.getState().loadSessions();
      });
  }, [client, isConnected, sessionId, sessionStore, sessionsStore, workspaceSessionId]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (AppState.currentState === "active") {
        void sessionsStore.getState().loadSessions();
      }
    }, 5000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, sessionsStore]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState !== "active") {
        return;
      }

      const currentSessionId =
        sessionStore.getState().sessionId ?? workspace.getState().sessionId;
      if (currentSessionId) {
        void sessionStore.getState().loadSession(currentSessionId, true);
      }
    });

    return () => subscription.remove();
  }, [sessionStore, workspace]);

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

  const {
    stopCurrentStream,
    loadSession,
    loadSessionById,
    openProjectInCode,
    handleNewSession,
    handleDirectorySelected,
    handleDeleteSession,
    handleInteractiveToolResult,
    handlePlanConfirm,
    handleSend,
    handleModelSelect,
    handleFastModeToggle,
    handleTabChange,
  } = useSessionActions({
    client,
    activeTab,
    activeToolCallId,
    setActiveToolCallId,
    setActiveTab,
    setDrawerOpen,
    ensureModelReady,
    researchEnabled,
    sessionStore,
    sessionsStore,
    workspace,
    sessions,
    models,
    suppressCompletionRef,
  });

  const handleSessionToolApproval = useCallback(
    (targetSessionId: string, toolCallId: string, approved: boolean) => {
      void handleToolApprovalAction(targetSessionId, toolCallId, approved);
    },
    [handleToolApprovalAction],
  );

  const handleStop = useCallback(() => {
    stopCurrentStream();
  }, [stopCurrentStream]);

  const handleChatBarSend = useCallback(
    async (content: string, attachments: ChatBarAttachment[] = []) => {
      const normalizedAttachments = attachments.map((attachment) => ({
        ...attachment,
        mimeType: attachment.mimeType ?? "application/octet-stream",
      })) as SessionAttachment[];
      await handleSend(content, normalizedAttachments);
    },
    [handleSend],
  );

  const handleSelectMakoView = useCallback(
    (view: MakoTopLevelView) => {
      handleTabChange(2);
      setMakoTopLevel(view);
      setDrawerOpen(false);
    },
    [handleTabChange],
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

  const handleToolboxPin = useCallback(() => {
    setIsSplit(true);
    splitProgress.value = withSpring(1, { damping: 22, stiffness: 280, mass: 0.8 });
  }, [splitProgress]);

  const handleToolboxUnpin = useCallback(() => {
    splitProgress.value = withTiming(0, { duration: 300 });
    setIsSplit(false);
  }, [splitProgress]);

  const closeToolbox = useCallback(() => {
    setToolboxOpen(false);
    setIsSplit(false);
  }, []);

  const handleToolboxClose = useCallback(() => {
    if (splitProgress.value > 0.1) {
      splitProgress.value = withTiming(0, { duration: 250 }, (finished) => {
        if (finished) runOnJS(closeToolbox)();
      });
    } else {
      setToolboxOpen(false);
    }
  }, [splitProgress, closeToolbox]);

  const chatOffsetStyle = useAnimatedStyle(() => ({
    marginTop: interpolate(splitProgress.value, [0, 1], [0, SPLIT_PANEL_HEIGHT]),
  }));

  const topBar = (
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
          setToolboxOpen(true);
        }}
        style={styles.menuBtn}
      >
        <Toolbox size={20} color={toolboxOpen ? t.userMessage : t.mutedForeground} strokeWidth={1.8} />
      </Pressable>
    </Animated.View>
  );

  const chatContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {topBar}

      <View style={styles.flex}>
        <Animated.View style={[styles.flex, entrance.contentStyle, chatOffsetStyle]}>
          <ChatTranscript
            messages={messages}
            sessionId={sessionId}
            isStreaming={isStreaming}
            isThinking={isThinking}
            activeToolCallId={activeToolCallId}
            onApproveTool={(targetSessionId, toolCallId) =>
              handleSessionToolApproval(targetSessionId, toolCallId, true)
            }
            onDenyTool={(targetSessionId, toolCallId) =>
              handleSessionToolApproval(targetSessionId, toolCallId, false)
            }
            onSubmitToolResult={(toolCallId, result) =>
              void handleInteractiveToolResult(toolCallId, result)
            }
            onPlanConfirm={(toolCallId, choice) =>
              void handlePlanConfirm(toolCallId, choice)
            }
            emptyState={
              <View style={styles.empty}>
                <KrustyLogo />
                {error ? (
                  <Text style={[styles.emptyHint, { color: t.error }]}>
                    {error}
                  </Text>
                ) : null}
              </View>
            }
            bottomPadding={composerReserveHeight}
          />
        </Animated.View>

        {(!toolboxOpen || isSplit) && (
          <Animated.View style={[entrance.bottomBarStyle, { overflow: "visible", zIndex: 300 }]}>
            <ChatBar
              onSend={handleChatBarSend}
              onStop={handleStop}
              onHeightChange={setComposerReserveHeight}
              isStreaming={isStreaming}
              disabled={!isConnected}
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
        )}

        <ToolboxPanel
          visible={toolboxOpen}
          onClose={handleToolboxClose}
          onTogglePin={isSplit ? handleToolboxUnpin : handleToolboxPin}
          isSplit={isSplit}
          splitProgress={splitProgress}
          activeTab={toolboxTab}
          onTabChange={setToolboxTab}
        />
      </View>
    </SafeAreaView>
  );

  const makoContent = (
    <Animated.View style={[styles.flex, entrance.contentStyle]}>
      <MakoScreen
        workspaceDirectory={workspaceDirectory}
        activeRunId={sessionId}
        requestedTopLevel={makoTopLevel}
        onOpenRunById={loadSessionById}
        onOpenProject={openProjectInCode}
        onDeleteRun={handleDeleteSession}
        onOpenMenu={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          setDrawerOpen(true);
        }}
        onTopLevelChange={setMakoTopLevel}
        chat={{
          sessionId,
          title: sessionTitle,
          messages,
          error,
          isLoading,
          isStreaming,
          isThinking,
          activeToolCallId,
          thinkingLevel: thinkingLevel as ThinkingLevel,
          permissionMode: permissionMode as PermissionMode,
          fastModeEnabled,
          fastModeSupported,
          mode,
          model,
          models,
          researchEnabled,
          tokenCount,
          onApproveTool: (targetSessionId, toolCallId) =>
            handleSessionToolApproval(targetSessionId, toolCallId, true),
          onDenyTool: (targetSessionId, toolCallId) =>
            handleSessionToolApproval(targetSessionId, toolCallId, false),
          onSubmitToolResult: (toolCallId, result) =>
            void handleInteractiveToolResult(toolCallId, result),
          onPlanConfirm: (toolCallId, choice) =>
            void handlePlanConfirm(toolCallId, choice),
          onSend: handleChatBarSend,
          onStop: handleStop,
          onThinkingChange: (level) =>
            sessionStore.getState().setThinkingLevel(level),
          onPermissionModeToggle: () =>
            sessionStore.getState().togglePermissionMode(),
          onFastModeToggle: handleFastModeToggle,
          onModeToggle: () =>
            sessionStore.getState().setMode(mode === "build" ? "plan" : "build"),
          onModelSelect: handleModelSelect,
          onResearchToggle: () => setResearchEnabled((current) => !current),
        }}
      />
    </Animated.View>
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
      activeMakoView={makoTopLevel}
      onSelectMakoView={handleSelectMakoView}
    >
      {activeTab === 2 ? makoContent : chatContent}

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
          activeMakoView={makoTopLevel}
          onSelectMakoView={handleSelectMakoView}
        />
      )}
    </DesktopShell>
  );
}
