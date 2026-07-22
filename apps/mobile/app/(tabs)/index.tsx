import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import {
  AppState,
  View,
  Text,
  Pressable,
  Alert,
  useWindowDimensions,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router, useLocalSearchParams } from "expo-router";
import { Bot, Menu, Toolbox } from "lucide-react-native";
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
import { buildToolDiffPresentation } from "../../components/chat/toolDiffModel";
import Animated from "react-native-reanimated";

import type {
  ModelInfo,
  ModelKey,
  SessionResponse,
} from "@krusty/api";
import type { MakoTopLevelView } from "../../components/mako/types";
import type {
  Attachment as SessionAttachment,
  PermissionMode,
  ThinkingLevel,
} from "@krusty/state";
import {
  modelKeysEqual,
  resolveUsableModel,
  supportsFastMode,
} from "@krusty/state";

import { ChatBootScreen } from "./chat-screen/BootScreen";
import {
  CHAT_BAR_ZONE,
  SELECTED_MODEL_KEY,
  flattenToolCalls,
  getActiveToolCall,
  getLastAssistantMessage,
  normalizeProviderId,
  sessionTypeForTab,
  tabForSessionType,
} from "./chat-screen/helpers";
import {
  DESKTOP_CHAT_MAX_WIDTH,
  TOOLBOX_DOCK_WIDTH,
  resolveDesktopChatMaxWidth,
  styles,
} from "./chat-screen/styles";
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
  const { splashDone } = useSplashState();

  if (!stores) {
    return (
      <ChatBootScreen
        status={status}
        isConfigured={isConfigured}
        connectionError={connectionError}
        showLogo={!splashDone}
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
  const routeParams = useLocalSearchParams<{
    sessionId?: string | string[];
    focus?: string | string[];
  }>();
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { width: windowWidth } = useWindowDimensions();
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
  const modelKey = useSessionStore((state) => state.modelKey) ?? null;
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
  const [defaultModelKey, setDefaultModelKey] = useState<ModelKey | null>(null);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState(1);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [makoTopLevel, setMakoTopLevel] = useState<MakoTopLevelView>("mako");
  const [toolboxOpen, setToolboxOpen] = useState(false);
  const [toolboxTab, setToolboxTab] = useState(0);
  const [bottomControlsOpen, setBottomControlsOpen] = useState(false);
  const [composerReserveHeight, setComposerReserveHeight] =
    useState(CHAT_BAR_ZONE);
  const [errorBannerHeight, setErrorBannerHeight] = useState(0);
  /** Measured desktop chat pane width (split host, before soft-cap). */
  const [desktopPaneWidth, setDesktopPaneWidth] = useState(0);
  const selectedModelInfo = useMemo(
    () =>
      (modelKey
        ? models.find((candidate) => modelKeysEqual(candidate.key, modelKey))
        : models.find((candidate) => candidate.id === model)) ?? null,
    [model, modelKey, models],
  );
  const fastModeSupported = supportsFastMode(
    selectedModelInfo ?? model,
    selectedModelInfo?.provider ?? null,
  );
  const fastModeEnabled = fastModeSupported && fastModeStoreEnabled;

  useEffect(() => {
    if (!model || !selectedModelInfo) {
      return;
    }
    sessionStore.getState().setModel(model, selectedModelInfo.provider, selectedModelInfo);
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
  const activityDiff = useMemo(
    () =>
      toolCalls.reduce(
        (total, toolCall) => {
          const diff = buildToolDiffPresentation(toolCall);
          if (diff) {
            total.additions += diff.additions;
            total.deletions += diff.deletions;
          }
          return total;
        },
        { additions: 0, deletions: 0 },
      ),
    [toolCalls],
  );

  const lastAssistantSnippet =
    lastAssistantMessage?.content?.slice(0, 200) ?? "";
  const showTranscriptError = Boolean(error && messages.length > 0);

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

      if (focus === "mako") {
        setActiveTab(2);
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

  useEffect(() => {
    const targetSessionId = Array.isArray(routeParams.sessionId)
      ? routeParams.sessionId[0]
      : routeParams.sessionId;
    const focus = Array.isArray(routeParams.focus)
      ? routeParams.focus[0]
      : routeParams.focus;
    if ((targetSessionId && targetSessionId !== sessionId) || focus === "mako") {
      void handleNotificationNavigate("/(tabs)", {
        ...(targetSessionId ? { sessionId: targetSessionId } : {}),
        ...(focus ? { focus } : {}),
      });
    }
  }, [handleNotificationNavigate, routeParams.focus, routeParams.sessionId, sessionId]);

  const handleRegisterNativeDevice = useCallback(
    async (deviceToken: string) => {
      if (!client || !isConnected || !deviceToken) {
        return false;
      }

      try {
        await client.registerApnsDevice(deviceToken);
        return true;
      } catch (error) {
        console.debug("Failed to register APNs device", error);
        return false;
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
    setDefaultModelKey(response.default_model_key ?? null);
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
    let fallbackDefaultKey = defaultModelKey;
    let allowedProviders = configuredProviders;

    if (catalog.length === 0) {
      const result = await loadModelCatalog().catch(() => null);
      if (!result) {
        return null;
      }
      catalog = result.response.models;
      fallbackDefault = result.response.default_model ?? null;
      fallbackDefaultKey = result.response.default_model_key ?? null;
      allowedProviders = result.configuredProviders;
    }

    const selectedModel = resolveUsableModel(
      existingModel,
      fallbackDefault,
      catalog,
      allowedProviders,
      sessionStore.getState().modelKey,
      fallbackDefaultKey,
    );

    if (selectedModel) {
      sessionStore
        .getState()
        .setModel(selectedModel.id, selectedModel.provider ?? null, selectedModel);
      await SecureStore.setItemAsync(SELECTED_MODEL_KEY, selectedModel.id);
      return selectedModel.id;
    }

    sessionStore.getState().setModel(null);
    await SecureStore.deleteItemAsync(SELECTED_MODEL_KEY).catch(() => {});
    return null;
  }, [
    configuredProviders,
    defaultModelId,
    defaultModelKey,
    loadModelCatalog,
    models,
    sessionStore,
  ]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
    void ensureModelReady();
  }, [client, ensureModelReady, isConnected, sessionsStore]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (AppState.currentState === "active") {
        void loadModelCatalog().catch(() => null);
      }
    }, 5 * 60 * 1000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, loadModelCatalog]);

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
      void loadModelCatalog().catch(() => null);
    });

    return () => subscription.remove();
  }, [loadModelCatalog, sessionStore, workspace]);

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
      startActivity(sessionId!, sessionTitle || "Chat");
      liveActivityOpenRef.current = true;
    }

    if (shouldKeepActivity) {
      updateActivity({
        chatTitle: sessionTitle || "Chat",
        status: awaitingApproval ? "needs_input" : "working",
        toolCount: toolCalls.length,
        filesAdded: activityDiff.additions,
        filesRemoved: activityDiff.deletions,
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
    activityDiff,
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
    toolCalls.length,
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

  const handleToolboxClose = useCallback(() => {
    setToolboxOpen(false);
  }, []);

  // Prefer measured host; fall back to window so the band never full-bleeds.
  const effectivePaneWidth = useMemo(() => {
    if (desktopPaneWidth > 0) return desktopPaneWidth;
    if (!isDesktop) return windowWidth;
    const toolboxSlice = toolboxOpen ? TOOLBOX_DOCK_WIDTH : 0;
    return Math.max(0, windowWidth - toolboxSlice);
  }, [desktopPaneWidth, isDesktop, toolboxOpen, windowWidth]);

  const desktopChatMaxWidth = resolveDesktopChatMaxWidth(effectivePaneWidth);

  // Title slot: real title → first user message snippet → "New chat".
  // Store strips "New Session" placeholders, so empty string is common mid-turn.
  const displayTitle = useMemo(() => {
    const real = sessionTitle?.trim();
    if (real) return { text: real, isPlaceholder: false };

    const firstUser = messages.find(
      (message) =>
        message.role === "user" &&
        typeof message.content === "string" &&
        message.content.trim().length > 0,
    );
    if (firstUser && typeof firstUser.content === "string") {
      const snippet = firstUser.content.trim().replace(/\s+/g, " ");
      const text =
        snippet.length > 56 ? `${snippet.slice(0, 56).trimEnd()}…` : snippet;
      return { text, isPlaceholder: true };
    }

    if (sessionId) {
      return { text: "New chat", isPlaceholder: true };
    }

    return { text: "", isPlaceholder: true };
  }, [messages, sessionId, sessionTitle]);

  const topBar = (
    <Animated.View
      style={[
        styles.topBar,
        isDesktop && styles.topBarDesktop,
        entrance.topBarStyle,
      ]}
    >
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
        disabled={!sessionId}
      >
        <Text
          style={[
            styles.title,
            displayTitle.isPlaceholder && styles.titlePlaceholder,
            {
              color: displayTitle.text
                ? displayTitle.isPlaceholder
                  ? t.mutedForeground
                  : t.foreground
                : "transparent",
            },
          ]}
          numberOfLines={1}
        >
          {displayTitle.text || " "}
        </Text>
      </Pressable>

      <View style={styles.topBarActions}>
        <Pressable
          onPress={() => handleSelectMakoView("mako")}
          style={isDesktop ? styles.toolboxCornerBtn : styles.menuBtn}
          accessibilityRole="button"
          accessibilityLabel="Open Mako"
        >
          <Bot size={20} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setToolboxOpen((open) => !open);
          }}
          style={[
            isDesktop ? styles.toolboxCornerBtn : styles.menuBtn,
            isDesktop && toolboxOpen
              ? { backgroundColor: `${t.thinking}22` }
              : null,
          ]}
          accessibilityRole="button"
          accessibilityLabel={toolboxOpen ? "Close toolbox" : "Open toolbox"}
        >
          <Toolbox
            size={20}
            color={toolboxOpen ? t.thinking : t.mutedForeground}
            strokeWidth={1.8}
          />
        </Pressable>
      </View>
    </Animated.View>
  );

  const transcriptAndComposer = (
    <View style={styles.flex}>
      <Animated.View style={[styles.flex, entrance.contentStyle]}>
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
          bottomPadding={
            composerReserveHeight
            + (showTranscriptError ? errorBannerHeight + 10 : 0)
          }
          hideJumpToLatest={bottomControlsOpen}
        />

        {showTranscriptError ? (
          <View
            accessibilityRole="alert"
            accessibilityLiveRegion="polite"
            onLayout={(event) => {
              const nextHeight = Math.ceil(event.nativeEvent.layout.height);
              setErrorBannerHeight((current) =>
                current === nextHeight ? current : nextHeight,
              );
            }}
            style={[
              styles.errorBanner,
              {
                position: "absolute",
                left: 0,
                right: 0,
                bottom: composerReserveHeight + 10,
                marginBottom: 0,
                zIndex: 30,
                borderColor: `${t.error}40`,
                backgroundColor: `${t.error}14`,
              },
            ]}
          >
            <Text
              selectable
              style={[styles.errorBannerText, { color: t.error }]}
            >
              {error}
            </Text>
          </View>
        ) : null}
      </Animated.View>

      <Animated.View
        style={[
          entrance.bottomBarStyle,
          { overflow: "visible", zIndex: 300 },
        ]}
      >
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
            sessionStore
              .getState()
              .setMode(mode === "build" ? "plan" : "build")
          }
          onModelSelect={handleModelSelect}
          model={model}
          models={models}
          sessionType={sessionTypeForTab(activeTab)}
          tokenCount={tokenCount}
          onOverlayOpenChange={setBottomControlsOpen}
          contentMaxWidth={isDesktop ? desktopChatMaxWidth : undefined}
        />
      </Animated.View>
    </View>
  );

  const chatMain = (
    <View style={styles.flex}>
      {isDesktop ? (
        <View
          style={styles.desktopChatColumnHost}
          onLayout={(event) => {
            const next = Math.round(event.nativeEvent.layout.width);
            setDesktopPaneWidth((prev) => (prev === next ? prev : next));
          }}
        >
          {/* Full-pane chrome: title + toolbox button in the true top-right. */}
          {topBar}
          {/* Messages + composer share a centered soft-capped band. */}
          <View
            style={[
              styles.desktopChatColumn,
              {
                maxWidth: desktopChatMaxWidth || DESKTOP_CHAT_MAX_WIDTH,
                width: "100%",
              },
            ]}
          >
            {transcriptAndComposer}
          </View>
        </View>
      ) : (
        <>
          {topBar}
          {transcriptAndComposer}
        </>
      )}
    </View>
  );

  const chatContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {isDesktop ? (
        // Desktop: chat fills remaining width; toolbox is a fixed-width rail.
        <View style={styles.desktopSplit}>
          <View style={styles.desktopSplitChat}>{chatMain}</View>
          {toolboxOpen ? (
            <ToolboxPanel
              variant="dock"
              visible={toolboxOpen}
              onClose={handleToolboxClose}
              activeTab={toolboxTab}
              onTabChange={setToolboxTab}
            />
          ) : null}
        </View>
      ) : (
        <>
          {chatMain}
          <ToolboxPanel
            variant="overlay"
            visible={toolboxOpen}
            onClose={handleToolboxClose}
            activeTab={toolboxTab}
            onTabChange={setToolboxTab}
          />
        </>
      )}
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
        }}
      />
    </Animated.View>
  );

  return (
    <DesktopShell
      sessions={sessions}
      activeSessionId={sessionId}
      onSelectSession={(session) => void loadSession(session)}
      onNewSession={() => void handleNewSession("chat")}
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
          onNewSession={() => void handleNewSession("chat")}
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
